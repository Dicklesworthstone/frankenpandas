//! Probe: Index unique / duplicated / drop_duplicates on UNSORTED int64.
//! Run: cargo run -p fp-index --example probe_dedup --release -- 1000000

use std::{hint::black_box, time::Instant};

use fp_index::{DuplicateKeep, Index, IndexLabel};

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
    // ~10% duplicates: values in [0, 0.9n), shuffled.
    let mut vals: Vec<i64> = (0..n).map(|i| (i as i64) * 9 / 10).collect();
    for i in (1..n).rev() {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        let j = (z as usize) % (i + 1);
        vals.swap(i, j);
    }
    let idx = Index::new(vals.iter().map(|&v| IndexLabel::Int64(v)).collect());

    bench("unique", 8, || idx.unique().labels().len());
    bench("duplicated_first", 8, || {
        idx.duplicated(DuplicateKeep::First)
            .iter()
            .filter(|b| **b)
            .count()
    });
    bench("duplicated_none", 8, || {
        idx.duplicated(DuplicateKeep::None)
            .iter()
            .filter(|b| **b)
            .count()
    });
    bench("drop_duplicates", 8, || {
        idx.drop_duplicates().labels().len()
    });
}
