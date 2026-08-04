//! No-mock conformance guard for the typed Series.shift levers (br-frankenpandas-233lo:
//! Float64 path 202cdf50, Int64-valid-fill path 51601b7a). Asserts the typed buffer
//! shift produces EXACTLY the expected values (present + fill/missing positions) and
//! preserves the index, for positive/negative/over-length periods. Compiled via
//! `cargo check --tests`; full run batch-pending.

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn f64_series(vals: &[f64]) -> Series {
    Series::from_values(
        "f",
        (0..vals.len() as i64).map(IndexLabel::Int64).collect(),
        vals.iter().map(|&x| Scalar::Float64(x)).collect(),
    )
    .unwrap()
}

fn i64_series(vals: &[i64]) -> Series {
    Series::from_values(
        "i",
        (0..vals.len() as i64).map(IndexLabel::Int64).collect(),
        vals.iter().map(|&x| Scalar::Int64(x)).collect(),
    )
    .unwrap()
}

fn approx(v: &Scalar, x: f64) -> bool {
    matches!(v, Scalar::Float64(y) if (y - x).abs() < 1e-12)
}

#[test]
fn shift_f64_positive_typed_matches_expected() {
    let s = f64_series(&[1.0, 2.0, 3.0, 4.0]);
    let out = s.shift(1).unwrap();
    assert!(out.values()[0].is_missing(), "vacated head is missing");
    assert!(approx(&out.values()[1], 1.0));
    assert!(approx(&out.values()[2], 2.0));
    assert!(approx(&out.values()[3], 3.0));
    // index preserved
    assert_eq!(out.index().labels().len(), 4);
}

#[test]
fn shift_f64_negative_typed_matches_expected() {
    let s = f64_series(&[1.0, 2.0, 3.0, 4.0]);
    let out = s.shift(-1).unwrap();
    assert!(approx(&out.values()[0], 2.0));
    assert!(approx(&out.values()[1], 3.0));
    assert!(approx(&out.values()[2], 4.0));
    assert!(out.values()[3].is_missing(), "vacated tail is missing");
}

#[test]
fn shift_i64_valid_fill_typed_matches_expected() {
    let s = i64_series(&[10, 20, 30, 40]);
    let out = s.shift_with_fill_value(2, Scalar::Int64(-9)).unwrap();
    let got: Vec<i64> = out
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect();
    assert_eq!(got, vec![-9, -9, 10, 20]);
}

#[test]
fn shift_i64_negative_valid_fill_typed_matches_expected() {
    let s = i64_series(&[10, 20, 30, 40]);
    let out = s.shift_with_fill_value(-1, Scalar::Int64(0)).unwrap();
    let got: Vec<i64> = out
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect();
    assert_eq!(got, vec![20, 30, 40, 0]);
}

#[test]
fn shift_over_length_all_fill() {
    let s = i64_series(&[10, 20, 30]);
    let out = s.shift_with_fill_value(10, Scalar::Int64(7)).unwrap();
    let got: Vec<i64> = out
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(x) => *x,
            _ => i64::MIN,
        })
        .collect();
    assert_eq!(got, vec![7, 7, 7]);
}
