//! Probe: Index::has_duplicates on an UNSORTED all-unique int64 index (worst
//! case: full scan, no early exit). Clones each iter to reset the cache.
//! Run: cargo run -p fp-index --example probe_has_dup --release -- 1000000

use std::{hint::black_box, time::Instant};

use fp_index::{Index, IndexLabel};

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let mut z = 0x1234_5678u64;
    let mut vals: Vec<i64> = (0..n as i64).collect();
    for i in (1..n).rev() {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        let j = (z as usize) % (i + 1);
        vals.swap(i, j);
    }
    let idx = Index::new(vals.iter().map(|&v| IndexLabel::Int64(v)).collect());
    for _ in 0..2 {
        black_box(idx.clone().has_duplicates());
    }
    let iters = 10;
    let start = Instant::now();
    let mut sink = 0u64;
    for _ in 0..iters {
        sink ^= black_box(idx.clone().has_duplicates()) as u64;
    }
    println!(
        "has_duplicates n={n}: {:.3} ms/iter (sink={sink})",
        start.elapsed().as_secs_f64() * 1000.0 / iters as f64
    );
}
