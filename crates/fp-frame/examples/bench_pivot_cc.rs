//! DataFrame.pivot_table head-to-head. 1M rows: int64 index col (~1000 distinct),
//! int64 columns col (~10 distinct), f64 values col. aggfunc sum/mean.
//! Run: cargo run -p fp-frame --example bench_pivot_cc --release -- 1000000 1000 10 20 sum

use std::{collections::BTreeMap, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn best<F: FnMut()>(iters: usize, mut f: F) -> u128 {
    let mut b = u128::MAX;
    for _ in 0..iters {
        let t = Instant::now();
        f();
        let e = t.elapsed().as_nanos();
        if e < b {
            b = e;
        }
    }
    b
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1_000_000);
    let n_idx: i64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
    let n_col: i64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(10);
    let iters: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20);
    let aggfunc = args.get(5).map(String::as_str).unwrap_or("sum");
    let keytype = args.get(6).map(String::as_str).unwrap_or("i64");

    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    // simple deterministic LCG-ish spread
    let idx_key = |i: usize| ((i as i64).wrapping_mul(2654435761) >> 13) % n_idx;
    let col_key = |i: usize| ((i as i64).wrapping_mul(40503) >> 7) % n_col;
    let (idx_vals, col_vals): (Vec<Scalar>, Vec<Scalar>) = if keytype == "str" {
        (
            (0..n)
                .map(|i| Scalar::Utf8(format!("r{:06}", idx_key(i))))
                .collect(),
            (0..n)
                .map(|i| Scalar::Utf8(format!("c{:04}", col_key(i))))
                .collect(),
        )
    } else if keytype == "dt" {
        (
            (0..n)
                .map(|i| Scalar::Datetime64(idx_key(i) * 86_400_000_000_000))
                .collect(),
            (0..n).map(|i| Scalar::Int64(col_key(i))).collect(),
        )
    } else {
        (
            (0..n).map(|i| Scalar::Int64(idx_key(i))).collect(),
            (0..n).map(|i| Scalar::Int64(col_key(i))).collect(),
        )
    };
    cols.insert("idx".to_string(), Column::from_values(idx_vals).unwrap());
    cols.insert("col".to_string(), Column::from_values(col_vals).unwrap());
    cols.insert(
        "val".to_string(),
        Column::from_values(
            (0..n)
                .map(|i| Scalar::Float64((i % 997) as f64 * 1.5))
                .collect(),
        )
        .unwrap(),
    );
    let order = vec!["idx".to_string(), "col".to_string(), "val".to_string()];
    let df = DataFrame::new_with_column_order(index, cols, order).unwrap();

    let t = best(iters, || {
        std::hint::black_box(
            df.pivot_table("val", "idx", "col", aggfunc)
                .expect("pivot_table"),
        );
    });
    println!(
        "pivot_table n={n} n_idx={n_idx} n_col={n_col} agg={aggfunc} key={keytype}: best={t}ns"
    );
}
