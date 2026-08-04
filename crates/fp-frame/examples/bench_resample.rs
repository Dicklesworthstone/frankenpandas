//! Series.resample(freq).{mean,sum} over a Datetime64-indexed series @1M. bench_resample <n> <freq> <op>
use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

fn sm(i: usize) -> f64 {
    let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    (h % 100_000) as f64
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let freq = a.get(2).map(String::as_str).unwrap_or("D");
    let op = a.get(3).map(String::as_str).unwrap_or("mean");
    let base = 1_577_836_800_000_000_000i64; // 2020-01-01
    let minute = 60_000_000_000i64;
    // minute-spaced timestamps -> ~694 days for 1M rows
    let labels: Vec<IndexLabel> = (0..n)
        .map(|i| IndexLabel::Datetime64(base + i as i64 * minute))
        .collect();
    let s = Series::new(
        "v",
        Index::new(labels),
        Column::from_f64_values((0..n).map(sm).collect()),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = s.resample(freq);
        let res = match op {
            "mean" => r.mean(),
            "sum" => r.sum(),
            _ => panic!("op"),
        };
        std::hint::black_box(res.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("resample_{op}_{freq} n={n}: best={best}ns");
}
