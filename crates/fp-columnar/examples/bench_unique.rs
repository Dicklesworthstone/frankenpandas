//! Bench + golden for Column::unique — dense direct-address for bounded Int64.
//!
//! Run: cargo run -p fp-columnar --example bench_unique --release
//!
//! unique used a std SipHash HashSet; an all-valid bounded-range Int64 column
//! dedups via a seen-bitset indexed by (v - min), hash-free, first-seen order
//! preserved. Non-bounded / non-Int64 / nullable columns keep the HashSet path.

use std::time::Instant;

use fp_columnar::Column;
use fp_types::{DType, IntervalClosed, NullKind, Scalar};
use rustc_hash::FxHashSet;

fn median_nanos(samples: &mut [u128]) -> u128 {
    samples.sort_unstable();
    match samples.get(samples.len() / 2) {
        Some(&value) => value,
        None => 0,
    }
}

fn measure_nanos<T>(f: impl FnOnce() -> T) -> (u128, T) {
    let start = Instant::now();
    let value = std::hint::black_box(f());
    (start.elapsed().as_nanos(), value)
}

fn build_unique_typed_i64(n: usize) -> Column {
    let values = (0..n)
        .map(|i| i64::try_from(i).expect("benchmark length fits i64") * 1_000_003)
        .collect();
    Column::from_i64_values_owned(values)
}

fn run_has_duplicates_attribution() {
    const N: usize = 250_000;
    const SAMPLES: usize = 11;
    let mut materialize = Vec::with_capacity(SAMPLES);
    let mut warm_scan = Vec::with_capacity(SAMPLES);
    let mut cold_public = Vec::with_capacity(SAMPLES);

    for _ in 0..SAMPLES {
        let column = build_unique_typed_i64(N);
        let (elapsed, len) = measure_nanos(|| std::hint::black_box(column.values()).len());
        assert_eq!(len, N);
        materialize.push(elapsed);

        let column = build_unique_typed_i64(N);
        std::hint::black_box(column.values());
        let (elapsed, has_duplicates) = measure_nanos(|| column.has_duplicates());
        assert!(!has_duplicates);
        warm_scan.push(elapsed);

        let column = build_unique_typed_i64(N);
        let (elapsed, has_duplicates) = measure_nanos(|| column.has_duplicates());
        assert!(!has_duplicates);
        cold_public.push(elapsed);
    }

    let materialize_p50 = median_nanos(&mut materialize);
    let warm_scan_p50 = median_nanos(&mut warm_scan);
    let cold_public_p50 = median_nanos(&mut cold_public);
    println!("typed Int64 has_duplicates attribution: n={N} samples={SAMPLES}");
    println!("Scalar materialization p50: {materialize_p50} ns");
    println!("warm generic duplicate scan p50: {warm_scan_p50} ns");
    println!("cold public has_duplicates p50: {cold_public_p50} ns");
}

#[allow(dead_code)]
#[derive(Hash, PartialEq, Eq)]
enum FormerDuplicateKey<'a> {
    Bool(bool),
    Int64(i64),
    FloatBits(u64),
    Utf8(&'a str),
    Timedelta64(i64),
    Datetime64(i64),
    Period(i64),
    Interval(u64, u64, IntervalClosed),
}

fn former_has_duplicates(column: &Column) -> bool {
    let mut seen = FxHashSet::<FormerDuplicateKey<'_>>::default();
    for value in column.values() {
        if let Scalar::Int64(value) = value {
            if !seen.insert(FormerDuplicateKey::Int64(*value)) {
                return true;
            }
        } else {
            return column.has_duplicates();
        }
    }
    false
}

fn measure_former_has_duplicates(column: &Column) -> u128 {
    let (elapsed, has_duplicates) = measure_nanos(|| former_has_duplicates(column));
    assert!(!has_duplicates);
    elapsed
}

fn measure_candidate_has_duplicates(column: &Column) -> u128 {
    let (elapsed, has_duplicates) = measure_nanos(|| column.has_duplicates());
    assert!(!has_duplicates);
    elapsed
}

