//! Bench: nullable-temporal Column::where_cond_series (col-vs-col other). No typed arm ->
//! generic Vec<Scalar> clone/cast select. NEW = typed i64-ns select over both backings.
//! Run: cargo run -p fp-columnar --release --example bench_where_series_temporal -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

fn build(n: usize, base: i64, seed: u64) -> (Column, Vec<Scalar>, Vec<bool>) {
    let mut state = seed;
    let mut data = vec![0i64; n];
    let mut validity = ValidityMask::all_valid(n);
    let mut vbits = vec![true; n];
    for i in 0..n {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if (state >> 40) % 4 == 0 {
            validity.set(i, false);
            vbits[i] = false;
        } else {
            data[i] = base + ((state >> 20) % 10_000_000) as i64;
        }
    }
    let scalars: Vec<Scalar> = (0..n)
        .map(|i| if vbits[i] { Scalar::Datetime64(data[i]) } else { Scalar::Null(NullKind::NaT) })
        .collect();
    (Column::from_datetime64_values_with_validity(data, validity), scalars, vbits)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let base: i64 = 1_600_000_000_000_000_000;

    let (s_col, s_sc, _) = build(n, base, 0x0AAA);
    let (o_col, o_sc, _) = build(n, base, 0x0BBB);
    let mut state = 0xCCCCu64;
    let cbits: Vec<bool> = (0..n)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 33) & 1 == 0
        })
        .collect();
    let cond = Column::from_bool_values(cbits.clone());

    let got = s_col.where_cond_series(&cond, &o_col).unwrap();
    assert_eq!(got.dtype(), fp_types::DType::Datetime64);
    println!("where_cond_series(Datetime64) OK");

    let mut best_new = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let o = s_col.where_cond_series(&cond, &o_col).unwrap();
        best_new = best_new.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    let mut best_ref = u128::MAX;
    for _ in 0..iters {
        let t = std::time::Instant::now();
        let out: Vec<Scalar> = (0..n)
            .map(|i| if cbits[i] { s_sc[i].clone() } else { o_sc[i].clone() })
            .collect();
        let o = Column::from_values(out).unwrap();
        best_ref = best_ref.min(t.elapsed().as_nanos());
        std::hint::black_box(&o);
    }
    println!(
        "where_cond_series nullable-Datetime64 n={n} NEW={:.2}ms REF(materialize)={:.2}ms ({:.1}x)",
        best_new as f64 / 1e6,
        best_ref as f64 / 1e6,
        best_ref as f64 / best_new as f64,
    );
}
