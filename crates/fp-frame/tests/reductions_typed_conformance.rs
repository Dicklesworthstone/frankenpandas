//! No-mock conformance guard for the typed Int64 reduction levers (br-frankenpandas-bwgyc
//! sum, 4qs3h max/min, v8jq6 prod): the as_i64_slice fast paths must match the Scalar
//! semantics exactly — including wrapping arithmetic and the empty-series identities
//! (sum->0, prod->1, max/min->Float64(NaN)). Compiled via `cargo check --tests`; full run
//! batch-pending.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn i64_series(vals: &[i64]) -> Series {
    Series::from_values(
        "v",
        (0..vals.len() as i64).map(IndexLabel::Int64).collect(),
        vals.iter().map(|&x| Scalar::Int64(x)).collect(),
    )
    .unwrap()
}

fn empty_i64_series() -> Series {
    Series::new("v", Index::new(vec![]), Column::from_i64_values(vec![])).unwrap()
}

#[test]
fn sum_typed_int64_matches_including_wrapping() {
    assert!(matches!(
        i64_series(&[1, 2, 3]).sum().unwrap(),
        Scalar::Int64(6)
    ));
    // numpy wrap-on-overflow (a52db): MAX + 1 wraps to MIN.
    let w = i64_series(&[i64::MAX, 1]).sum().unwrap();
    assert!(matches!(w, Scalar::Int64(x) if x == i64::MIN));
    // empty Int64 sum -> 0
    assert!(matches!(
        empty_i64_series().sum().unwrap(),
        Scalar::Int64(0)
    ));
}

#[test]
fn max_min_typed_int64_matches() {
    assert!(matches!(
        i64_series(&[3, 1, 2, -5, 4]).max().unwrap(),
        Scalar::Int64(4)
    ));
    assert!(matches!(
        i64_series(&[3, 1, 2, -5, 4]).min().unwrap(),
        Scalar::Int64(-5)
    ));
    // empty -> Float64(NaN)
    assert!(matches!(empty_i64_series().max().unwrap(), Scalar::Float64(x) if x.is_nan()));
    assert!(matches!(empty_i64_series().min().unwrap(), Scalar::Float64(x) if x.is_nan()));
}

#[test]
fn prod_typed_int64_matches_including_wrapping_and_identity() {
    assert!(matches!(
        i64_series(&[2, 3, 4]).prod().unwrap(),
        Scalar::Int64(24)
    ));
    // empty -> 1 (multiplicative identity)
    assert!(matches!(
        empty_i64_series().prod().unwrap(),
        Scalar::Int64(1)
    ));
    // contains zero -> 0
    assert!(matches!(
        i64_series(&[5, 0, 7]).prod().unwrap(),
        Scalar::Int64(0)
    ));
}
