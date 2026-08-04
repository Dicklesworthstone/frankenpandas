//! Micro-benchmark for `Index::get_indexer` (br-frankenpandas-idxdup). Two
//! strictly-ascending indexes (the common alignment shape) resolve via a
//! hash-free two-pointer merge instead of building an O(n) FxHashMap of self.
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
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);

    // Sorted self and sorted target with ~50% overlap.
    let a: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Int64(i as i64 * 2)).collect();
    let t: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Int64(i as i64 * 2 + 1))
        .collect();
    let ia = Index::new(a);
    let target = Index::new(t);

    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..iters {
        let out = black_box(ia.get_indexer(&target));
        sink ^= out.iter().filter(|x| x.is_some()).count();
    }
    let elapsed = start.elapsed();
    println!(
        "get_indexer n={n} iters={iters}: {:.3} ms/iter (sink={sink})",
        elapsed.as_secs_f64() * 1000.0 / iters as f64
    );
}
