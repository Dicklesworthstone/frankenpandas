//! No-mock conformance guard for the typed Int64 row-wise reduction path
//! (reduce_rows_int64, behind DataFrame::sum_axis1 / mean_axis1). Asserts per-row
//! sum/mean over an all-Int64 frame match expected values. Compiled via
//! `cargo check --tests`; full run batch-pending.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn df_i64(a: &[i64], b: &[i64]) -> DataFrame {
    let n = a.len();
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "a".to_string(),
        Column::from_values(a.iter().map(|&x| Scalar::Int64(x)).collect()).unwrap(),
    );
    cols.insert(
        "b".to_string(),
        Column::from_values(b.iter().map(|&x| Scalar::Int64(x)).collect()).unwrap(),
    );
    DataFrame::new_with_column_order(index, cols, vec!["a".to_string(), "b".to_string()]).unwrap()
}

fn f64s(s: &fp_frame::Series) -> Vec<f64> {
    s.values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x as f64,
            Scalar::Float64(x) => *x,
            _ => f64::NAN,
        })
        .collect()
}

#[test]
fn sum_axis1_typed_int64_per_row() {
    let df = df_i64(&[1, 2, 3], &[10, 20, 30]);
    let s = df.sum_axis1().unwrap();
    assert_eq!(f64s(&s), vec![11.0, 22.0, 33.0]);
}

#[test]
fn mean_axis1_typed_int64_per_row() {
    let df = df_i64(&[1, 2, 3], &[10, 20, 30]);
    let s = df.mean_axis1().unwrap();
    let got = f64s(&s);
    let expected = [5.5, 11.0, 16.5];
    assert_eq!(got.len(), expected.len());
    for (a, b) in got.iter().zip(expected.iter()) {
        assert!((a - b).abs() < 1e-12, "mean_axis1 mismatch: {a} vs {b}");
    }
}
