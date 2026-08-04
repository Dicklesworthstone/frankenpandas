//! Probe: string CategoricalIndex factorize / value_counts / unique /
//! duplicated on a LOW-cardinality Utf8 index (the common categorical case),
//! with a generic Index label-hash baseline on the same labels.
//! Run: cargo run -p fp-index --example probe_str_categorical --release -- 1000000

use std::{hint::black_box, time::Instant};

use fp_index::{CategoricalIndex, DuplicateKeep, Index, IndexLabel};

fn bench(name: &str, batches: usize, iters: usize, mut f: impl FnMut() -> usize) {
    for _ in 0..2 {
        black_box(f());
    }
    let mut best = f64::INFINITY;
    let mut total = 0.0;
    let mut sink = 0usize;
    for _ in 0..batches {
        let start = Instant::now();
        for _ in 0..iters {
            sink ^= black_box(f());
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / iters as f64;
        best = best.min(ms);
        total += ms;
    }
    println!(
        "{name}: best={best:.3} ms/iter mean={:.3} ms/iter (sink={sink})",
        total / batches as f64
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
    let iters: usize = std::env::args()
        .nth(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    let mut z = 0x1234u64;
    let labels: Vec<String> = (0..n)
        .map(|_| {
            z ^= z << 13;
            z ^= z >> 7;
            z ^= z << 17;
            format!("cat_{:04}", (z as usize) % cats)
        })
        .collect();
    let categories: Vec<String> = (0..cats).map(|i| format!("cat_{i:04}")).collect();
    let categorical = CategoricalIndex::with_categories(labels.clone(), categories, false).unwrap();
    let hash_index = Index::new(labels.into_iter().map(IndexLabel::Utf8).collect());
    let categories: Vec<String> = (0..cats).map(|i| format!("cat_{i:04}")).collect();
    let monotonic_labels: Vec<String> = (0..n)
        .map(|i| format!("cat_{:04}", i.saturating_mul(cats) / n.max(1)))
        .collect();
    let monotonic = CategoricalIndex::with_categories(monotonic_labels, categories, true).unwrap();

    bench("cat_factorize", 5, iters, || {
        let (codes, uniques) = categorical.factorize();
        codes.len() ^ uniques.len()
    });
    bench("hash_factorize", 5, iters, || {
        let (codes, uniques) = hash_index.factorize();
        codes.len() ^ uniques.len()
    });
    bench("cat_value_counts", 5, iters, || {
        categorical.value_counts().len()
    });
    bench("cat_nunique", 5, iters, || categorical.nunique());
    bench("cat_unique", 5, iters, || categorical.unique().len());
    bench("cat_monotonic_increasing", 5, iters, || {
        usize::from(monotonic.is_monotonic_increasing())
    });
    bench("cat_duplicated", 5, iters, || {
        categorical
            .duplicated(DuplicateKeep::First)
            .iter()
            .filter(|b| **b)
            .count()
    });
    bench("hash_duplicated", 5, iters, || {
        hash_index
            .duplicated(DuplicateKeep::First)
            .iter()
            .filter(|b| **b)
            .count()
    });
}
