use std::time::Instant;

use fp_frame::Series;
use fp_index::IndexLabel;
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
    let op = a.get(2).map(String::as_str).unwrap_or("replace");
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    let words = [
        "apple",
        "Banana",
        "cherry",
        "DATE",
        "elderberry",
        "Fig",
        "grape",
    ];
    let vals: Vec<Scalar> = (0..n)
        .map(|i| {
            let w = words[(sm(i, 0) % 7) as usize];
            Scalar::Utf8(format!("{w}_{}_{}", sm(i, 1) % 1000, w))
        })
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values("s", labels, vals).unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        match op {
            "replace" => {
                std::hint::black_box(s.str().replace("a", "X").unwrap());
            }
            "slice" => {
                std::hint::black_box(s.str().slice(Some(1), Some(5), None).unwrap());
            }
            "split_get" => {
                std::hint::black_box(s.str().split_get("_", 0).unwrap());
            }
            "strip" => {
                std::hint::black_box(s.str().strip().unwrap());
            }
            _ => panic!(),
        };
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("str_{op} n={n}: best={best}ns");
}
