//! DataFrame.groupby(datetime_col).agg() — grouping by a Datetime64 key @1M.
//! Run: bench_gb_dtkey <n> <gcard> <op>
use std::collections::BTreeMap;

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
    let gc: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let op = a.get(3).map(String::as_str).unwrap_or("sum");
    let base = 1_577_836_800_000_000_000i64;
    let day = 86_400_000_000_000i64;
    // group key: one of gc distinct days (wide ns span, low cardinality)
    let keys: Vec<i64> = (0..n)
        .map(|i| base + (sm(i, 0) % gc) as i64 * day)
        .collect();
    let vals: Vec<f64> = (0..n).map(|i| sm(i, 1) as f64).collect();
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let mut cols = BTreeMap::new();
    cols.insert("k".to_string(), Column::from_datetime64_values(keys));
    cols.insert("v".to_string(), Column::from_f64_values(vals));
    let df =
        DataFrame::new_with_column_order(Index::new(labels), cols, vec!["k".into(), "v".into()])
            .unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = match op {
            "sum" => df.groupby(&["k"]).unwrap().sum(),
            "mean" => df.groupby(&["k"]).unwrap().mean(),
            "count" => df.groupby(&["k"]).unwrap().count(),
            "max" => df.groupby(&["k"]).unwrap().max(),
            _ => panic!("op"),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("gb_dtkey_{op} n={n} gc={gc}: best={best}ns");
}
