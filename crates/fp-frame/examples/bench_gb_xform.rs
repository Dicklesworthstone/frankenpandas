//! SeriesGroupBy.{rank,cummin,cumprod,cumcount,ffill,bfill} @1M. bench_gb_xform <n> <g> <op>
use std::time::Instant;

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};

fn sm(i: usize) -> u64 {
    let mut h = (i as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let g: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let op = a.get(3).map(String::as_str).unwrap_or("rank");
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let keys: Vec<i64> = (0..n).map(|i| (sm(i) % g as u64) as i64).collect();
    let vals: Vec<f64> = (0..n).map(|i| (sm(i + 7) % 100_000) as f64).collect();
    let value = Series::new(
        "v",
        Index::new(labels.clone()),
        Column::from_f64_values(vals),
    )
    .unwrap();
    let key = Series::new("k", Index::new(labels), Column::from_i64_values(keys)).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
        let gb = value.groupby(&key).unwrap();
        let r = match op {
            "tmean" => gb.transform("mean"),
            "tsum" => gb.transform("sum"),
            "rank" => gb.rank("average", true, "keep"),
            "cummin" => gb.cummin(),
            "cumprod" => gb.cumprod(),
            "cumcount" => gb.cumcount(),
            "ffill" => gb.ffill(None),
            "bfill" => gb.bfill(None),
            _ => panic!("op"),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("gbx_{op} n={n} g={g}: best={best}ns");
}
