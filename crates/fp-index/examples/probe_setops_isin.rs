//! Probe: Index::intersection + Index::isin on UNSORTED int64 (the
//! FxHashMap<&IndexLabel> fallback paths). Run:
//!   cargo run -p fp-index --example probe_setops_isin --release -- 1000000

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
    let ms = start.elapsed().as_secs_f64() * 1000.0 / iters as f64;
    println!("{name}: {ms:.3} ms/iter (sink={sink})");
}

fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let mut z = 0x1234_5678u64;
    let mut perm: Vec<i64> = (0..n as i64).collect();
    for i in (1..n).rev() {
        z ^= z << 13;
        z ^= z >> 7;
        z ^= z << 17;
        let j = (z as usize) % (i + 1);
        perm.swap(i, j);
    }
    let a = Index::new(perm.iter().map(|&v| IndexLabel::Int64(v)).collect());
    let b = Index::new((0..n as i64).map(|i| IndexLabel::Int64(i * 2)).collect());
    let vals: Vec<IndexLabel> = (0..n as i64).map(|i| IndexLabel::Int64(i * 2)).collect();

    bench("intersection", 8, || a.intersection(&b).len());
    bench("union", 8, || a.union(&b).len());
    bench("difference", 8, || a.difference(&b).len());
    bench("symmetric_difference", 8, || {
        a.symmetric_difference(&b).len()
    });
    bench("isin", 8, || a.isin(&vals).iter().filter(|x| **x).count());
}
