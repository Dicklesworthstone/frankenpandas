//! Probe: Index factorize / get_indexer_non_unique / value_counts on UNSORTED
//! int64. Run: cargo run -p fp-index --example probe_factorize_nonuniq --release -- 1000000

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
    // ~10% dups, shuffled, bounded.
    let mut vals: Vec<i64> = (0..n).map(|i| (i as i64) * 9 / 10).collect();
    for i in (1..n).rev() {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        let j = (z as usize) % (i + 1);
        vals.swap(i, j);
    }
    let idx = Index::new(vals.iter().map(|&v| IndexLabel::Int64(v)).collect());
    let target = Index::new((0..n as i64).map(|i| IndexLabel::Int64(i * 2)).collect());

    bench("factorize", 6, || idx.factorize().0.len());
    bench("get_indexer_non_unique", 6, || {
        idx.get_indexer_non_unique(&target).0.len()
    });
    bench("value_counts", 6, || idx.value_counts().len());
}
