//! DataFrame row-wise (axis=1) reductions over a K-column f64 frame.
//! Run: bench_axis1 <n> <ncols> <op>
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
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(500_000);
    let ncols: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let op = a.get(3).map(String::as_str).unwrap_or("sum");
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..ncols {
        let name = format!("c{c}");
        cols.insert(
            name.clone(),
            Column::from_f64_values((0..n).map(|i| sm(i, c as u64) as f64 / 1e6).collect()),
        );
        order.push(name);
    }
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let df = DataFrame::new_with_column_order(Index::new(labels), cols, order).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = match op {
            "sum" => df.sum_axis1(),
            "mean" => df.mean_axis1(),
            "min" => df.min_axis1(),
            "max" => df.max_axis1(),
            "std" => df.std_axis1(),
            "var" => df.var_axis1(),
            "median" => df.median_axis1(),
            "skew" => df.skew_axis1(),
            "count" => df.count_axis1(),
            _ => panic!("op"),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("axis1_{op} n={n} ncols={ncols}: best={best}ns");
}
