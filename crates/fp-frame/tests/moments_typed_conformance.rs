//! No-mock conformance guard for the typed numeric-moment foundations
//! (br-frankenpandas-0xdfx: numeric_moments for var/std/sem, numeric_values for
//! skew/kurt). Asserts the typed Int64 path yields IDENTICAL results to the typed
//! Float64 path (both must equal the Scalar fold) and matches known values. Compiled
//! via `cargo check --tests`; full run batch-pending.

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

fn scalar_f64(s: Scalar) -> f64 {
    match s {
        Scalar::Float64(x) => x,
        Scalar::Int64(x) => x as f64,
        _ => f64::NAN,
    }
}

#[test]
fn var_std_typed_int64_known_values() {
    let s = i64_series(&[1, 2, 3, 4, 5]);
    // sample variance (ddof=1): sum((x-3)^2)/4 = 10/4 = 2.5
    assert!((scalar_f64(s.var().unwrap()) - 2.5).abs() < 1e-12);
    assert!((scalar_f64(s.std().unwrap()) - 2.5_f64.sqrt()).abs() < 1e-12);
}

#[test]
fn moments_typed_int64_match_float64() {
    let vi = i64_series(&[2, 4, 4, 4, 5, 5, 7, 9]);
    let vf = f64_series(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
    // typed Int64 path must equal typed Float64 path (both == Scalar fold).
    assert!((scalar_f64(vi.var().unwrap()) - scalar_f64(vf.var().unwrap())).abs() < 1e-12);
    assert!((scalar_f64(vi.std().unwrap()) - scalar_f64(vf.std().unwrap())).abs() < 1e-12);
    assert!((vi.sem().unwrap() - vf.sem().unwrap()).abs() < 1e-12);
    assert!((vi.skew().unwrap() - vf.skew().unwrap()).abs() < 1e-12);
    assert!((vi.kurtosis().unwrap() - vf.kurtosis().unwrap()).abs() < 1e-12);
}

#[test]
fn skew_symmetric_is_zero_int64() {
    // a symmetric distribution has zero skew
    let s = i64_series(&[1, 2, 3, 4, 5]);
    assert!(s.skew().unwrap().abs() < 1e-9);
}
