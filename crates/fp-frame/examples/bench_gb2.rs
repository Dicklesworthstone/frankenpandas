use std::{collections::BTreeMap, time::Instant};

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
    let g: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let op = a.get(3).map(String::as_str).unwrap_or("sum");
    let it: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(6);
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    cols.insert(
        "k1".to_string(),
        Column::from_i64_values((0..n).map(|i| (sm(i, 0) % g) as i64).collect()),
    );
    cols.insert(
        "k2".to_string(),
        Column::from_i64_values((0..n).map(|i| (sm(i, 1) % g) as i64).collect()),
    );
    cols.insert(
        "v".to_string(),
        Column::from_f64_values((0..n).map(|i| sm(i, 2) as f64).collect()),
    );
    let df =
        DataFrame::new_with_column_order(index, cols, vec!["k1".into(), "k2".into(), "v".into()])
            .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        match op {
            "sum" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().sum().unwrap());
            }
            "mean" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().mean().unwrap());
            }
            "count" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().count().unwrap());
            }
            "min" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().min().unwrap());
            }
            "max" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().max().unwrap());
            }
            "median" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().median().unwrap());
            }
            "aggsum" => {
                let mut m = std::collections::HashMap::new();
                m.insert("v".to_string(), "sum".to_string());
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().agg(&m).unwrap());
            }
            _ => panic!(),
        };
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("gb2_{op} n={n} g={g}x{g}: best={best}ns");
}
