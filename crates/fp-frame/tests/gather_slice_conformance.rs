//! No-mock conformance guard for the zero-copy gather/slice levers — take_positions +
//! Index::take + Column/Index::slice across loc_bool (t0y8n), dropna (9i6my), sort_values
//! (7ufhq), head/tail (6wx84), take (3v7o8). Asserts each produces EXACTLY the same
//! values AND carried index labels as the semantic op it replaced. Compiled via
//! `cargo check --tests`; full run batch-pending.

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn s_i64(idx: &[i64], vals: &[i64]) -> Series {
    Series::from_values(
        "v",
        idx.iter().map(|&x| IndexLabel::Int64(x)).collect(),
        vals.iter().map(|&x| Scalar::Int64(x)).collect(),
    )
    .unwrap()
}

fn vals(s: &Series) -> Vec<i64> {
    s.values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect()
}

fn idx(s: &Series) -> Vec<i64> {
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
fn loc_bool_keeps_exact_positions_and_index() {
    let s = s_i64(&[10, 11, 12, 13, 14], &[1, 2, 3, 4, 5]);
    let out = s.loc_bool(&[true, false, true, false, true]).unwrap();
    assert_eq!(vals(&out), vec![1, 3, 5]);
    assert_eq!(idx(&out), vec![10, 12, 14]); // index carried via Index::take
}

#[test]
fn sort_values_carries_index_with_values() {
    let s = s_i64(&[10, 11, 12, 13], &[30, 10, 40, 20]);
    let out = s.sort_values(true).unwrap();
    assert_eq!(vals(&out), vec![10, 20, 30, 40]);
    // The original labels follow their values through the reorder (Index::take).
    assert_eq!(idx(&out), vec![11, 13, 10, 12]);
}

#[test]
fn head_tail_zero_copy_slice_exact() {
    let s = s_i64(&[10, 11, 12, 13, 14], &[1, 2, 3, 4, 5]);
    let h = s.head(2).unwrap();
    assert_eq!(vals(&h), vec![1, 2]);
    assert_eq!(idx(&h), vec![10, 11]);
    let t = s.tail(2).unwrap();
    assert_eq!(vals(&t), vec![4, 5]);
    assert_eq!(idx(&t), vec![13, 14]);
}

#[test]
fn take_gather_exact_including_negative() {
    let s = s_i64(&[10, 11, 12, 13], &[1, 2, 3, 4]);
    let out = s.take(&[3, 0, -1, 1]).unwrap();
    // -1 resolves to the last position (index 3 / value 4).
    assert_eq!(vals(&out), vec![4, 1, 4, 2]);
    assert_eq!(idx(&out), vec![13, 10, 13, 11]);
}

#[test]
fn dropna_keeps_non_missing_with_index() {
    let s = Series::from_values(
        "v",
        (0..4_i64).map(IndexLabel::Int64).collect(),
        vec![
            Scalar::Float64(1.0),
            Scalar::Null(fp_types::NullKind::NaN),
            Scalar::Float64(3.0),
            Scalar::Null(fp_types::NullKind::NaN),
        ],
    )
    .unwrap();
    let out = s.dropna().unwrap();
    let got: Vec<f64> = out
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Float64(x) => *x,
            _ => f64::NAN,
        })
        .collect();
    assert_eq!(got, vec![1.0, 3.0]);
    assert_eq!(idx(&out), vec![0, 2]); // kept indices via Index::take
}
