//! DataFrame.rolling median/skew across k cols, shuffled. Run: -- 1000000 4 median 100
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
fn sm(i: usize, s: u64) -> f64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    (h >> 11) as f64 * 1e-6
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let k: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);
    let op = a.get(3).map(String::as_str).unwrap_or("median");
    let w: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(100);
    let it: usize = a.get(5).and_then(|s| s.parse().ok()).unwrap_or(5);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    let mut order = vec![];
    for c in 0..k {
        let nm = format!("c{c}");
        cols.insert(
            nm.clone(),
            Column::from_f64_values((0..n).map(|i| sm(i, c as u64 * 13)).collect()),
        );
        order.push(nm);
    }
    let df = DataFrame::new_with_column_order(index, cols, order).unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        let r = match op {
            "median" => df.rolling(w, None).median().unwrap(),
            "skew" => df.rolling(w, None).skew().unwrap(),
            _ => panic!(),
        };
        std::hint::black_box(r);
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("df_roll_{op} n={n} k={k} w={w}: best={best}ns");
}
