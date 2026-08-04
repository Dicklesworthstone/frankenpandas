//! df.melt(id, value_vars) reshape. bench_melt <rows> <ncols>
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
    let rows: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let ncols: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(10);
    let mut cols = BTreeMap::new();
    let mut order = vec!["id".to_string()];
    cols.insert(
        "id".to_string(),
        Column::from_i64_values((0..rows).map(|i| i as i64).collect()),
    );
    let value_names: Vec<String> = (0..ncols).map(|c| format!("v{c}")).collect();
    for (c, name) in value_names.iter().enumerate() {
        cols.insert(
            name.clone(),
            Column::from_f64_values((0..rows).map(|i| sm(i, c as u64) as f64).collect()),
        );
        order.push(name.clone());
    }
    let labels: Vec<IndexLabel> = (0..rows as i64).map(IndexLabel::Int64).collect();
    let df = DataFrame::new_with_column_order(Index::new(labels), cols, order).unwrap();
    let vrefs: Vec<&str> = value_names.iter().map(String::as_str).collect();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
        std::hint::black_box(df.melt(&["id"], &vrefs, None, None).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!(
        "melt rows={rows} ncols={ncols} (->{}): best={best}ns",
        rows * ncols
    );
}
