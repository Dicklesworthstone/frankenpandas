//! DataFrameGroupBy.first()/last() with a Utf8 value column @1M.
//! Run: bench_gb_first <n> <gcard> <op>
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
fn contig(n: usize, f: impl Fn(usize) -> String) -> Column {
    let mut bytes = Vec::new();
    let mut offsets = Vec::with_capacity(n + 1);
    offsets.push(0usize);
    for i in 0..n {
        bytes.extend_from_slice(f(i).as_bytes());
        offsets.push(bytes.len());
    }
    Column::from_utf8_contiguous(bytes, offsets)
}
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let n: usize = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let gc: u64 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let op = a.get(3).map(String::as_str).unwrap_or("first");
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        contig(n, |i| format!("g{:04}", sm(i, 0) % gc)),
    );
    cols.insert("v".to_string(), contig(n, |i| format!("v{:08}", sm(i, 1))));
    let df = DataFrame::new_with_column_order(index, cols, vec!["k".into(), "v".into()]).unwrap();
    let mut best = u128::MAX;
    for _ in 0..6 {
        let t = Instant::now();
        match op {
            "first" => {
                black_box(df.groupby(&["k"]).unwrap().first().unwrap());
            }
            "last" => {
                black_box(df.groupby(&["k"]).unwrap().last().unwrap());
            }
            "max" => {
                black_box(df.groupby(&["k"]).unwrap().max().unwrap());
            }
            "min" => {
                black_box(df.groupby(&["k"]).unwrap().min().unwrap());
            }
            "nunique" => {
                black_box(df.groupby(&["k"]).unwrap().nunique().unwrap());
            }
            "count" => {
                black_box(df.groupby(&["k"]).unwrap().count().unwrap());
            }
            "tflist" => {
                black_box(
                    df.groupby(&["k"])
                        .unwrap()
                        .transform_list(&["first"])
                        .unwrap(),
                );
            }
            "tflcount" => {
                black_box(
                    df.groupby(&["k"])
                        .unwrap()
                        .transform_list(&["count"])
                        .unwrap(),
                );
            }
            _ => panic!("op"),
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("gb_{op} n={n} gc={gc}: best={best}ns");
}
