use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::{DataFrame, Series};
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
    let op = a.get(3).map(String::as_str).unwrap_or("nth");
    let it: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(6);
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    let index = Index::new(labels.clone());
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_i64_values((0..n).map(|i| (sm(i, 0) as u64 as i64) % g).collect()),
    );
    cols.insert(
        "a".to_string(),
        Column::from_f64_values((0..n).map(|i| sm(i, 1)).collect()),
    );
    let df = DataFrame::new_with_column_order(index, cols, vec!["k".into(), "a".into()]).unwrap();
    let keys = Series::new(
        "k".to_string(),
        Index::new(labels.clone()),
        Column::from_i64_values((0..n).map(|i| (sm(i, 0) as u64 as i64) % g).collect()),
    )
    .unwrap();
    let vs = Series::new(
        "a".to_string(),
        Index::new(labels),
        Column::from_f64_values((0..n).map(|i| sm(i, 1)).collect()),
    )
    .unwrap();
    let mut best = u128::MAX;
    for _ in 0..it {
        let t = Instant::now();
        match op {
            "nth" => {
                std::hint::black_box(df.groupby(&["k"]).unwrap().nth(0).unwrap());
            }
            "head" => {
                std::hint::black_box(df.groupby(&["k"]).unwrap().head(5).unwrap());
            }
            "sgb_nth" => {
                std::hint::black_box(vs.groupby(&keys).unwrap().nth(0).unwrap());
            }
            "sgb_head" => {
                std::hint::black_box(vs.groupby(&keys).unwrap().head(5).unwrap());
            }
            "sgb_tf_std" => {
                std::hint::black_box(vs.groupby(&keys).unwrap().transform("std").unwrap());
            }
            other => {
                eprintln!("unknown op '{other}'");
                std::process::exit(2);
            }
        };
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("{op} n={n} g={g}: best={best}ns");
}
