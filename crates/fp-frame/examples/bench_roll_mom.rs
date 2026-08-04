//! rolling skew/kurt/sem/std + apply, shuffled. Run: -- 1000000 100 skew
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn sm(i: usize) -> f64 {
    let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    (h >> 11) as f64 * 1e-6
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let w: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let op = a.get(3).map(String::as_str).unwrap_or("skew");
    let it: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(6);
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
        let r = match op {
            "skew" => s.rolling(w, Some(w)).skew().unwrap(),
            "kurt" => s.rolling(w, Some(w)).kurt().unwrap(),
            "sem" => s.rolling(w, Some(w)).sem().unwrap(),
            "std" => s.rolling(w, Some(w)).std().unwrap(),
            _ => panic!("op"),
        };
        std::hint::black_box(r);
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("roll_{op} n={n} w={w}: best={best}ns");
}
