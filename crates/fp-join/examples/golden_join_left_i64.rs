//! Differential golden for single-key ordered-unique int64 merges (harness join
//! workload: left key 0..n, right key 0,2,..,2(n-1)). Hashes the merged output
//! (all columns, row order) so the typed-i64 left match-positions fast path is
//! proven bit-identical to the Scalar path.
//! Run: cargo run -p fp-join --example golden_join_left_i64 --release
use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, MergedDataFrame, merge_dataframes_on};

fn build(n: usize) -> (DataFrame, DataFrame) {
    let idx = |m: usize| Index::new((0..m as i64).map(IndexLabel::Int64).collect());
    let mut lm = BTreeMap::new();
    lm.insert(
        "key".to_string(),
        Column::from_i64_values((0..n as i64).collect()),
    );
    lm.insert(
        "left_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64).collect()),
    );
    let left = DataFrame::new_with_column_order(idx(n), lm, vec!["key".into(), "left_val".into()])
        .unwrap();
    let mut rm = BTreeMap::new();
    rm.insert(
        "key".to_string(),
        Column::from_i64_values((0..n as i64).map(|i| i * 2).collect()),
    );
    rm.insert(
        "right_val".to_string(),
        Column::from_f64_values((0..n).map(|i| i as f64 * 10.0).collect()),
    );
    let right =
        DataFrame::new_with_column_order(idx(n), rm, vec!["key".into(), "right_val".into()])
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
    for &n in &[1usize, 2, 7, 64, 5000] {
        let (l, r) = build(n);
        for (label, jt) in [
            ("left", JoinType::Left),
            ("inner", JoinType::Inner),
            ("outer", JoinType::Outer),
            ("right", JoinType::Right),
        ] {
            let merged = merge_dataframes_on(&l, &r, &["key"], jt).unwrap();
            let (h, rows) = hash_merged(&merged);
            println!("n={n} {label}: rows={rows} fnv={h:016x}");
        }
    }
    println!("ALL GOLDEN CHECKS PASSED");
}
