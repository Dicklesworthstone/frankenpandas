//! Micro-benchmark for `Index::intersection` on two sorted, materialized Int64
//! indexes (br-frankenpandas-idxdup set ops). Both strictly ascending => a
//! hash-free two-pointer merge replaces building two FxHashMaps.
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

    // Two strictly-ascending Int64 indexes with ~50% overlap.
    let a: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64 * 2)).collect();
    let b: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Int64(i as i64 * 2 + n as i64))
        .collect();
    let ia = Index::new(a);
    let ib = Index::new(b);

    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..iters {
        sink ^= black_box(ia.intersection(&ib)).len();
    }
    let elapsed = start.elapsed();
    println!(
        "intersection n={n} iters={iters}: {:.3} ms/iter (sink={sink})",
        elapsed.as_secs_f64() * 1000.0 / iters as f64
    );
}
