use std::{collections::BTreeMap, hint::black_box, time::Instant};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, merge_dataframes_on};
fn build(n: usize) -> (DataFrame, DataFrame) {
    let idx = |m: usize| Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    // sparse unique keys: shuffle of {0,7,14,...}; left half-matches (offset some by +1)
    let lk: Vec<i64> = (0..n as i64)
        .map(|i| {
            let v = (i * 2654435761i64).rem_euclid(n as i64);
            v * 7 + (v % 2)
        })
        .collect();
    let rk: Vec<i64> = (0..n as i64)
        .map(|i| (i * 40503i64).rem_euclid(n as i64) * 7)
        .collect();
    let mut lm = BTreeMap::new();
    lm.insert("key".to_string(), Column::from_i64_values(lk));
    lm.insert(
        "left_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(idx(n), lm, vec!["key".into(), "left_val".into()])
        .unwrap();
    let mut rm = BTreeMap::new();
    rm.insert("key".to_string(), Column::from_i64_values(rk));
    rm.insert(
        "right_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64 * 10.0).collect()),
    );
    let right =
        DataFrame::new_with_column_order(idx(n), rm, vec!["key".into(), "right_val".into()])
            .unwrap();
    (left, right)
}
fn main() {
    let n = 1_000_000usize;
    let it = 30usize;
    let (l, r) = build(n);
    for _ in 0..3 {
        black_box(
            merge_dataframes_on(&l, &r, &["key"], JoinType::Left)
                .unwrap()
                .columns
                .len(),
        );
    }
    let st = Instant::now();
    let mut k = 0usize;
    for _ in 0..it {
        k ^= black_box(
            merge_dataframes_on(&l, &r, &["key"], JoinType::Left)
                .unwrap()
                .columns
                .len(),
        );
    }
    println!(
        "dense_sparse_left_1M   : {:.3} ms/call (k={k})",
        st.elapsed().as_secs_f64() * 1000.0 / it as f64
    );
}
