//! Series.expanding().{std,var,skew,kurt,cov,corr} @1M. bench_expanding <n> <op>
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
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("skew");
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::new(
        "v",
        Index::new(labels),
        Column::from_f64_values((0..n).map(|i| (sm(i, 0) % 100_000) as f64).collect()),
    )
    .unwrap();
    let other = s.clone();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let e = s.expanding(None);
        let r = match op {
            "sum" => e.sum(),
            "mean" => e.mean(),
            "std" => e.std(),
            "var" => e.var(),
            "skew" => e.skew(),
            "kurt" => e.kurt(),
            "sem" => e.sem(),
            "cov" => e.cov(&other),
            "corr" => e.corr(&other),
            _ => panic!("op"),
        };
        std::hint::black_box(r.unwrap());
        let el = t.elapsed().as_nanos();
        if el < best {
            best = el;
        }
    }
    println!("expanding_{op} n={n}: best={best}ns");
}
