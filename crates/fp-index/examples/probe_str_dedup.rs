//! Probe: duplicate-heavy unsorted Utf8 Index unique/nunique/value_counts/
//! duplicated/drop_duplicates vs the FxHashMap<&IndexLabel> path.
//! Run: cargo run -p fp-index --example probe_str_dedup --release -- 1000000 10000

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
    let mut args = std::env::args().skip(1);
    let n: usize = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let card: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(10_000);
    let mut z = 0x9E37_79B9u64;
    let a = Index::new(
        (0..n)
            .map(|_| {
                z ^= z << 13;
                z ^= z >> 7;
                z ^= z << 17;
                IndexLabel::Utf8(format!("item_{:08}", (z as usize) % card))
            })
            .collect(),
    );

    bench("unique", 6, || a.unique().labels().len());
    bench("nunique", 6, || a.nunique());
    bench("value_counts", 6, || a.value_counts().len());
    bench("duplicated", 6, || {
        a.duplicated(DuplicateKeep::First)
            .iter()
            .filter(|x| **x)
            .count()
    });
    bench("drop_duplicates", 6, || a.drop_duplicates().labels().len());
}
