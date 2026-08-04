//! interpolate/clip/ffill/cumsum(series), shuffled. Run: -- 5000000 clip
use std::time::Instant;

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
fn sm(i: usize) -> f64 {
    let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    (h >> 11) as f64 * 1e-6
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(5_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("clip");
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // for ffill/interpolate, inject some nulls
    let vals: Vec<f64> = (0..n).map(sm).collect();
    let s = Series::new(
        "v".to_string(),
        Index::new(labels),
        Column::from_f64_values(vals),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        let r = match op {
            "clip" => s.clip(Some(-1.0), Some(1.0)).unwrap(),
            "ffill" => s.ffill(None).unwrap(),
            "interpolate" => s.interpolate().unwrap(),
            "cumsum" => s.cumsum().unwrap(),
            "abs" => s.abs().unwrap(),
            _ => panic!(),
        };
        std::hint::black_box(r);
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("{op} n={n}: best={best}ns");
}
