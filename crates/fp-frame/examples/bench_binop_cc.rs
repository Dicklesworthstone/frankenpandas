//! Series binary ops on same index: add, mul, gt. 2M Float64. Verify no align pathology.
//! Run: cargo run -p fp-frame --example bench_binop_cc --release -- 2000000 30

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
    let a = Series::from_values(
        "a",
        labels.clone(),
        (0..n as i64)
            .map(|x| Scalar::Float64(x as f64 * 1.5))
            .collect(),
    )
    .unwrap();
    let b = Series::from_values(
        "b",
        labels,
        (0..n as i64)
            .map(|x| Scalar::Float64(x as f64 + 0.5))
            .collect(),
    )
    .unwrap();

    let add = best(iters, || {
        std::hint::black_box(a.add(&b).expect("add"));
    });
    let mul = best(iters, || {
        std::hint::black_box(a.mul(&b).expect("mul"));
    });
    let gt = best(iters, || {
        std::hint::black_box(a.gt(&b).expect("gt"));
    });
    println!("binop n={n}: add={add}ns mul={mul}ns gt={gt}ns");
}
