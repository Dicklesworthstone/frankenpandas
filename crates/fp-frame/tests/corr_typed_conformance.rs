//! No-mock conformance guard for the typed both-Int64 cov_components path
//! (br-frankenpandas-3rrcz). Asserts corr/cov over Int64 columns match the Float64
//! path exactly (cross-dtype equality) and known values. Compiled via
//! `cargo check --tests`; full run batch-pending.

use fp_frame::Series;
use fp_index::IndexLabel;
use fp_types::Scalar;

fn i64_series(vals: &[i64]) -> Series {
    Series::from_values(
        "v",
        (0..vals.len() as i64).map(IndexLabel::Int64).collect(),
        vals.iter().map(|&x| Scalar::Int64(x)).collect(),
    )
    .unwrap()
}

fn f64_series(vals: &[f64]) -> Series {
    Series::from_values(
        "v",
        (0..vals.len() as i64).map(IndexLabel::Int64).collect(),
        vals.iter().map(|&x| Scalar::Float64(x)).collect(),
    )
    .unwrap()
}

#[test]
fn corr_perfectly_correlated_int64() {
    // y = 2x => correlation 1.0
    let x = i64_series(&[1, 2, 3, 4]);
    let y = i64_series(&[2, 4, 6, 8]);
    assert!((x.corr(&y).unwrap() - 1.0).abs() < 1e-12);
}

#[test]
fn cov_self_equals_variance_int64() {
    // cov(x, x) == var(x); var([1,2,3]) (ddof=1) = 1.0
    let x = i64_series(&[1, 2, 3]);
    assert!((x.cov(&x).unwrap() - 1.0).abs() < 1e-12);
}

#[test]
fn corr_cov_int64_match_float64() {
    let xi = i64_series(&[3, 1, 4, 1, 5, 9, 2, 6]);
    let yi = i64_series(&[2, 7, 1, 8, 2, 8, 1, 8]);
    let xf = f64_series(&[3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0]);
    let yf = f64_series(&[2.0, 7.0, 1.0, 8.0, 2.0, 8.0, 1.0, 8.0]);
    // typed Int64 cov_components path must equal the typed Float64 path.
    assert!((xi.corr(&yi).unwrap() - xf.corr(&yf).unwrap()).abs() < 1e-12);
    assert!((xi.cov(&yi).unwrap() - xf.cov(&yf).unwrap()).abs() < 1e-12);
}
