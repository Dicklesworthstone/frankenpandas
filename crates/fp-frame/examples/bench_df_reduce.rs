use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
fn sm(i: usize, s: u64) -> f64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    (h >> 11) as f64
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let k: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let op = a.get(3).map(String::as_str).unwrap_or("std");
    let it: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(8);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    let mut order = vec![];
    for c in 0..k {
        let nm = format!("c{c}");
        cols.insert(
            nm.clone(),
            Column::from_f64_values((0..n).map(|i| sm(i, c as u64)).collect()),
        );
        order.push(nm);
    }
    let df = DataFrame::new_with_column_order(index, cols, order).unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        match op {
            "std" => {
                std::hint::black_box(df.std().unwrap());
            }
            "skew" => {
                std::hint::black_box(df.skew().unwrap());
            }
            "sem" => {
                std::hint::black_box(df.sem().unwrap());
            }
            "sum" => {
                std::hint::black_box(df.sum().unwrap());
            }
            _ => panic!(),
        };
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("df_{op} n={n} k={k}: best={best}ns");
}
