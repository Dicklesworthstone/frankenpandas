use std::{
    collections::{BTreeMap, HashMap},
    time::Instant,
};

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
    let g: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let op = a.get(3).map(String::as_str).unwrap_or("agg");
    let it: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(6);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_i64_values((0..n).map(|i| (sm(i, 0) as u64 as i64) % g).collect()),
    );
    cols.insert(
        "a".to_string(),
        Column::from_f64_values((0..n).map(|i| sm(i, 1)).collect()),
    );
    cols.insert(
        "b".to_string(),
        Column::from_f64_values((0..n).map(|i| sm(i, 2)).collect()),
    );
    let df =
        DataFrame::new_with_column_order(index, cols, vec!["k".into(), "a".into(), "b".into()])
            .unwrap();
    let mut m = HashMap::new();
    let f1 = a.get(5).map(String::as_str).unwrap_or("sum");
    let f2 = a.get(6).map(String::as_str).unwrap_or("std");
    m.insert("a".to_string(), f1.to_string());
    m.insert("b".to_string(), f2.to_string());
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        let r = match op {
            "agg" => df.groupby(&["k"]).unwrap().agg(&m).unwrap(),
            "sort_index" => df.sort_index(true).unwrap(),
            _ => panic!(),
        };
        std::hint::black_box(r);
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("{op} n={n} g={g}: best={best}ns");
}
