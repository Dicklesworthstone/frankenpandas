//! expanding median/quantile/corr/cov/var, shuffled. Run: -- 1000000 median
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
    let op = a.get(2).map(String::as_str).unwrap_or("median");
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(6);
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
            "median" => s1.expanding(Some(1)).median().unwrap(),
            "quantile" => s1.expanding(Some(1)).quantile(0.25).unwrap(),
            "corr" => s1.expanding(Some(1)).corr(&s2).unwrap(),
            "cov" => s1.expanding(Some(1)).cov(&s2).unwrap(),
            "skew" => s1.expanding(Some(1)).skew().unwrap(),
            _ => panic!(),
        };
        std::hint::black_box(r);
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("exp_{op} n={n}: best={best}ns");
}
