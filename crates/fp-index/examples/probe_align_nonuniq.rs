//! Probe: align_non_unique (all 4 modes) on duplicate-containing Int64 indexes.
//! Run: cargo run -p fp-index --example probe_align_nonuniq --release -- 1000000

use std::{hint::black_box, time::Instant};

use fp_index::{AlignMode, Index, IndexLabel, align};

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
    let mut z = 0x1234u64;
    let shuf = |base: Vec<i64>, z: &mut u64| -> Index {
        let mut v = base;
        for i in (1..v.len()).rev() {
            *z ^= *z << 13;
            *z ^= *z >> 7;
            *z ^= *z << 17;
            let j = (*z as usize) % (i + 1);
            v.swap(i, j);
        }
        Index::new(v.into_iter().map(IndexLabel::Int64).collect())
    };
    // ~10% duplicates (each value ~1.1 occurrences) so the cartesian stays
    // tame and the grouping cost dominates. ~50% key overlap between sides.
    let l = shuf((0..n).map(|i| (i as i64) * 9 / 10).collect(), &mut z);
    let r = shuf(
        (0..n).map(|i| (i as i64) * 9 / 10 + n as i64 / 2).collect(),
        &mut z,
    );

    bench("inner", 5, || {
        align(&l, &r, AlignMode::Inner).union_index.len()
    });
    bench("left", 5, || {
        align(&l, &r, AlignMode::Left).union_index.len()
    });
    bench("outer", 5, || {
        align(&l, &r, AlignMode::Outer).union_index.len()
    });
}
