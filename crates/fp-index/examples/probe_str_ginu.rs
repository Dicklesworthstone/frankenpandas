//! Probe: get_indexer_non_unique on a duplicate-heavy unsorted Utf8 source.
//! Run: cargo run -p fp-index --example probe_str_ginu --release -- 1000000 10000

use std::{hint::black_box, time::Instant};

use fp_index::{Index, IndexLabel};

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let card: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let mut z = 0x9E37_79B9u64;
    let source = Index::new(
        (0..n)
            .map(|_| {
                z ^= z << 13;
                z ^= z >> 7;
                z ^= z << 17;
                IndexLabel::Utf8(format!("item_{:08}", (z as usize) % card))
            })
            .collect(),
    );
    // target: each distinct id once (some present, some absent)
    let target = Index::new(
        (0..card * 2)
            .map(|i| IndexLabel::Utf8(format!("item_{i:08}")))
            .collect(),
    );

    for _ in 0..2 {
        black_box(source.get_indexer_non_unique(&target));
    }
    let start = Instant::now();
    let iters = 6;
    let mut sink = 0usize;
    for _ in 0..iters {
        let (indexer, missing) = source.get_indexer_non_unique(&target);
        sink ^= indexer.len() ^ missing.len();
    }
    println!(
        "get_indexer_non_unique: {:.3} ms/iter (sink={sink})",
        start.elapsed().as_secs_f64() * 1000.0 / iters as f64
    );
}
