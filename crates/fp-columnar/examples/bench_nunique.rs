//! Bench + golden for Column::nunique — dense direct-address for bounded Int64.
//!
//! Run: cargo run -p fp-columnar --example bench_nunique --release
//!
//! nunique counted distinct via fp-types nannunique (SipHash); an all-valid
//! bounded-range Int64 column counts via a seen-bitset indexed by (v-min),
//! hash-free. Non-bounded / non-Int64 / nullable keep the nannunique path.

use std::time::Instant;

use fp_columnar::{Column, ValidityMask};
use fp_types::{DType, NullKind, Scalar, nannunique};

fn icol(v: Vec<i64>) -> Column {
    Column::new(DType::Int64, v.into_iter().map(Scalar::Int64).collect()).unwrap()
}

fn ds(s: &Scalar) -> String {
    match s {
        Scalar::Int64(i) => format!("{i}"),
        o => format!("{o:?}"),
    }
}

fn golden() -> String {
    let mut out = String::new();
    // bounded int (dense)
    out.push_str(&format!(
        "dense:{}\n",
        ds(&icol(vec![3, 1, 3, -2, 1, 5, -2, 3]).nunique())
    ));
    out.push_str(&format!("single:{}\n", ds(&icol(vec![7, 7, 7]).nunique())));
    out.push_str(&format!("empty:{}\n", ds(&icol(vec![]).nunique())));
    // wide range -> nannunique fallback
    out.push_str(&format!(
        "wide:{}\n",
        ds(&icol(vec![0, 1_000_000_000, 0, 5]).nunique())
    ));
    // nullable: dropna true vs false (nannunique path; missing skipped/+1)
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
    out.push_str(&format!(
        "null_drop:{}\n",
        ds(&ni.nunique_with_dropna(true))
    ));
    out.push_str(&format!(
        "null_keep:{}\n",
        ds(&ni.nunique_with_dropna(false))
    ));
    // utf8
    let s = Column::new(
        DType::Utf8,
        ["b", "a", "b", "c", "a"]
            .iter()
            .map(|x| Scalar::Utf8((*x).into()))
            .collect(),
    )
    .unwrap();
    out.push_str(&format!("utf8:{}\n", ds(&s.nunique())));
    out
}

fn reference_nunique_with_dropna(col: &Column, dropna: bool) -> Scalar {
    let values = col.values();
    let mut distinct = match nannunique(values) {
        Scalar::Int64(count) => count,
        _ => 0,
    };
    if !dropna && values.iter().any(Scalar::is_missing) {
        distinct += 1;
    }
    Scalar::Int64(distinct)
}

fn percentile_ms(samples: &mut [f64], percentile: usize) -> f64 {
    samples.sort_by(f64::total_cmp);
    let index = (samples.len() - 1) * percentile / 100;
    match samples.get(index) {
        Some(sample) => *sample * 1e3,
        None => 0.0,
    }
}

fn bench_nullable_bool() {
    let n = std::env::args()
        .nth(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(5_000_000usize);
    let iters = std::env::args()
        .nth(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or(9usize)
        .max(1);

    let data: Vec<bool> = (0..n).map(|i| i % 3 == 0).collect();
    let mut validity = ValidityMask::all_valid(n);
    for i in (0..n).step_by(4) {
        validity.set(i, false);
    }
    let candidate = Column::from_bool_values_with_validity(data.clone(), validity.clone());
    let control = Column::from_bool_values_with_validity(data, validity);

    for dropna in [true, false] {
        assert_eq!(
            candidate.nunique_with_dropna(dropna),
            reference_nunique_with_dropna(&control, dropna)
        );
    }

    std::hint::black_box(candidate.nunique_with_dropna(false));
    std::hint::black_box(reference_nunique_with_dropna(&control, false));

    let mut new_samples = Vec::with_capacity(iters);
    let mut control_samples = Vec::with_capacity(iters);
    for iteration in 0..iters {
        let mut time_new = || {
            let start = Instant::now();
            std::hint::black_box(candidate.nunique_with_dropna(false));
            new_samples.push(start.elapsed().as_secs_f64());
        };
        let mut time_control = || {
            let start = Instant::now();
            std::hint::black_box(reference_nunique_with_dropna(&control, false));
            control_samples.push(start.elapsed().as_secs_f64());
        };
        if iteration % 2 == 0 {
            time_new();
            time_control();
        } else {
            time_control();
            time_new();
        }
    }

    let new_p50 = percentile_ms(&mut new_samples, 50);
    let new_p95 = percentile_ms(&mut new_samples, 95);
    let control_p50 = percentile_ms(&mut control_samples, 50);
    let control_p95 = percentile_ms(&mut control_samples, 95);
    println!(
        "nunique nullable-bool n={n} NEW(p50/p95)={new_p50:.3}/{new_p95:.3}ms \
         CONTROL(p50/p95)={control_p50:.3}/{control_p95:.3}ms speedup={:.3}x",
        control_p50 / new_p50
    );
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("nullable-bool") {
        bench_nullable_bool();
        return;
    }

    let g = golden();
    print!("GOLDEN_BEGIN\n{g}GOLDEN_END\n");

    let n: usize = 4_000_000;
    let mut x: u64 = 0x000f_ee15;
    let col = icol(
        (0..n)
            .map(|_| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (x >> 40) as i64 % 200_000
            })
            .collect(),
    );

    let _ = col.nunique(); // warmup

    let t = Instant::now();
    let mut sink = 0i64;
    for _ in 0..3 {
        if let Scalar::Int64(c) = col.nunique() {
            sink = sink.wrapping_add(c);
        }
    }
    let d = t.elapsed();
    std::hint::black_box(sink);

    println!("TIMING n={n} nunique_x3={:.3}ms", d.as_secs_f64() * 1e3);
}
