//! Series.rank() on Float64 (unique vs tie-heavy). Run: -- 2000000 average <card?>
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let method = a.get(2).map(String::as_str).unwrap_or("average");
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let card: i64 = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(i64::MAX);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values(
        "c",
        labels,
        (0..n)
            .map(|i| {
                let v = {
                    let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
                    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                    h = (h ^ (h >> 27)).wrapping_mul(0x94D049BB133111EB);
                    h ^= h >> 31;
                    (h >> 1) as i64
                }
                .rem_euclid(card);
                Scalar::Float64(v as f64)
            })
            .collect(),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        std::hint::black_box(s.rank(method, true, "keep").unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("rank_f64 n={n} method={method} card={card}: best={best}ns");
}
