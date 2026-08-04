//! Micro-benchmark for `Index::unique` on a sorted, materialized Int64 index
//! (br-frankenpandas-idxdup dedup family). A strictly-ascending index is already
//! unique, so the fast path returns an O(1) Arc-sharing clone instead of hashing
//! every label and rebuilding the vector.
//!
//! `FP_NO_IDXDUP=1` forces the old FxHashMap path for the baseline measurement.

use std::{hint::black_box, time::Instant};

use fp_index::{Index, IndexLabel};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);

    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64 * 2)).collect();
    let base = Index::new(labels);

    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..iters {
        sink ^= black_box(base.unique()).len();
    }
    let elapsed = start.elapsed();
    println!(
        "unique n={n} iters={iters}: {:.3} ms/iter (sink={sink})",
        elapsed.as_secs_f64() * 1000.0 / iters as f64
    );
}
