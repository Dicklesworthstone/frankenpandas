//! No-mock conformance guard for the composite (Utf8) RIGHT-merge path after it
//! was switched from a left_map (O(left) hash build) to probing the small
//! right_map with a single left pass + per-right bucketing. Pins the exact output
//! ordering pandas produces (right-row order outer, left-position order inner),
//! including duplicate right keys, multiple left matches, and unmatched right rows.
//! Verified independently against pandas 2.2.3.

use std::collections::BTreeMap;

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_join::{JoinType, MergedDataFrame, merge_dataframes_on};
use fp_types::Scalar;

fn df(keys: &[&str], vname: &str, vals: &[f64]) -> DataFrame {
    let n = keys.len();
    let index = Index::new((0..n as i64).map(IndexLabel::Int64).collect());
    let mut cols = BTreeMap::new();
    cols.insert(
        "key".to_string(),
        Column::from_values(
            keys.iter()
                .map(|s| Scalar::Utf8((*s).to_string()))
                .collect(),
        )
        .unwrap(),
    );
    cols.insert(vname.to_string(), Column::from_f64_values(vals.to_vec()));
    DataFrame::new_with_column_order(index, cols, vec!["key".to_string(), vname.to_string()])
        .unwrap()
}

fn col_f64(d: &MergedDataFrame, name: &str) -> Vec<f64> {
    d.columns
        .get(name)
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Float64(x) => *x,
            Scalar::Null(_) => f64::NAN,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

fn assert_f64(got: &[f64], want: &[f64]) {
    assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        if w.is_nan() {
            assert!(g.is_nan(), "expected NaN got {g}");
        } else {
            assert!((g - w).abs() < 1e-12, "{g} vs {w}");
        }
    }
}

// pandas merge(how='right'):
//   left  key [A,B,A,C] lv [1,2,3,4]
//   right key [A,B,B,D] rv [10,20,30,40]
//   -> lv [1,3,2,2,nan], rv [10,10,20,30,40]
// Exercises: A has TWO left matches (0,2) emitted in left order; B is a duplicate
// right key (rows 1,2) each taking the single left match; D is unmatched.
#[test]
fn right_merge_dup_keys_multi_match_unmatched() {
    let left = df(&["A", "B", "A", "C"], "lv", &[1.0, 2.0, 3.0, 4.0]);
    let right = df(&["A", "B", "B", "D"], "rv", &[10.0, 20.0, 30.0, 40.0]);
    let m = merge_dataframes_on(&left, &right, &["key"], JoinType::Right).unwrap();
    assert_f64(&col_f64(&m, "lv"), &[1.0, 3.0, 2.0, 2.0, f64::NAN]);
    assert_f64(&col_f64(&m, "rv"), &[10.0, 10.0, 20.0, 30.0, 40.0]);
}

// All right rows unmatched -> each emits one (NaN, rv) row in right order.
#[test]
fn right_merge_all_unmatched() {
    let left = df(&["X"], "lv", &[1.0]);
    let right = df(&["A", "B"], "rv", &[10.0, 20.0]);
    let m = merge_dataframes_on(&left, &right, &["key"], JoinType::Right).unwrap();
    assert_f64(&col_f64(&m, "lv"), &[f64::NAN, f64::NAN]);
    assert_f64(&col_f64(&m, "rv"), &[10.0, 20.0]);
}
