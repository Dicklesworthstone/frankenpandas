//! No-mock conformance guard for the typed Int64 Index<->Column levers
//! (br-frankenpandas-bp6k7 reset_index, p9omo set_index). Asserts the typed fast paths
//! move an all-Int64 index <-> column with EXACTLY the right labels/values, and that a
//! reset_index -> set_index round-trip is identity. Compiled via `cargo check --tests`;
//! full run batch-pending.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn df_i64(idx: &[i64], a: &[i64]) -> DataFrame {
    let index = Index::new(idx.iter().map(|&x| IndexLabel::Int64(x)).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_values(a.iter().map(|&x| Scalar::Int64(x)).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(index, cols, vec!["a".to_string()]).unwrap()
}

fn col_i64(df: &DataFrame, name: &str) -> Vec<i64> {
    df.column(name)
        .expect("column present")
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect()
}

fn idx_i64(df: &DataFrame) -> Vec<i64> {
    df.index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect()
}

#[test]
fn reset_index_typed_moves_int64_index_to_column() {
    let df = df_i64(&[100, 101, 102], &[1, 2, 3]);
    let out = df.reset_index(false).unwrap();
    // The former (unnamed) Int64 index becomes the "index" column, verbatim.
    assert_eq!(col_i64(&out, "index"), vec![100, 101, 102]);
    // The data column is preserved.
    assert_eq!(col_i64(&out, "a"), vec![1, 2, 3]);
    // The new index is a default 0..n range.
    assert_eq!(idx_i64(&out), vec![0, 1, 2]);
}

#[test]
fn set_index_typed_moves_int64_column_to_index() {
    let df = df_i64(&[100, 101, 102], &[1, 2, 3]);
    let out = df.set_index("a", true).unwrap();
    // Column "a" becomes the Int64 index, verbatim.
    assert_eq!(idx_i64(&out), vec![1, 2, 3]);
}

#[test]
fn reset_then_set_index_round_trip_is_identity() {
    let df = df_i64(&[100, 101, 102], &[1, 2, 3]);
    let reset = df.reset_index(false).unwrap();
    // Put the former index ("index" column) back as the index.
    let restored = reset.set_index("index", true).unwrap();
    assert_eq!(idx_i64(&restored), vec![100, 101, 102]);
    assert_eq!(col_i64(&restored, "a"), vec![1, 2, 3]);
}
