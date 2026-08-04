//! Index::get_indexer on an UNSORTED unique Utf8 index, repeated (mirrors repeated
//! reindex/align/join of the same frame — pandas caches its index engine for this;
//! fp previously rebuilt the pointer-key position map every call). Lever: c90bo.
//! Run: cargo run -p fp-frame --example bench_get_indexer_utf8_cc --release -- 1000000 30

use std::time::Instant;

use fp_index::{Index, IndexLabel};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);

    let mut keys: Vec<usize> = (0..n).collect();
    let mut st: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in (1..n).rev() {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = (st >> 33) as usize % (i + 1);
        keys.swap(i, j);
    }
    let labels: Vec<IndexLabel> = keys
        .iter()
        .map(|&i| IndexLabel::Utf8(format!("k{i:08}")))
        .collect();
    let idx = Index::new(labels);
    let tgt: Vec<IndexLabel> = (0..1000)
        .map(|j| IndexLabel::Utf8(format!("k{:08}", j * (n / 1000))))
        .collect();
    let target = Index::new(tgt);

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = idx.get_indexer(&target);
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!(
        "get_indexer_utf8 n={n} m=1000: best={best}ns ({:.4}ms)",
        best as f64 / 1e6
    );
}
