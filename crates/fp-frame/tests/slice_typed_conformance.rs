//! No-mock conformance guard for the zero-copy contiguous-slice lever in Series.iloc_slice
//! (br-frankenpandas-qynot: Index::slice + Column::slice). Asserts the typed slice yields
//! EXACTLY the expected contiguous values + carried index for start/stop, open-ended,
//! negative, and out-of-range bounds. Compiled via `cargo check --tests`; full run
//! batch-pending.

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
fn iloc_slice_interior_range() {
    let s = s_i64(&[10, 11, 12, 13, 14], &[1, 2, 3, 4, 5]);
    let out = s.iloc_slice(Some(1), Some(4)).unwrap();
    assert_eq!(vals(&out), vec![2, 3, 4]);
    assert_eq!(idx(&out), vec![11, 12, 13]);
}

#[test]
fn iloc_slice_open_ended() {
    let s = s_i64(&[10, 11, 12, 13, 14], &[1, 2, 3, 4, 5]);
    assert_eq!(vals(&s.iloc_slice(Some(2), None).unwrap()), vec![3, 4, 5]);
    assert_eq!(vals(&s.iloc_slice(None, Some(2)).unwrap()), vec![1, 2]);
    assert_eq!(
        vals(&s.iloc_slice(None, None).unwrap()),
        vec![1, 2, 3, 4, 5]
    );
}

#[test]
fn iloc_slice_negative_and_oob() {
    let s = s_i64(&[10, 11, 12, 13, 14], &[1, 2, 3, 4, 5]);
    // negative start resolves from the end
    assert_eq!(vals(&s.iloc_slice(Some(-2), None).unwrap()), vec![4, 5]);
    // stop past the end clamps
    assert_eq!(vals(&s.iloc_slice(Some(3), Some(99)).unwrap()), vec![4, 5]);
    // empty range
    assert!(vals(&s.iloc_slice(Some(3), Some(3)).unwrap()).is_empty());
}
