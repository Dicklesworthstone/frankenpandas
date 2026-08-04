//! Probe: unsorted Utf8 Index get_indexer / intersection / unique vs the
//! FxHashMap<&IndexLabel> path. Run:
//!   cargo run -p fp-index --example probe_str_setops --release -- 1000000

use std::{hint::black_box, time::Instant};

use fp_index::{Index, IndexLabel};

fn bench(name: &str, iters: usize, mut f: impl FnMut() -> usize) {
    for _ in 0..2 {
        black_box(f());
    }
    let start = Instant::now();
    let mut sink = 0usize;
    for _ in 0..iters {
        sink ^= black_box(f());
    }
    println!(
        "{name}: {:.3} ms/iter (sink={sink})",
        start.elapsed().as_secs_f64() * 1000.0 / iters as f64
    );
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let mut z = 0x1234_5678u64;
    let mut perm: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        let j = (z as usize) % (i + 1);
        perm.swap(i, j);
    }
    // ~15-byte unique ids, shuffled (unsorted).
    let a = Index::new(
        perm.iter()
            .map(|&i| IndexLabel::Utf8(format!("item_{i:010}")))
            .collect(),
    );
    let b = Index::new(
        (0..n)
            .map(|i| IndexLabel::Utf8(format!("item_{:010}", i * 2)))
            .collect(),
    );
    let vals: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Utf8(format!("item_{:010}", i * 2)))
        .collect();

    bench("get_indexer", 6, || {
        a.get_indexer(&b).iter().filter(|x| x.is_some()).count()
    });
    bench("intersection", 6, || a.intersection(&b).labels().len());
    bench("unique", 6, || a.unique().labels().len());
    bench("isin", 6, || a.isin(&vals).iter().filter(|x| **x).count());
    bench("union", 6, || a.union(&b).labels().len());
    bench("difference", 6, || a.difference(&b).labels().len());
    bench("symmetric_difference", 6, || {
        a.symmetric_difference(&b).labels().len()
    });
}
