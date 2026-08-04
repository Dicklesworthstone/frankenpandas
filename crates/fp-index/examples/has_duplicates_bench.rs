//! Micro-benchmark for `Index::has_duplicates` on a sorted, materialized Int64
//! index (br-frankenpandas-idxdup). Each iteration clones the base index (Arc
//! label share + cold caches) so `has_duplicates` recomputes from scratch,
//! isolating the strict-ascending fast path from the FxHashMap fallback.
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

    // Strictly ascending, gapped (so it is a materialized label vector, not a
    // unit-range backing that would short-circuit before either path).
    let labels: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64 * 2)).collect();
    let base = Index::new(labels);
    // Intentionally do NOT query base.has_duplicates()/sort_order() here: keep
    // the caches cold so every clone recomputes the answer in the timed loop.

    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..iters {
        let clone = base.clone();
        sink ^= black_box(clone.has_duplicates()) as usize;
    }
    let elapsed = start.elapsed();
    println!(
        "has_duplicates n={n} iters={iters}: {:.3} ms/iter (sink={sink})",
        elapsed.as_secs_f64() * 1000.0 / iters as f64
    );
}
