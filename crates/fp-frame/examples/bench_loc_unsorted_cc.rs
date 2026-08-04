//! Series.loc[[labels]] on a non-unit-range, UNSORTED unique Int64 index.
//! Sorted indexes hit sorted_unique_int64_positions (0pkt2); this exercises the
//! unsorted-unique fallback that still builds the per-call pointer-key map.
//! Run: cargo run -p fp-frame --example bench_loc_unsorted_cc --release -- 2000000 1000 50

use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_000_000);
    let k: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let iters: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(50);

    // Unique Int64 labels 0..2n step-2, then Fisher-Yates shuffle with an LCG so
    // the index is unique but NOT sorted ⇒ binary-search path declines, exercises
    // the unsorted-unique fallback. Deterministic (no rand crate).
    let mut keys: Vec<i64> = (0..n as i64).map(|i| i * 2).collect();
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in (1..n).rev() {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let j = (state >> 33) as usize % (i + 1);
        keys.swap(i, j);
    }
    let labels: Vec<IndexLabel> = keys.iter().map(|&v| IndexLabel::Int64(v)).collect();
    let s = Series::from_values(
        "v",
        labels,
        (0..n).map(|i| Scalar::Float64(i as f64)).collect(),
    )
    .unwrap();
    // select k existing labels spread through the shuffled key space
    let sel: Vec<IndexLabel> = (0..k)
        .map(|j| IndexLabel::Int64(keys[(j * (n / k)) % n]))
        .collect();

    let mut best = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        let out = s.loc(&sel).expect("loc");
        let e = t.elapsed().as_nanos();
        std::hint::black_box(&out);
        if e < best {
            best = e;
        }
    }
    println!("loc_unsorted_i64 n={n} k={k}: best={best}ns");
}
