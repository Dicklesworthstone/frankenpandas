//! No-mock conformance guard for the nullable-Float64 typed Series reduction
//! fast paths: `Series.sum()`/`mean()`/`var()`/`std()` over a Float64 column WITH
//! missing values now fold the typed `(data, validity)` buffer (skipping missing
//! slots) instead of materializing a `Vec<Scalar>` and matching each element.
//! Bit-identical to the Scalar-loop reference: missing values are skipped, sum is
//! a 0.0-seeded left-fold in row order, var is the two-pass mean-centered ddof=1
//! fold. Also guards the all-valid path (unchanged).

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;
use fp_types::{NullKind, Scalar};

fn series(vals: Vec<Scalar>) -> Series {
    let n = vals.len();
    Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_values(vals).unwrap(),
    )
    .unwrap()
}

fn f(x: f64) -> Scalar {
    Scalar::Float64(x)
}
fn nan() -> Scalar {
    Scalar::Null(NullKind::NaN)
}

fn as_f64(s: Scalar) -> f64 {
    match s {
        Scalar::Float64(x) => x,
        Scalar::Int64(x) => x as f64,
        Scalar::Null(_) => f64::NAN,
        other => panic!("unexpected {other:?}"),
    }
}

// valid [2,4,6]: sum 12, count 3, mean 4, var (4+0+4)/2 = 4, std 2
#[test]
fn series_nullable_sum() {
    let s = series(vec![f(2.0), nan(), f(4.0), nan(), f(6.0)]);
    assert!((as_f64(s.sum().unwrap()) - 12.0).abs() < 1e-12);
}

#[test]
fn series_nullable_mean() {
    let s = series(vec![f(2.0), nan(), f(4.0), nan(), f(6.0)]);
    assert!((as_f64(s.mean().unwrap()) - 4.0).abs() < 1e-12);
}

#[test]
fn series_nullable_var() {
    let s = series(vec![f(2.0), nan(), f(4.0), nan(), f(6.0)]);
    assert!((as_f64(s.var().unwrap()) - 4.0).abs() < 1e-12);
}

#[test]
fn series_nullable_std() {
    let s = series(vec![f(2.0), nan(), f(4.0), nan(), f(6.0)]);
    assert!((as_f64(s.std().unwrap()) - 2.0).abs() < 1e-12);
}

// all-missing: sum 0.0 (0.0-seeded fold), mean NaN (count 0), var/std NaN (count<2)
#[test]
fn series_all_missing() {
    let s = series(vec![nan(), nan()]);
    assert_eq!(as_f64(s.sum().unwrap()), 0.0);
    assert!(as_f64(s.mean().unwrap()).is_nan());
    assert!(as_f64(s.var().unwrap()).is_nan());
    assert!(as_f64(s.std().unwrap()).is_nan());
}

// all-valid regression guard (unchanged path): [1,2,3,4] sum 10, mean 2.5,
// var (2.25+0.25+0.25+2.25)/3 = 5/3, std sqrt(5/3)
#[test]
fn series_all_valid_unchanged() {
    let s = series(vec![f(1.0), f(2.0), f(3.0), f(4.0)]);
    assert!((as_f64(s.sum().unwrap()) - 10.0).abs() < 1e-12);
    assert!((as_f64(s.mean().unwrap()) - 2.5).abs() < 1e-12);
    assert!((as_f64(s.var().unwrap()) - 5.0 / 3.0).abs() < 1e-12);
    assert!((as_f64(s.std().unwrap()) - (5.0_f64 / 3.0).sqrt()).abs() < 1e-12);
}
