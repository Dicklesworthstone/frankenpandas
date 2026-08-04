//! DataFrame.idxmax()/idxmin() axis=0 over f64 columns @1M x 10.
use std::{collections::BTreeMap, hint::black_box, time::Instant};

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
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let k: usize = 10;
    let op = a.get(2).map(String::as_str).unwrap_or("idxmax");
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    for c in 0..k {
        let name = format!("c{c}");
        cols.insert(
            name.clone(),
            Column::from_f64_values(
                (0..n)
                    .map(|i| (sm(i, c as u64) % 1_000_000) as f64)
                    .collect(),
            ),
        );
        order.push(name);
    }
    let df = DataFrame::new_with_column_order(index, cols, order).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
        match op {
            "idxmax" => {
                black_box(df.idxmax().unwrap());
            }
            "idxmin" => {
                black_box(df.idxmin().unwrap());
            }
            _ => panic!(),
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("df_{op} n={n}: best={best}ns");
}
