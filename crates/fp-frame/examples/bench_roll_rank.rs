//! Series.rolling(w).rank() shuffled. Run: -- 1000000 100
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn sm(i: usize) -> f64 {
    let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    (h >> 1) as f64
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let w: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(6);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "c",
        labels,
        (0..n).map(|i| Scalar::Float64(sm(i))).collect(),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        std::hint::black_box(s.rolling(w, Some(w)).rank("average", true, "keep").unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("roll_rank n={n} w={w}: best={best}ns");
}
