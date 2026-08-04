//! Multi-key groupby on TWO Utf8 keys: df.groupby([k1,k2]).agg() @1M.
//! Run: bench_gb2_utf8 <n> <g> <op>
use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;

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
    // Contiguous Utf8 key columns (as read_csv / str-ops produce) so the dense
    // factorize path is exercised — from_values(Vec<Scalar::Utf8>) is Scalar-backed
    // and would bail as_utf8_contiguous.
    let contig = |f: &dyn Fn(usize) -> String| -> Column {
        let mut bytes = Vec::new();
        let mut offsets = Vec::with_capacity(n + 1);
        offsets.push(0usize);
        for i in 0..n {
            bytes.extend_from_slice(f(i).as_bytes());
            offsets.push(bytes.len());
        }
        Column::from_utf8_contiguous(bytes, offsets)
    };
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    cols.insert(
        "k1".to_string(),
        contig(&|i| format!("g{:04}", sm(i, 0) % g)),
    );
    cols.insert(
        "k2".to_string(),
        contig(&|i| format!("h{:04}", sm(i, 1) % g)),
    );
    let _ = Scalar::Int64(0);
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
            "nunique" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().nunique().unwrap());
            }
            "idxmax" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().idxmax().unwrap());
            }
            "all" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().all().unwrap());
            }
            "any" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().any().unwrap());
            }
            "size" => {
                std::hint::black_box(df.groupby(&["k1", "k2"]).unwrap().size().unwrap());
            }
            _ => panic!("op"),
        }
        let e = t.elapsed().as_nanos();
        if e < best {
            best = e;
        }
    }
    println!("gb2utf8_{op} n={n} g={g}: best={best}ns");
}
