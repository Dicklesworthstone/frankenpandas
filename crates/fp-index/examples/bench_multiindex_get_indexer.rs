//! Bench + golden digest for MultiIndex::get_indexer (br-frankenpandas-mipack).
//!
//! Run: cargo run -p fp-index --example bench_multiindex_get_indexer --release -- <n>
//!
//! The packed-key path dictionary-encodes each level and packs the tuple into
//! one u64, so the lookup hashes an integer per row instead of allocating a
//! Vec<IndexLabel> (and cloning Utf8 Strings) per row. `FP_NO_MIPACK=1` forces
//! the Vec<IndexLabel>-key baseline; the printed `chk` (FNV digest of the
//! indexer) must match between the two runs.

use std::{hint::black_box, time::Instant};

use fp_index::{IndexLabel, MultiIndex};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let k = 1000usize; // k * (n/k) unique 2-level tuples

    let l0: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Utf8(format!("a{}", i / k)))
        .collect();
    let l1: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Utf8(format!("b{}", i % k)))
        .collect();
    let source = MultiIndex::from_arrays(vec![l0, l1]).expect("source");
    // Target = same tuples reversed (all present) -> full lookup, all hits.
    let t0: Vec<IndexLabel> = (0..n)
        .rev()
        .map(|i| IndexLabel::Utf8(format!("a{}", i / k)))
        .collect();
    let t1: Vec<IndexLabel> = (0..n)
        .rev()
        .map(|i| IndexLabel::Utf8(format!("b{}", i % k)))
        .collect();
    let target = MultiIndex::from_arrays(vec![t0, t1]).expect("target");

    let indexer = source.get_indexer(&target).expect("get_indexer");
    let mut chk: u64 = 0xcbf29ce484222325;
    for v in &indexer {
        chk = (chk ^ (*v as u64)).wrapping_mul(0x100000001b3);
    }

    let mut sink = 0i64;
    let start = Instant::now();
    for _ in 0..iters {
        let ix = black_box(source.get_indexer(&target).expect("get_indexer"));
        sink = sink.wrapping_add(ix.len() as i64);
    }
    let elapsed = start.elapsed();
    println!(
        "mi_get_indexer n={n} iters={iters}: {:.3} ms/iter (chk={chk:016x} sink={sink})",
        elapsed.as_secs_f64() * 1000.0 / iters as f64
    );
}
