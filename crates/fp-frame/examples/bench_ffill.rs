//! Bench for Series.ffill on a Float64 column with periodic NaN gaps (typed
//! as_f64_slice_with_validity path). Tests whether the "column-rebuild = loss vs pandas"
//! pattern (seen in shift/concat) also holds for ffill. Compare vs pandas s.ffill().
//!
//! Run: cargo run -p fp-frame --example bench_ffill --release -- 2000000 50

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn build(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // ~10% NaN gaps to forward-fill.
    let values: Vec<Scalar> = (0..n)
        .map(|i| {
            if i % 10 == 3 {
                Scalar::Float64(f64::NAN)
            } else {
                Scalar::Float64(i as f64 * 1.5)
            }
        })
        .collect();
    Series::from_values("v", labels, values).expect("build series")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
    let s = build(n);
    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = s.ffill(None).expect("ffill");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!("ffill n={n} iters={iters}: best={best}ns");
}
