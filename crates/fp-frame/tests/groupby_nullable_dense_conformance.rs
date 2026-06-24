//! No-mock conformance guard for the nullable-Float64 dense groupby reduction
//! lever (br-frankenpandas-1q4q4 follow-up): a single Int64 key + a Float64 value
//! column WITH missing values now flows through the dense `dense_aggregate_emit`
//! skipna arm instead of the generic `build_groups` path (which was ~0.4x pandas).
//! This test pins the BIT-IDENTICAL semantics with hand-computed expected output:
//! sum skips missing (all-missing group => 0.0), mean/min skip (all-missing => NaN),
//! std is the two-pass mean-centered ddof=1 fold (count<2 => NaN), count = #valid.
//! Int64 keys 0,1,2,3 are first-seen == sorted so group order is unambiguous.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::{NullKind, Scalar};

// keys 0,1,0,2,1,0,3
// vals 2, NaN, 4, NaN, 6, 6, 5  (Float64 with missing)
// group 0 (rows 0,2,5): valid [2,4,6]    sum 12 mean 4 min 2 count 3 std 2.0
// group 1 (rows 1,4):   valid [6]         sum 6  mean 6 min 6 count 1 std NaN
// group 2 (row 3):      valid []          sum 0  mean NaN min NaN count 0 std NaN
// group 3 (row 6):      valid [5]         sum 5  mean 5 min 5 count 1 std NaN
fn fixture() -> DataFrame {
    let keys = [0_i64, 1, 0, 2, 1, 0, 3];
    let n = keys.len();
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let vals = [
        Scalar::Float64(2.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(4.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(6.0),
        Scalar::Float64(6.0),
        Scalar::Float64(5.0),
    ];
    let mut cols = BTreeMap::new();
    cols.insert("k".to_string(), Column::from_i64_values(keys.to_vec()));
    cols.insert("v".to_string(), Column::from_values(vals.to_vec()).unwrap());
    DataFrame::new_with_column_order(index, cols, vec!["k".to_string(), "v".to_string()]).unwrap()
}

fn group_keys(df: &DataFrame) -> Vec<i64> {
    df.index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(x) => *x,
            other => panic!("unexpected key label {other:?}"),
        })
        .collect()
}

fn col_f64(df: &DataFrame, name: &str) -> Vec<f64> {
    df.column(name)
        .expect("column present")
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Float64(x) => *x,
            Scalar::Int64(x) => *x as f64,
            Scalar::Null(_) => f64::NAN,
            other => panic!("unexpected value {other:?}"),
        })
        .collect()
}

fn assert_f64(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "length mismatch: {got:?} vs {want:?}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        if w.is_nan() {
            assert!(g.is_nan(), "idx {i}: expected NaN got {g}");
        } else {
            assert!((g - w).abs() < 1e-12, "idx {i}: expected {w} got {g}");
        }
    }
}

#[test]
fn groupby_nullable_dense_sum() {
    let g = fixture().groupby(&["k"]).unwrap().sum().unwrap();
    assert_eq!(group_keys(&g), vec![0, 1, 2, 3]);
    assert_f64(&col_f64(&g, "v"), &[12.0, 6.0, 0.0, 5.0]);
}

#[test]
fn groupby_nullable_dense_mean() {
    let g = fixture().groupby(&["k"]).unwrap().mean().unwrap();
    assert_eq!(group_keys(&g), vec![0, 1, 2, 3]);
    assert_f64(&col_f64(&g, "v"), &[4.0, 6.0, f64::NAN, 5.0]);
}

#[test]
fn groupby_nullable_dense_min() {
    let g = fixture().groupby(&["k"]).unwrap().min().unwrap();
    assert_eq!(group_keys(&g), vec![0, 1, 2, 3]);
    assert_f64(&col_f64(&g, "v"), &[2.0, 6.0, f64::NAN, 5.0]);
}

#[test]
fn groupby_nullable_dense_count() {
    let g = fixture().groupby(&["k"]).unwrap().count().unwrap();
    assert_eq!(group_keys(&g), vec![0, 1, 2, 3]);
    assert_f64(&col_f64(&g, "v"), &[3.0, 1.0, 0.0, 1.0]);
}

#[test]
fn groupby_nullable_dense_std() {
    let g = fixture().groupby(&["k"]).unwrap().std().unwrap();
    assert_eq!(group_keys(&g), vec![0, 1, 2, 3]);
    assert_f64(&col_f64(&g, "v"), &[2.0, f64::NAN, f64::NAN, f64::NAN]);
}

// group 0 valid [2,4,6], group 1 valid [6], group 2 valid [], group 3 valid [5]
#[test]
fn groupby_nullable_dense_prod() {
    // all-missing group 2 -> empty product 1.0 (pandas min_count=0)
    let g = fixture().groupby(&["k"]).unwrap().prod().unwrap();
    assert_eq!(group_keys(&g), vec![0, 1, 2, 3]);
    assert_f64(&col_f64(&g, "v"), &[48.0, 6.0, 1.0, 5.0]);
}

#[test]
fn groupby_nullable_dense_first() {
    // first NON-missing in ascending row order; group 1 skips its leading NaN
    let g = fixture().groupby(&["k"]).unwrap().first().unwrap();
    assert_eq!(group_keys(&g), vec![0, 1, 2, 3]);
    assert_f64(&col_f64(&g, "v"), &[2.0, 6.0, f64::NAN, 5.0]);
}

#[test]
fn groupby_nullable_dense_last() {
    // last NON-missing in ascending row order
    let g = fixture().groupby(&["k"]).unwrap().last().unwrap();
    assert_eq!(group_keys(&g), vec![0, 1, 2, 3]);
    assert_f64(&col_f64(&g, "v"), &[6.0, 6.0, f64::NAN, 5.0]);
}

#[test]
fn groupby_nullable_dense_median() {
    // median over valid values; all-missing group 2 -> NaN
    let g = fixture().groupby(&["k"]).unwrap().median().unwrap();
    assert_eq!(group_keys(&g), vec![0, 1, 2, 3]);
    assert_f64(&col_f64(&g, "v"), &[4.0, 6.0, f64::NAN, 5.0]);
}