fn run_has_duplicates_ab() {
    const N: usize = 250_000;
    const SAMPLES: usize = 15;

    let former = build_unique_typed_i64(N);
    let candidate = build_unique_typed_i64(N);
    assert_eq!(former_has_duplicates(&former), candidate.has_duplicates());
    let duplicate = Column::from_i64_values_owned(vec![1, 2, 3, 1]);
    assert!(former_has_duplicates(&duplicate));
    assert!(duplicate.has_duplicates());

    for _ in 0..3 {
        let former = build_unique_typed_i64(N);
        let candidate = build_unique_typed_i64(N);
        std::hint::black_box(measure_former_has_duplicates(&former));
        std::hint::black_box(measure_candidate_has_duplicates(&candidate));
    }

    let mut former_a = Vec::with_capacity(SAMPLES);
    let mut former_b = Vec::with_capacity(SAMPLES);
    let mut candidate_a = Vec::with_capacity(SAMPLES);
    let mut candidate_b = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let former_a_column = build_unique_typed_i64(N);
        let former_b_column = build_unique_typed_i64(N);
        let candidate_a_column = build_unique_typed_i64(N);
        let candidate_b_column = build_unique_typed_i64(N);
        if sample.is_multiple_of(2) {
            former_a.push(measure_former_has_duplicates(&former_a_column));
            candidate_a.push(measure_candidate_has_duplicates(&candidate_a_column));
            candidate_b.push(measure_candidate_has_duplicates(&candidate_b_column));
            former_b.push(measure_former_has_duplicates(&former_b_column));
        } else {
            former_b.push(measure_former_has_duplicates(&former_b_column));
            candidate_b.push(measure_candidate_has_duplicates(&candidate_b_column));
            candidate_a.push(measure_candidate_has_duplicates(&candidate_a_column));
            former_a.push(measure_former_has_duplicates(&former_a_column));
        }
    }

    let former_a_p50 = median_nanos(&mut former_a);
    let former_b_p50 = median_nanos(&mut former_b);
    let candidate_a_p50 = median_nanos(&mut candidate_a);
    let candidate_b_p50 = median_nanos(&mut candidate_b);
    let former_mean = (former_a_p50 + former_b_p50) as f64 / 2.0;
    let candidate_mean = (candidate_a_p50 + candidate_b_p50) as f64 / 2.0;
    println!("typed Int64 has_duplicates A/B: n={N} samples={SAMPLES}");
    println!("former enum-key p50 A/B: {former_a_p50} / {former_b_p50} ns");
    println!("candidate raw-i64 p50 A/B: {candidate_a_p50} / {candidate_b_p50} ns");
    println!(
        "former/candidate duplicate-p50 ratio: {:.6}x",
        former_mean / candidate_mean
    );
}

fn icol(v: Vec<i64>) -> Column {
    Column::new(DType::Int64, v.into_iter().map(Scalar::Int64).collect()).unwrap()
}

fn dump(c: &Column) -> String {
    let mut s = format!("[{:?}]", c.dtype());
    for v in c.values() {
        match v {
            Scalar::Int64(i) => s.push_str(&format!("{i},")),
            Scalar::Float64(f) => s.push_str(&format!("f{},", f.to_bits())),
            Scalar::Utf8(x) => s.push_str(&format!("{x},")),
            Scalar::Null(_) => s.push_str("N,"),
            o => s.push_str(&format!("{o:?},")),
        }
    }
    s
}

fn golden() -> String {
    let mut out = String::new();
    // bounded int with dups + negatives (dense path), first-seen order.
    out.push_str(&format!(
        "dense:{}\n",
        dump(&icol(vec![3, 1, 3, -2, 1, 5, -2, 3]).unique().unwrap())
    ));
    out.push_str(&format!(
        "single:{}\n",
        dump(&icol(vec![7, 7, 7]).unique().unwrap())
    ));
    out.push_str(&format!(
        "empty:{}\n",
        dump(&icol(vec![]).unique().unwrap())
    ));
    // huge-range int (range > cap) -> HashSet fallback
    out.push_str(&format!(
        "wide:{}\n",
        dump(&icol(vec![0, 1_000_000_000, 0, 5]).unique().unwrap())
    ));
    // nullable int -> HashSet path (missing skipped)
    let ni = Column::new(
        DType::Int64,
        vec![
            Scalar::Int64(2),
            Scalar::Null(NullKind::NaN),
            Scalar::Int64(2),
            Scalar::Int64(9),
        ],
    )
    .unwrap();
    out.push_str(&format!("nullable:{}\n", dump(&ni.unique().unwrap())));
    // strings -> HashSet path
    let s = Column::new(
        DType::Utf8,
        ["b", "a", "b", "c", "a"]
            .iter()
            .map(|x| Scalar::Utf8((*x).into()))
            .collect(),
    )
    .unwrap();
    out.push_str(&format!("utf8:{}\n", dump(&s.unique().unwrap())));
    out
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("has-duplicates-attribution") {
        run_has_duplicates_attribution();
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("has-duplicates-ab") {
        run_has_duplicates_ab();
        return;
    }

    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    // Bounded Int64, low cardinality but large n (heavy dedup work).
    let n: usize = 2_000_000;
    let mut x: u64 = 0x000b_ead5;
    let col = icol(
        (0..n)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (x >> 40) as i64 % 100_000
            })
            .collect(),
    );

    let _ = col.unique().unwrap(); // warmup

    let t = Instant::now();
    let r = col.unique().unwrap();
    let d = t.elapsed();
    std::hint::black_box(&r);

    println!("TIMING n={n} unique={:.3}ms", d.as_secs_f64() * 1e3);
}
