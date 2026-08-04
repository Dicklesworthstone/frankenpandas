//! Probe: align inner/left on two Int64 indexes (sorted + unsorted), ~50% overlap.
//! Run: cargo run -p fp-index --example probe_align_modes --release -- 1000000

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
    let lu = shuf((0..n as i64).collect(), &mut z);
    let ru = shuf((0..n as i64).map(|i| i + n as i64 / 2).collect(), &mut z);

    bench("inner_unsorted", 6, || {
        align(&lu, &ru, AlignMode::Inner).union_index.len()
    });
    bench("left_unsorted", 6, || {
        align(&lu, &ru, AlignMode::Left).left_positions.len()
    });
}
