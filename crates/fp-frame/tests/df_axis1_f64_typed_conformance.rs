//! No-mock conformance guard for the typed-f64-output change to
//! `reduce_rows_func_f64` (behind DataFrame std/var/sem/skew/kurt axis=1): the
//! per-row reduced values are now ingested via `from_f64_values` (typed) instead
//! of a `Vec<Scalar::Float64>` + `from_values`. Asserts per-row values against an
//! independent oracle and that a zero-variance row behaves identically.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::Index;
use fp_types::Scalar;

// 3 rows x 4 cols (column-major):
// row0 = [1,2,3,4], row1 = [10,10,10,10], row2 = [5,5,5,6]
fn fixture() -> DataFrame {
    let cols_data = [
        ("c0", vec![1.0, 10.0, 5.0]),
        ("c1", vec![2.0, 10.0, 5.0]),
        ("c2", vec![3.0, 10.0, 5.0]),
        ("c3", vec![4.0, 10.0, 6.0]),
    ];
    let n = 3;
    let index = Index::new_known_unique_int64_unit_range(0, n);
    let mut cols = BTreeMap::new();
    let mut order = Vec::new();
    for (name, d) in cols_data {
        cols.insert(name.to_string(), Column::from_f64_values(d));
        order.push(name.to_string());
    }
    DataFrame::new_with_column_order(index, cols, order).unwrap()
}

fn rows() -> Vec<Vec<f64>> {
    vec![
        vec![1.0, 2.0, 3.0, 4.0],
        vec![10.0, 10.0, 10.0, 10.0],
        vec![5.0, 5.0, 5.0, 6.0],
    ]
}

fn oracle_var(r: &[f64]) -> f64 {
    let n = r.len() as f64;
    let mean = r.iter().sum::<f64>() / n;
    r.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0)
}

fn vals(s: &fp_frame::Series) -> Vec<f64> {
    s.column()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Float64(x) => *x,
            Scalar::Int64(x) => *x as f64,
            Scalar::Null(_) => f64::NAN,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn var_axis1_typed_matches_oracle() {
    let got = vals(&fixture().var_axis1().unwrap());
    let want: Vec<f64> = rows().iter().map(|r| oracle_var(r)).collect();
    assert_eq!(got.len(), 3);
    for (g, w) in got.iter().zip(&want) {
        assert!((g - w).abs() < 1e-12, "var_axis1 {g} vs {w}");
    }
}

#[test]
fn std_axis1_typed_matches_oracle() {
    let got = vals(&fixture().std_axis1().unwrap());
    let want: Vec<f64> = rows().iter().map(|r| oracle_var(r).sqrt()).collect();
    for (g, w) in got.iter().zip(&want) {
        assert!((g - w).abs() < 1e-12, "std_axis1 {g} vs {w}");
    }
}

// zero-variance row (row1) must produce std/var 0.0 (not NaN/missing) — values
// pass through the typed output unchanged.
#[test]
fn axis1_zero_variance_row() {
    let v = vals(&fixture().var_axis1().unwrap());
    assert_eq!(v[1], 0.0);
    let s = vals(&fixture().std_axis1().unwrap());
    assert_eq!(s[1], 0.0);
}
