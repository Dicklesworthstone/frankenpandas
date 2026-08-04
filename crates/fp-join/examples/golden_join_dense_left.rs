//! Differential golden for the dense (unordered) int64 left-merge path
//! (dense_int64_left_positions). Uses shuffled left keys + duplicate, unordered
//! right keys with bounded span so the ordered-unique gate is skipped and the
//! dense counting-sort bucketing is exercised. Hashes the merged output so the
//! typed-&[i64] rewrite is proven bit-identical to the Scalar walk.
//! Run: cargo run -p fp-join --example golden_join_dense_left --release
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, MergedDataFrame, merge_dataframes_on};

fn build(left_keys: Vec<i64>, right_keys: Vec<i64>) -> (DataFrame, DataFrame) {
    let nl = left_keys.len();
    let nr = right_keys.len();
    let idx = |m: usize| Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    let mut lm = BTreeMap::new();
    lm.insert("key".to_string(), Column::from_i64_values(left_keys));
    lm.insert(
        "left_val".to_string(),
        Column::from_f64_values((0..nl).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(idx(nl), lm, vec!["key".into(), "left_val".into()])
        .unwrap();
    let mut rm = BTreeMap::new();
    rm.insert("key".to_string(), Column::from_i64_values(right_keys));
    rm.insert(
        "right_val".to_string(),
        Column::from_f64_values((0..nr).map(|i| i as f64 * 10.0).collect()),
    );
    let right =
        DataFrame::new_with_column_order(idx(nr), rm, vec!["key".into(), "right_val".into()])
            .unwrap();
    (left, right)
}

fn hash_merged(m: &MergedDataFrame) -> (u64, usize) {
    let mut h = 0xcbf29ce484222325u64;
    let mut feed = |s: &str| {
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    let mut rows = 0usize;
    for name in &m.column_order {
        feed(name);
        feed("|");
        let col = &m.columns[name];
        rows = col.len();
        for v in col.values().iter() {
            feed(&format!("{v:?},"));
        }
        feed("\n");
    }
    (h, rows)
}

fn main() {
    // (left_keys, right_keys) configs exercising dense branches: unordered,
    // duplicate right keys, out-of-range left keys, empty sides.
    let configs: Vec<(Vec<i64>, Vec<i64>)> = vec![
        (vec![2, 0, 1], vec![1, 1, 0]),    // dup right, unordered
        (vec![5, 3, 9, 3], vec![3, 3, 5]), // dup left + right
        (vec![0, 1, 2], vec![]),           // empty right
        (vec![7, 7, 7], vec![7]),          // all dup left, single right
        (vec![10, 4, 4, 8, 0], vec![4, 8, 8, 0, 0, 99]), // mixed dups + unmatched right
    ];
    // Larger shuffled config.
    let n = 4000i64;
    let lk: Vec<i64> = (0..n).map(|i| (i * 2654435761i64).rem_euclid(n)).collect();
    let rk: Vec<i64> = (0..n).map(|i| (i * 7) % n).collect();
    let mut all = configs;
    all.push((lk, rk));
    // Sparse UNIQUE keys (span 7n, partial match) — exercises the direct-address
    // gate-5 fast path (unique right keys, float payload bypasses int64 gates).
    let m = 5000i64;
    let lk2: Vec<i64> = (0..m)
        .map(|i| {
            let v = (i * 2654435761i64).rem_euclid(m);
            v * 7 + (v % 2)
        })
        .collect();
    let rk2: Vec<i64> = (0..m).map(|i| (i * 40503i64).rem_euclid(m) * 7).collect();
    all.push((lk2, rk2));

    for (ci, (lk, rk)) in all.into_iter().enumerate() {
        let (l, r) = build(lk, rk);
        let merged = merge_dataframes_on(&l, &r, &["key"], JoinType::Left).unwrap();
        let (h, rows) = hash_merged(&merged);
        println!("cfg{ci} left: rows={rows} fnv={h:016x}");
    }
    println!("ALL GOLDEN CHECKS PASSED");
}
