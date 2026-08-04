//! Bench for the typed Int64 reduction levers (sum bwgyc, max/min 4qs3h, var/std via
//! numeric_moments 0xdfx) on a large all-valid Int64 column. Self-times best-of-N per op.
//! Compare against pandas Series.{sum,max,min,std,var} on the same workload.
//!
//! Run: cargo run -p fp-frame --example bench_reductions --release -- 2000000 100

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn build(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n as i64).map(Scalar::Int64).collect();
    Series::from_values("v", labels, values).expect("build series")
}

fn best<F: Fn()>(iters: usize, f: F) -> u128 {
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
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let s = build(n);

    let sum = best(iters, || {
        std::hint::black_box(s.sum().unwrap());
    });
    let max = best(iters, || {
        std::hint::black_box(s.max().unwrap());
    });
    let min = best(iters, || {
        std::hint::black_box(s.min().unwrap());
    });
    let std = best(iters.min(50), || {
        std::hint::black_box(s.std().unwrap());
    });
    let var = best(iters.min(50), || {
        std::hint::black_box(s.var().unwrap());
    });

    println!("reductions n={n}: sum={sum}ns max={max}ns min={min}ns std={std}ns var={var}ns");
}
