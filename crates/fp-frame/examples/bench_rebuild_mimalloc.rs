//! Tests whether the mimalloc 13× concat win (bead 3nah5) GENERALIZES to the other two
//! rebuild-class losses, shift and ffill. Same workloads as bench_shift / bench_ffill, but
//! with mimalloc as the global allocator. If shift/ffill also recover ~10×, a pooling
//! global allocator fixes the entire rebuild-class at once — strong evidence for 3nah5.
//!
//! Run: cargo run -p fp-frame --example bench_rebuild_mimalloc --release -- 2000000 100

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

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
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();

    // shift workload: 2M f64, periods=1 (matches bench_shift f64 path).
    let shift_vals: Vec<Scalar> = (0..n as i64)
        .map(|x| Scalar::Float64(x as f64 * 1.5))
        .collect();
    let shift_s = Series::from_values("v", labels.clone(), shift_vals).unwrap();
    let shift_best = best(iters, || {
        std::hint::black_box(shift_s.shift(1).expect("shift"));
    });

    // ffill workload: 2M f64, ~10% NaN gaps (matches bench_ffill).
    let ffill_vals: Vec<Scalar> = (0..n)
        .map(|i| {
            if i % 10 == 3 {
                Scalar::Float64(f64::NAN)
            } else {
                Scalar::Float64(i as f64 * 1.5)
            }
        })
        .collect();
    let ffill_s = Series::from_values("v", labels, ffill_vals).unwrap();
    let ffill_best = best(iters, || {
        std::hint::black_box(ffill_s.ffill(None).expect("ffill"));
    });

    println!("rebuild_mimalloc n={n} iters={iters}: shift={shift_best}ns ffill={ffill_best}ns");
}
