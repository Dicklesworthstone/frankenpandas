//! rolling corr/cov, shuffled. Run: -- 1000000 100 corr
use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> f64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    (h >> 11) as f64 * 1e-6
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let w: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let op = a.get(3).map(String::as_str).unwrap_or("corr");
    let it: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(6);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s1 = Series::from_values(
        "a",
        labels.clone(),
        (0..n).map(|i| Scalar::Float64(sm(i, 0))).collect(),
    )
    .unwrap();
    let s2 = Series::from_values(
        "b",
        labels,
        (0..n).map(|i| Scalar::Float64(sm(i, 99))).collect(),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        let r = match op {
            "corr" => s1.rolling(w, Some(w)).corr(&s2).unwrap(),
            "cov" => s1.rolling(w, Some(w)).cov(&s2).unwrap(),
            _ => panic!(),
        };
        std::hint::black_box(r);
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("roll_{op} n={n} w={w}: best={best}ns");
}
