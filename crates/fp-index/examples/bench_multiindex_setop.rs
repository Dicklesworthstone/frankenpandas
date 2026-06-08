//! Bench + golden digest for MultiIndex::intersection/difference
//! (br-frankenpandas-misetop).
//!
//! Run: cargo run -p fp-index --example bench_multiindex_setop --release -- <n>
//!
//! The packed-key path hashes identity-coded u64s instead of building a
//! HashMap<Vec<IndexLabel>> from to_list() (per-row Vec + Utf8 clone).
//! `FP_NO_MISETOP=1` forces the to_list baseline; the printed `chk` (FNV digest
//! of the result tuples) must match between the two runs.

use std::hint::black_box;
use std::time::Instant;

use fp_index::{IndexLabel, MultiIndex};

fn mk(n: usize, off: usize) -> MultiIndex {
    let l0: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Utf8(format!("g{}", (i + off) % 700))).collect();
    let l1: Vec<IndexLabel> = (0..n).map(|i| IndexLabel::Utf8(format!("h{}", ((i + off) / 7) % 700))).collect();
    MultiIndex::from_arrays(vec![l0, l1]).expect("mi")
}

fn digest(mi: &MultiIndex) -> u64 {
    let mut chk: u64 = 0xcbf29ce484222325;
    for tuple in mi.to_list() {
        for lbl in tuple {
            let h = match lbl {
                IndexLabel::Utf8(s) => s.bytes().fold(1469598103934665603u64, |a, b| (a ^ b as u64).wrapping_mul(0x100000001b3)),
                IndexLabel::Int64(v) => v as u64,
                _ => 0xDEAD,
            };
            chk = (chk ^ h).wrapping_mul(0x100000001b3);
        }
    }
    chk ^ (mi.len() as u64).wrapping_mul(0x9E3779B97F4A7C15)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let a = mk(n, 0);
    let b = mk(n, n / 3); // ~partial overlap

    let chk = digest(&a.intersection(&b).unwrap()) ^ digest(&a.difference(&b).unwrap()).rotate_left(1);

    let mut sink = 0usize;
    let start = Instant::now();
    for _ in 0..iters {
        sink ^= black_box(a.intersection(&b).unwrap()).len();
        sink ^= black_box(a.difference(&b).unwrap()).len();
    }
    let elapsed = start.elapsed();
    println!(
        "mi_setop n={n} iters={iters}: {:.3} ms/iter (chk={chk:016x} sink={sink})",
        elapsed.as_secs_f64() * 1000.0 / iters as f64
    );
}
