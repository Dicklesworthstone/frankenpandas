//! Bench + golden for `Series.sort_values` on a large shuffled Int64 column with
//! an Int64 index — exercises the zero-copy reorder (`reorder_by_positions`:
//! `Index::take` for the index, br-frankenpandas-7ufhq, + `Column::take_positions`
//! for the values). `n` rows; values are a deterministic LCG permutation-ish
//! shuffle. Golden = FNV-1a64 over the sorted (index-label, value) pairs, pinning
//! both the value order and the carried index so the gather stays bit-transparent.
//!
//! Run: cargo run -p fp-frame --example bench_sort_values --release -- 1000000 50

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn build(n: usize) -> Series {
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let values: Vec<Scalar> = (0..n)
        .map(|row| {
            let mixed = (row as u64)
                .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .rotate_left(29)
                ^ (row as u64).wrapping_mul(0xD1B5_4A32_D192_ED03);
            Scalar::Int64((mixed % 1_000_003) as i64)
        })
        .collect();
    Series::from_values("v", labels, values).expect("build series")
}

fn fnv1a64_update(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

/// Hash the (index-label, value) pairs in output order so the value order and the
/// carried index labels are both pinned.
fn digest(sorted: &Series) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (lbl, val) in sorted.index().labels().iter().zip(sorted.values().iter()) {
        let l = match lbl {
            IndexLabel::Int64(x) => *x,
            _ => i64::MIN,
        };
        fnv1a64_update(&mut h, &l.to_le_bytes());
        let v = match val {
            Scalar::Int64(x) => *x,
            Scalar::Float64(x) => *x as i64,
            _ => i64::MIN,
        };
        fnv1a64_update(&mut h, &v.to_le_bytes());
    }
    h
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let iters: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);

    let series = build(n);
    let golden = digest(&series.sort_values(true).expect("sort_values"));

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let sorted = series.sort_values(true).expect("sort_values");
        let elapsed = t.elapsed().as_nanos();
        std::hint::black_box(&sorted);
        if elapsed < best {
            best = elapsed;
        }
    }

    println!("sort_values n={n} iters={iters}: best={best}ns golden={golden:016x}");
}
