//! Bench: nullable-Int64 Column::where_cond_series (col-vs-col other). The typed arm gated
//! on BOTH all-valid, so a nullable Int64 self/other fell to the generic Vec<Scalar> select.
//! NEW = typed i64 select over both backings + validity.
//! Run: cargo run -p fp-columnar --release --example bench_where_series_i64null -- 5000000 20

use fp_columnar::{Column, ValidityMask};
use fp_types::{NullKind, Scalar};

fn build(n: usize, seed: u64) -> (Column, Vec<Scalar>) {
    let mut state = seed;
    let mut data = vec![0i64; n];
    let mut validity = ValidityMask::all_valid(n);
    let mut sc: Vec<Scalar> = Vec::with_capacity(n);
    for i in 0..n {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        if (state >> 40) % 3 == 0 {
            validity.set(i, false);
            sc.push(Scalar::Null(NullKind::Null));
        } else {
            data[i] = ((state >> 20) % 1_000_000) as i64;
            sc.push(Scalar::Int64(data[i]));
        }
    }
    (Column::from_i64_values_with_validity(data, validity), sc)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);

    let (s_col, s_sc) = build(n, 0x0AAA);
    let (o_col, o_sc) = build(n, 0x0BBB);
    let mut state = 0xCCCCu64;
    let cbits: Vec<bool> = (0..n)
        .map(|_| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            (state >> 33) & 1 == 0
        })
        .collect();
    let cond = Column::from_bool_values(cbits.clone());

    let got = s_col.where_cond_series(&cond, &o_col).unwrap();
    assert_eq!(got.dtype(), fp_types::DType::Int64);
    println!("where_cond_series(nullable Int64) OK");

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
        "where_cond_series nullable-Int64 n={n} NEW={:.2}ms REF(materialize)={:.2}ms ({:.1}x)",
        best_new as f64 / 1e6,
        best_ref as f64 / 1e6,
        best_ref as f64 / best_new as f64,
    );
}
