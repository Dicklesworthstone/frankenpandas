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
    let op = a.get(2).map(String::as_str).unwrap_or("year");
    let it: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);
    // datetimes around 2020-2024, ns
    let base = 1_577_836_800_000_000_000_u64; // 2020-01-01
    let vals: Vec<Scalar> = (0..n)
        .map(|i| Scalar::Datetime64((base + (sm(i, 0) % (126_000_000)) * 1_000_000_000) as i64))
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::from_values("s", labels, vals).unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        match op {
            "year" => {
                std::hint::black_box(s.dt().year().unwrap());
            }
            "month" => {
                std::hint::black_box(s.dt().month().unwrap());
            }
            "day" => {
                std::hint::black_box(s.dt().day().unwrap());
            }
            "dayofweek" => {
                std::hint::black_box(s.dt().dayofweek().unwrap());
            }
            "strftime" => {
                std::hint::black_box(s.dt().strftime("%Y-%m-%d").unwrap());
            }
            _ => panic!(),
        };
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("dt_{op} n={n}: best={best}ns");
}
