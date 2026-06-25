use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let op = a.get(1).map(String::as_str).unwrap_or("std");
    let n: usize = 1_000_000;
    let k: usize = 8;
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..k {
        let name = format!("c{c}");
        let data: Vec<f64> = (0..n).map(|i| (sm(i, c as u64 + 1) % 100_000) as f64).collect();
        cols.insert(name.clone(), Column::from_f64_values(data));
        order.push(name);
    }
    let df = DataFrame::new_with_column_order(index, cols, order).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        let r = match op {
            "sum" => df.sum_axis1(),
            "mean" => df.mean_axis1(),
            "std" => df.std_axis1(),
            "var" => df.var_axis1(),
            "skew" => df.skew_axis1(),
            "max" => df.max_axis1(),
            _ => panic!(),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("probedf_{op} n={n} k={k}: best={best}ns");
}
