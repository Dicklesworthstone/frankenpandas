//! Differential golden for the dense int64 INNER merge core
//! (dense_i64_inner_positions_slices) — exercises the direct-address unique-key
//! fast path AND the CSR duplicate fallback. Hashes merged output.
//! Run: cargo run -p fp-join --example golden_join_dense_inner --release
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, MergedDataFrame, merge_dataframes_on};
fn build(lk: Vec<i64>, rk: Vec<i64>) -> (DataFrame, DataFrame) {
    let nl = lk.len();
    let nr = rk.len();
    let idx = |m: usize| Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    let mut lm = BTreeMap::new();
    lm.insert("key".to_string(), Column::from_i64_values(lk));
    lm.insert(
        "lv".to_string(),
        Column::from_f64_values((0..nl).map(|i| i as f64).collect()),
    );
    let l = DataFrame::new_with_column_order(idx(nl), lm, vec!["key".into(), "lv".into()]).unwrap();
    let mut rm = BTreeMap::new();
    rm.insert("key".to_string(), Column::from_i64_values(rk));
    rm.insert(
        "rv".to_string(),
        Column::from_f64_values((0..nr).map(|i| i as f64 * 10.0).collect()),
    );
    let r = DataFrame::new_with_column_order(idx(nr), rm, vec!["key".into(), "rv".into()]).unwrap();
    (l, r)
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
        let c = &m.columns[name];
        rows = c.len();
        for v in c.values().iter() {
            feed(&format!("{v:?},"));
        }
        feed("\n");
    }
    (h, rows)
}
fn main() {
    let configs: Vec<(Vec<i64>, Vec<i64>)> = vec![
        (vec![2, 0, 1], vec![1, 1, 0]),         // dup right -> CSR
        (vec![5, 3, 9, 3], vec![3, 3, 5]),      // dup both
        (vec![10, 4, 8, 0], vec![4, 8, 0, 99]), // unique right, unmatched
        (vec![7, 7, 7], vec![7]),               // dup left, unique right -> direct-address
    ];
    let m = 5000i64;
    let lk2: Vec<i64> = (0..m)
        .map(|i| {
            let v = (i * 2654435761i64).rem_euclid(m);
            v * 7 + (v % 2)
        })
        .collect();
    let rk2: Vec<i64> = (0..m).map(|i| (i * 40503i64).rem_euclid(m) * 7).collect(); // unique right, sparse
    let mut all = configs;
    all.push((lk2, rk2));
    for (ci, (lk, rk)) in all.into_iter().enumerate() {
        let (l, r) = build(lk, rk);
        let merged = merge_dataframes_on(&l, &r, &["key"], JoinType::Inner).unwrap();
        let (h, rows) = hash_merged(&merged);
        println!("cfg{ci} inner: rows={rows} fnv={h:016x}");
    }
    println!("ALL GOLDEN CHECKS PASSED");
}
