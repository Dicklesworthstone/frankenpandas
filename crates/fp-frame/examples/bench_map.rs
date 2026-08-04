use std::time::Instant;

use fp_columnar::Column;
use fp_frame::{Series, cut, qcut};
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("map");
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    let nd: u64 = 1000;
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    // NON-NEGATIVE keys 0..nd (typed Int64 column), matching python `% nd`.
    let ic: Vec<i64> = (0..n).map(|i| (sm(i, 0) % nd) as i64).collect();
    let fc: Vec<f64> = (0..n).map(|i| sm(i, 1) as f64).collect();
    let si = Series::new("s", Index::new(labels.clone()), Column::from_i64_values(ic)).unwrap();
    let sf = Series::new("s", Index::new(labels), Column::from_f64_values(fc)).unwrap();
    let mapping: Vec<(Scalar, Scalar)> = (0..nd as i64)
        .map(|k| (Scalar::Int64(k), Scalar::Int64(k * 7)))
        .collect();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        match op {
            "map" => {
                std::hint::black_box(si.map(&mapping).unwrap());
            }
            "replace" => {
                std::hint::black_box(si.replace(&mapping).unwrap());
            }
            "cut" => {
                std::hint::black_box(cut(&sf, 10).unwrap());
            }
            "qcut" => {
                std::hint::black_box(qcut(&sf, 10).unwrap());
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
