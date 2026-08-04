//! df.groupby([k1,k2]).{sum,mean,count,max} @1M. bench_gb_multikey <n> <g1> <g2> <op>
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
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
    let g1: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let g2: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
    let op = a.get(4).map(String::as_str).unwrap_or("sum");
    let mut cols = BTreeMap::new();
    cols.insert(
        "k1".to_string(),
        Column::from_i64_values((0..n).map(|x| (sm(x, 0) % g1 as u64) as i64).collect()),
    );
    cols.insert(
        "k2".to_string(),
        Column::from_i64_values((0..n).map(|x| (sm(x, 1) % g2 as u64) as i64).collect()),
    );
    cols.insert(
        "v".to_string(),
        Column::from_f64_values((0..n).map(|x| (sm(x, 2) % 100_000) as f64).collect()),
    );
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let df = DataFrame::new_with_column_order(
        Index::new(labels),
        cols,
        vec!["k1".into(), "k2".into(), "v".into()],
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
        let gb = df.groupby(&["k1", "k2"]).unwrap();
        let r = match op {
            "sum" => gb.sum(),
            "mean" => gb.mean(),
            "count" => gb.count(),
            "max" => gb.max(),
            _ => panic!("op"),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("gbmk_{op} n={n} g1={g1} g2={g2}: best={best}ns");
}
