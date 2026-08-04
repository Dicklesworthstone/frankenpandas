//! DataFrame::crosstab head-to-head. n rows, two Utf8 (or i64) categorical cols.
//! Run: cargo run -p fp-frame --example bench_crosstab_cc --release -- 1000000 1000 50 15 str
use std::time::Instant;

use fp_frame::{DataFrame, Series};
use fp_index::IndexLabel;
use fp_types::Scalar;
fn best<F: FnMut()>(iters: usize, mut f: F) -> u128 {
    let mut b = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        f();
        let e = t.elapsed().as_nanos();
        if e < b {
            b = e;
        }
    }
    b
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let n_r: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let n_c: i64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);
    let iters: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(15);
    let kt = a.get(5).map(String::as_str).unwrap_or("str");
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let rk = |i: usize| ((i as i64).wrapping_mul(2654435761) >> 13) % n_r;
    let ck = |i: usize| ((i as i64).wrapping_mul(40503) >> 7) % n_c;
    let (rv, cv): (Vec<Scalar>, Vec<Scalar>) = if kt == "str" {
        (
            (0..n)
                .map(|i| Scalar::Utf8(format!("r{:06}", rk(i))))
                .collect(),
            (0..n)
                .map(|i| Scalar::Utf8(format!("c{:04}", ck(i))))
                .collect(),
        )
    } else {
        (
            (0..n).map(|i| Scalar::Int64(rk(i))).collect(),
            (0..n).map(|i| Scalar::Int64(ck(i))).collect(),
        )
    };
    let rs = Series::from_values("row", labels.clone(), rv).unwrap();
    let cs = Series::from_values("col", labels, cv).unwrap();
    let t = best(iters, || {
        std::hint::black_box(DataFrame::crosstab(&rs, &cs).unwrap());
    });
    println!("crosstab n={n} n_r={n_r} n_c={n_c} key={kt}: best={t}ns");
}
