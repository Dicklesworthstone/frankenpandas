//! No-mock conformance guard for the typed all-Utf8 counting-sort in
//! `sort_outer_join_rows` (outer-merge output ordering). pandas outer merge sorts
//! the result by the join key (lexicographically, stable within a key). This pins
//! that the factorize+counting-sort path produces the SAME order as the previous
//! comparison sort, matching pandas (verified independently against pandas 2.2.3).

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

fn col_str(d: &MergedDataFrame, name: &str) -> Vec<String> {
    d.columns
        .get(name)
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Utf8(s) => s.clone(),
            Scalar::Null(_) => "<NA>".to_string(),
            other => panic!("unexpected {other:?}"),
        })
        .collect()
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

// pandas: key [a,b,b,c,z], lv [2,1,3,4,nan], rv [10,20,20,nan,30]
#[test]
fn outer_merge_sorted_by_key_stable_within_key() {
    let left = df(&["b", "a", "b", "c"], "lv", &[1.0, 2.0, 3.0, 4.0]);
    let right = df(&["a", "b", "z"], "rv", &[10.0, 20.0, 30.0]);
    let m = merge_dataframes_on(&left, &right, &["key"], JoinType::Outer).unwrap();
    assert_eq!(col_str(&m, "key"), vec!["a", "b", "b", "c", "z"]);
    assert_f64(&col_f64(&m, "lv"), &[2.0, 1.0, 3.0, 4.0, f64::NAN]);
    assert_f64(&col_f64(&m, "rv"), &[10.0, 20.0, 20.0, f64::NAN, 30.0]);
}

// single distinct key (d==1) and all-distinct keys exercise the counting-sort edges
#[test]
fn outer_merge_single_key() {
    let left = df(&["k", "k"], "lv", &[1.0, 2.0]);
    let right = df(&["k"], "rv", &[9.0]);
    let m = merge_dataframes_on(&left, &right, &["key"], JoinType::Outer).unwrap();
    assert_eq!(col_str(&m, "key"), vec!["k", "k"]);
    assert_f64(&col_f64(&m, "lv"), &[1.0, 2.0]);
}

#[test]
fn outer_merge_all_distinct_sorted() {
    let left = df(&["c", "a"], "lv", &[1.0, 2.0]);
    let right = df(&["b", "d"], "rv", &[3.0, 4.0]);
    let m = merge_dataframes_on(&left, &right, &["key"], JoinType::Outer).unwrap();
    assert_eq!(col_str(&m, "key"), vec!["a", "b", "c", "d"]);
}
