use std::time::Instant;

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("clip");
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let _idx = IndexLabel::Int64(0);
    let fc: Vec<f64> = (0..n).map(|i| (sm(i, 1) % 100000) as f64).collect();
    let s = Series::new(
        "s",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_f64_values(fc),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        match op {
            "clip" => {
                std::hint::black_box(s.clip(Some(1000.0), Some(50000.0)).unwrap());
            }
            "between" => {
                std::hint::black_box(
                    s.between(
                        &fp_types::Scalar::Float64(1000.0),
                        &fp_types::Scalar::Float64(50000.0),
                        "both",
                    )
                    .unwrap(),
                );
            }
            "cummax" => {
                std::hint::black_box(s.cummax().unwrap());
            }
            "cummin" => {
                std::hint::black_box(s.cummin().unwrap());
            }
            "cumsum" => {
                std::hint::black_box(s.cumsum().unwrap());
            }
            "diff" => {
                std::hint::black_box(s.diff(1).unwrap());
            }
            _ => panic!(),
        };
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("{op} n={n}: best={best}ns");
}
