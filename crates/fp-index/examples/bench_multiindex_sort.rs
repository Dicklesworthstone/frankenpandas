//! Bench + golden digest for MultiIndex::argsort/sort_values (br-frankenpandas-misort).
//!
//! Run: cargo run -p fp-index --example bench_multiindex_sort --release -- <n>
//!
//! The packed-key path sorts one u64 per row (ascending u64 == lexicographic
//! tuple order) instead of comparing Utf8 tuples. `FP_NO_MISORT=1` forces the
//! tuple-comparison baseline; the printed `chk` (FNV digest of the argsort
//! permutation) must match between the two runs.

use std::{hint::black_box, time::Instant};

use fp_index::{IndexLabel, MultiIndex};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let k = 1000usize;

    // Deterministically shuffled 2-Utf8-level tuples (so the sort must reorder).
    let mut l0 = Vec::with_capacity(n);
    let mut l1 = Vec::with_capacity(n);
    for i in 0..n {
        let mixed = (i as u64)
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .rotate_left(17) as usize;
        l0.push(IndexLabel::Utf8(format!("a{}", (mixed / k) % k)));
        l1.push(IndexLabel::Utf8(format!("b{}", mixed % k)));
    }
    let mi = MultiIndex::from_arrays(vec![l0, l1]).expect("mi");

    let order = mi.argsort();
    let mut chk: u64 = 0xcbf29ce484222325;
    for &p in &order {
        chk = (chk ^ p as u64).wrapping_mul(0x100000001b3);
    }

    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..iters {
        sink ^= black_box(mi.argsort()).len();
    }
    let elapsed = start.elapsed();
    println!(
        "mi_argsort n={n} iters={iters}: {:.3} ms/iter (chk={chk:016x} sink={sink})",
        elapsed.as_secs_f64() * 1000.0 / iters as f64
    );
}
