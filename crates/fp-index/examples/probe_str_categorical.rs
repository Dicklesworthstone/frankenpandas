//! Probe: string categorical factorize / value_counts / unique / duplicated on
//! a LOW-cardinality Utf8 index (the common categorical case).
//! Run: cargo run -p fp-index --example probe_str_categorical --release -- 1000000

use fp_index::{DuplicateKeep, Index, IndexLabel};
use std::hint::black_box;
use std::time::Instant;

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
    let cats: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let mut z = 0x1234u64;
    let labels: Vec<IndexLabel> = (0..n)
        .map(|_| {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            IndexLabel::Utf8(format!("cat_{:04}", (z as usize) % cats))
        })
        .collect();
    let idx = Index::new(labels);

    bench("factorize", 6, || idx.factorize().0.len());
    bench("value_counts", 6, || idx.value_counts().len());
    bench("unique", 6, || idx.unique().labels().len());
    bench("duplicated", 6, || {
        idx.duplicated(DuplicateKeep::First).iter().filter(|b| **b).count()
    });
}
