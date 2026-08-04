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
    let g: i64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let w: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let op = a.get(4).map(String::as_str).unwrap_or("sum");
    let it: usize = a.get(5).and_then(|s| s.parse().ok()).unwrap_or(6);
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_i64_values((0..n).map(|i| (sm(i, 0) as i64) % g).collect()),
    );
    cols.insert(
        "v".to_string(),
        Column::from_f64_values((0..n).map(|i| sm(i, 1) as f64).collect()),
    );
    let df = DataFrame::new_with_column_order(index, cols, vec!["k".into(), "v".into()]).unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        let r = match op {
            "sum" => df.groupby(&["k"]).unwrap().rolling(w).sum(),
            "mean" => df.groupby(&["k"]).unwrap().rolling(w).mean(),
            _ => panic!(),
        };
        std::hint::black_box(r.unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("gbroll_{op} n={n} g={g} w={w}: best={best}ns");
}
