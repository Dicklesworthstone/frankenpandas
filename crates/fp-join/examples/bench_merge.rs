//! Inner merge on Int64 vs Utf8 key @1M left x K right.
//! bench_merge <n> <k> <keytype> [sorted_dup]
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes};

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}

fn sorted_dup_key(row: usize, rows: usize, cardinality: usize) -> i64 {
    if rows == 0 || cardinality == 0 {
        return 0;
    }
    let key = (row as u128 * cardinality as u128) / rows as u128;
    key.min((cardinality - 1) as u128) as i64
}

fn frame(keys_i64: Vec<i64>, vals: Vec<f64>, vname: &str, utf8: bool) -> DataFrame {
    let n = keys_i64.len();
    let key_col = if utf8 {
        let mut bytes = Vec::new();
        let mut offs = vec![0usize];
        for &k in &keys_i64 {
            bytes.extend_from_slice(format!("k{k:07}").as_bytes());
            offs.push(bytes.len());
        }
        Column::from_utf8_contiguous(bytes, offs)
    } else {
        Column::from_i64_values(keys_i64)
    };
    let mut cols = BTreeMap::new();
    cols.insert("key".to_string(), key_col);
    cols.insert(vname.to_string(), Column::from_f64_values(vals));
    let labels: Vec<IndexLabel> = (0..n as i64).map(IndexLabel::Int64).collect();
    DataFrame::new_with_column_order(Index::new(labels), cols, vec!["key".into(), vname.into()])
        .unwrap()
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let k: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100_000);
    let utf8 = a.get(3).map(|s| s == "utf8").unwrap_or(false);
    let sorted_dup = a.get(4).map(|s| s == "sorted_dup").unwrap_or(false);
    let left = frame(
        (0..n)
            .map(|i| {
                if sorted_dup {
                    sorted_dup_key(i, n, k)
                } else {
                    (sm(i, 0) % k as u64) as i64
                }
            })
            .collect(),
        (0..n).map(|i| sm(i, 2) as f64).collect(),
        "lv",
        utf8,
    );
    // right: K unique keys, one row each
    let right = frame(
        (0..k).map(|i| i as i64).collect(),
        (0..k).map(|i| sm(i, 3) as f64).collect(),
        "rv",
        utf8,
    );
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = std::time::Instant::now();
        std::hint::black_box(merge_dataframes(&left, &right, "key", JoinType::Inner).unwrap());
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    let key_name = if utf8 { "utf8" } else { "i64" };
    let mode = if sorted_dup { "_sorted_dup" } else { "" };
    println!("merge_inner_{key_name}{mode} n={n} k={k}: best={best}ns");
}
