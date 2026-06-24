//! No-mock conformance guard for the nullable-Float64 dense SeriesGroupBy fold
//! lever: `Series.groupby(int64_key).sum()/mean()/min()/max()` over a Float64
//! value column WITH missing values now flows through `dense_group_fold`'s skipna
//! branch instead of the slow generic `agg_numeric` build_groups path. Pins the
//! BIT-IDENTICAL semantics with hand-computed expected output, matching
//! `agg_numeric`'s reference exactly: missing values are skipped (`is_missing()`),
//! and an ALL-MISSING group emits `Null(NaN)` for EVERY func (sum included — this
//! is the SeriesGroupBy `nums.is_empty() -> Null(NaN)` arm, distinct from
//! DataFrameGroupBy where all-missing sum is 0.0). Int64 keys 0,1,2,3 are
//! first-seen == sorted so group order is unambiguous.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::{NullKind, Scalar};

// keys 0,1,0,2,1,0,3 ; vals 2,NaN,4,NaN,6,6,5
// group 0 (rows 0,2,5): valid [2,4,6]
// group 1 (rows 1,4):   valid [6]
// group 2 (row 3):      valid []   -> Null(NaN) for every func
// group 3 (row 6):      valid [5]
fn key_series() -> Series {
    Series::new(
        "k",
        Index::new_known_unique_int64_unit_range(0, 7),
        Column::from_i64_values(vec![0, 1, 0, 2, 1, 0, 3]),
    )
    .unwrap()
}

fn val_series() -> Series {
    let vals = vec![
        Scalar::Float64(2.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(4.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(6.0),
        Scalar::Float64(6.0),
        Scalar::Float64(5.0),
    ];
    Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, 7),
        Column::from_values(vals).unwrap(),
    )
    .unwrap()
}

fn group_keys(s: &Series) -> Vec<i64> {
    s.index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(x) => *x,
            other => panic!("unexpected key label {other:?}"),
        })
        .collect()
}

fn vals_f64(s: &Series) -> Vec<f64> {
    s.column()
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
    assert_eq!(got.len(), want.len(), "len: {got:?} vs {want:?}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        if w.is_nan() {
            assert!(g.is_nan(), "idx {i}: expected NaN got {g}");
        } else {
            assert!((g - w).abs() < 1e-12, "idx {i}: expected {w} got {g}");
        }
    }
}

#[test]
fn sgb_nullable_dense_sum() {
    let v = val_series();
    let r = v.groupby(&key_series()).unwrap().sum().unwrap();
    assert_eq!(group_keys(&r), vec![0, 1, 2, 3]);
    assert_f64(&vals_f64(&r), &[12.0, 6.0, f64::NAN, 5.0]);
}

#[test]
fn sgb_nullable_dense_mean() {
    let v = val_series();
    let r = v.groupby(&key_series()).unwrap().mean().unwrap();
    assert_eq!(group_keys(&r), vec![0, 1, 2, 3]);
    assert_f64(&vals_f64(&r), &[4.0, 6.0, f64::NAN, 5.0]);
}

#[test]
fn sgb_nullable_dense_min() {
    let v = val_series();
    let r = v.groupby(&key_series()).unwrap().min().unwrap();
    assert_eq!(group_keys(&r), vec![0, 1, 2, 3]);
    assert_f64(&vals_f64(&r), &[2.0, 6.0, f64::NAN, 5.0]);
}

#[test]
fn sgb_nullable_dense_max() {
    let v = val_series();
    let r = v.groupby(&key_series()).unwrap().max().unwrap();
    assert_eq!(group_keys(&r), vec![0, 1, 2, 3]);
    assert_f64(&vals_f64(&r), &[6.0, 6.0, f64::NAN, 5.0]);
}
