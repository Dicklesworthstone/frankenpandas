//! Differential golden for the multi-key (composite) int64 INNER merge
//! (dense_packed_int64_inner_positions -> dense_i64_inner_positions_slices).
//! Exercises the direct-address unique-packed-key fast path + CSR dup fallback.
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, MergedDataFrame, merge_dataframes_on_with};
fn build(lk1: Vec<i64>, lk2: Vec<i64>, rk1: Vec<i64>, rk2: Vec<i64>) -> (DataFrame, DataFrame) {
    let nl = lk1.len();
    let nr = rk1.len();
    let idx = |m: usize| Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    let mut lm = BTreeMap::new();
    lm.insert("a".to_string(), Column::from_i64_values(lk1));
    lm.insert("b".to_string(), Column::from_i64_values(lk2));
    lm.insert(
        "lv".to_string(),
        Column::from_f64_values((0..nl).map(|i| i as f64).collect()),
    );
    let l =
        DataFrame::new_with_column_order(idx(nl), lm, vec!["a".into(), "b".into(), "lv".into()])
            .unwrap();
    let mut rm = BTreeMap::new();
    rm.insert("a".to_string(), Column::from_i64_values(rk1));
    rm.insert("b".to_string(), Column::from_i64_values(rk2));
    rm.insert(
        "rv".to_string(),
        Column::from_f64_values((0..nr).map(|i| i as f64 * 10.0).collect()),
    );
    let r =
        DataFrame::new_with_column_order(idx(nr), rm, vec!["a".into(), "b".into(), "rv".into()])
            .unwrap();
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
    // configs: (lk1,lk2,rk1,rk2). unique composite -> direct-address; dup -> CSR.
    type KeyColumns = (Vec<i64>, Vec<i64>, Vec<i64>, Vec<i64>);
    let cfgs: Vec<KeyColumns> = vec![
        (vec![0, 1, 2], vec![5, 6, 7], vec![1, 2, 9], vec![6, 7, 0]), // unique both, partial
        (vec![1, 1, 2], vec![3, 3, 4], vec![1, 2], vec![3, 4]),       // dup left composite -> CSR
        (vec![0, 1], vec![0, 0], vec![0, 1, 0], vec![0, 0, 0]),       // dup right composite -> CSR
    ];
    let m = 4000i64;
    let lk1: Vec<i64> = (0..m).map(|i| (i * 2654435761i64).rem_euclid(m)).collect();
    let lk2: Vec<i64> = (0..m).map(|i| (i * 40503i64).rem_euclid(7)).collect();
    let rk1: Vec<i64> = (0..m).map(|i| (i * 7i64).rem_euclid(m)).collect();
    let rk2: Vec<i64> = (0..m).map(|i| (i * 13i64).rem_euclid(7)).collect();
    let mut all = cfgs;
    all.push((lk1, lk2, rk1, rk2));
    for (ci, (a, b, c, d)) in all.into_iter().enumerate() {
        let (l, r) = build(a, b, c, d);
        let merged =
            merge_dataframes_on_with(&l, &r, &["a", "b"], &["a", "b"], JoinType::Inner).unwrap();
        let (h, rows) = hash_merged(&merged);
        println!("cfg{ci} mkinner: rows={rows} fnv={h:016x}");
    }
    println!("ALL GOLDEN CHECKS PASSED");
}
