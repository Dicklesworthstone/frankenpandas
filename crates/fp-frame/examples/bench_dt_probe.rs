//! Probe Series.dt.* over a TYPED (i64-backed) Datetime64 column @1M.
//! Run: bench_dt_probe <n> <op>
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
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let op = a.get(2).map(String::as_str).unwrap_or("year");
    let base = 1_577_836_800_000_000_000i64; // 2020-01-01 ns
    let data: Vec<i64> = (0..n)
        .map(|i| base + (sm(i, 0) % 126_000_000) as i64 * 1_000_000_000)
        .collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let s = Series::new(
        "s",
        Index::new(labels),
        Column::from_datetime64_values(data),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..8 {
        let t = Instant::now();
        let r = match op {
            "year" => s.dt().year(),
            "month" => s.dt().month(),
            "day" => s.dt().day(),
            "hour" => s.dt().hour(),
            "minute" => s.dt().minute(),
            "second" => s.dt().second(),
            "dayofweek" => s.dt().dayofweek(),
            "quarter" => s.dt().quarter(),
            "dayofyear" => s.dt().dayofyear(),
            "is_month_end" => s.dt().is_month_end(),
            "days_in_month" => s.dt().days_in_month(),
            "date" => s.dt().date(),
            "floor" => s.dt().floor("D"),
            "ceil" => s.dt().ceil("h"),
            "round" => s.dt().round("h"),
            "normalize" => s.dt().normalize(),
            "strftime" => s.dt().strftime("%Y-%m-%d"),
            _ => panic!("op"),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("dt_{op} n={n}: best={best}ns");
}
