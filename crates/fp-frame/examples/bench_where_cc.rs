//! Series.where / mask head-to-head. 2M Float64, 50% bool cond, scalar other.
//! Verifies the eydcr typed where/mask claim. Run: ... bench_where_cc --release -- 2000000 30

use std::time::Instant;

use fp_frame::Series;
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
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "v",
        labels.clone(),
        (0..n as i64)
            .map(|x| Scalar::Float64(x as f64 * 1.5))
            .collect(),
    )
    .unwrap();
    let cond = Series::from_values(
        "c",
        labels,
        (0..n).map(|i| Scalar::Bool(i % 2 == 0)).collect(),
    )
    .unwrap();
    let other = Scalar::Float64(0.0);

    let w = best(iters, || {
        std::hint::black_box(s.where_cond(&cond, Some(&other)).expect("where"));
    });
    let m = best(iters, || {
        std::hint::black_box(s.mask(&cond, Some(&other)).expect("mask"));
    });
    println!("where_mask n={n}: where={w}ns mask={m}ns");
}
