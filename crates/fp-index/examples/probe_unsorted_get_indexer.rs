//! Probe: Index::get_indexer on an UNSORTED bounded-int64 index (the FxHashMap
//! fallback path) — to decide whether a dense direct-address lever is worth it.
//! Run: cargo run -p fp-index --example probe_unsorted_get_indexer --release -- 1000000

use std::{hint::black_box, time::Instant};

use fp_index::{Index, IndexLabel};

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    // Unsorted self: a shuffled permutation of 0..n (bounded range, all distinct).
    let mut z = 0x1234_5678u64;
    let mut perm: Vec<i64> = (0..n as i64).collect();
    for i in (1..n).rev() {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        let j = (z as usize) % (i + 1);
        perm.swap(i, j);
    }
    let self_labels: Vec<IndexLabel> = perm.iter().map(|&v| IndexLabel::Int64(v)).collect();
    let target_labels: Vec<IndexLabel> = (0..n as i64).map(|i| IndexLabel::Int64(i * 2)).collect();
    let idx = Index::new(self_labels);
    let target = Index::new(target_labels);

    for _ in 0..3 {
        black_box(idx.get_indexer(&target));
    }
    let iters = 10;
    let start = Instant::now();
    let mut sink = 0usize;
    for _ in 0..iters {
        let out = black_box(idx.get_indexer(&target));
        sink ^= out.iter().filter(|x| x.is_some()).count();
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    println!("unsorted get_indexer n={n}: {ms:.3} ms/iter (sink={sink})");
}
