//! No-mock conformance guard for the typed buffer concat lever (br-frankenpandas-tbrtu:
//! concat_series_columns + typed Int64 index concat). Asserts the typed fast path
//! produces EXACTLY the same values and index as the semantic concat, across the
//! homogeneous-Int64/Float64 (typed) and mixed-dtype (Scalar-fallback) cases, for both
//! ignore_index branches. Compiled via `cargo check --tests`; full run batch-pending.

use fp_frame::{Series, concat_series_with_ignore_index};
use fp_index::IndexLabel;
use fp_types::Scalar;

fn i64_series(name: &str, idx: &[i64], vals: &[i64]) -> Series {
    Series::from_values(
        name,
        idx.iter().map(|&x| IndexLabel::Int64(x)).collect(),
        vals.iter().map(|&x| Scalar::Int64(x)).collect(),
    )
    .unwrap()
}

fn vals_i64(s: &Series) -> Vec<i64> {
    s.values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect()
}

fn idx_i64(s: &Series) -> Vec<i64> {
    s.index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect()
}

#[test]
fn concat_int64_ignore_index_true_typed_matches_expected() {
    let a = i64_series("v", &[10, 11, 12], &[1, 2, 3]);
    let b = i64_series("v", &[20, 21], &[4, 5]);
    let out = concat_series_with_ignore_index(&[&a, &b], true).unwrap();
    // ignore_index=True: values concatenated in order, index is a 0..total range.
    assert_eq!(vals_i64(&out), vec![1, 2, 3, 4, 5]);
    assert_eq!(idx_i64(&out), vec![0, 1, 2, 3, 4]);
}

#[test]
fn concat_int64_labeled_typed_matches_expected() {
    let a = i64_series("v", &[10, 11, 12], &[1, 2, 3]);
    let b = i64_series("v", &[20, 21], &[4, 5]);
    let out = concat_series_with_ignore_index(&[&a, &b], false).unwrap();
    // ignore_index=False: both values AND the Int64 index labels are concatenated.
    assert_eq!(vals_i64(&out), vec![1, 2, 3, 4, 5]);
    assert_eq!(idx_i64(&out), vec![10, 11, 12, 20, 21]);
}

#[test]
fn concat_float64_labeled_typed_matches_expected() {
    let mk = |idx: &[i64], vals: &[f64]| {
        Series::from_values(
            "f",
            idx.iter().map(|&x| IndexLabel::Int64(x)).collect(),
            vals.iter().map(|&x| Scalar::Float64(x)).collect(),
        )
        .unwrap()
    };
    let a = mk(&[0, 1], &[1.5, 2.5]);
    let b = mk(&[2, 3], &[3.5, 4.5]);
    let out = concat_series_with_ignore_index(&[&a, &b], false).unwrap();
    assert_eq!(
        out.column().as_f64_slice(),
        Some([1.5, 2.5, 3.5, 4.5].as_slice())
    );
    let got: Vec<f64> = out
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Float64(x) => *x,
            _ => f64::NAN,
        })
        .collect();
    assert_eq!(got, vec![1.5, 2.5, 3.5, 4.5]);
    assert_eq!(idx_i64(&out), vec![0, 1, 2, 3]);
}

#[test]
fn concat_mixed_dtype_falls_back_to_scalar_path() {
    // An Int64 series + a Utf8 series: not homogeneous, so the typed fast path is
    // skipped and the Scalar concat preserves each value verbatim.
    let a = i64_series("x", &[0, 1], &[7, 8]);
    let b = Series::from_values(
        "x",
        vec![IndexLabel::Int64(2), IndexLabel::Int64(3)],
        vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into())],
    )
    .unwrap();
    let out = concat_series_with_ignore_index(&[&a, &b], true).unwrap();
    assert_eq!(out.values().len(), 4);
    assert!(matches!(out.values()[0], Scalar::Int64(7)));
    assert!(matches!(out.values()[3], Scalar::Utf8(ref s) if s == "b"));
}
