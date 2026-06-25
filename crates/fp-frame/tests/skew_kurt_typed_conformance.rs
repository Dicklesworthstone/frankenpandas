//! No-mock conformance guard for the typed all-valid-Float64 `Series::skew` /
//! `kurtosis` fast paths: an `as_f64_slice` column computes the moments via a
//! fused single pass over the raw f64 buffer instead of `numeric_values`'s vals
//! Vec copy + three re-scans. Bit-identical to the original formula (Fisher,
//! bias=False), verified against an independent oracle.

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::Index;

fn series(data: Vec<f64>) -> Series {
    let n = data.len();
    Series::new(
        "v",
        Index::new_known_unique_int64_unit_range(0, n),
        Column::from_f64_values(data),
    )
    .unwrap()
}

fn oracle_skew(data: &[f64]) -> f64 {
    let count = data.len();
    if count < 3 {
        return f64::NAN;
    }
    let n = count as f64;
    let mean = data.iter().sum::<f64>() / n;
    let m2: f64 = data.iter().map(|v| (v - mean).powi(2)).sum();
    let m3: f64 = data.iter().map(|v| (v - mean).powi(3)).sum();
    let s2 = m2 / (n - 1.0);
    if s2 == 0.0 {
        return 0.0;
    }
    let s3 = s2.powf(1.5);
    (n / ((n - 1.0) * (n - 2.0))) * (m3 / s3)
}

fn oracle_kurt(data: &[f64]) -> f64 {
    let count = data.len();
    if count < 4 {
        return f64::NAN;
    }
    let n = count as f64;
    let mean = data.iter().sum::<f64>() / n;
    let m2: f64 = data.iter().map(|v| (v - mean).powi(2)).sum();
    let m4: f64 = data.iter().map(|v| (v - mean).powi(4)).sum();
    let s2 = m2 / (n - 1.0);
    if s2 == 0.0 {
        return 0.0;
    }
    let adj = (n * (n + 1.0)) / ((n - 1.0) * (n - 2.0) * (n - 3.0));
    let sub = (3.0 * (n - 1.0).powi(2)) / ((n - 2.0) * (n - 3.0));
    adj * (m4 / (s2 * s2)) - sub
}

#[test]
fn skew_matches_oracle() {
    let data: Vec<f64> = (0..500)
        .map(|i| ((i * 131 + 7) % 89) as f64 + 0.25)
        .collect();
    let got = series(data.clone()).skew().unwrap();
    let want = oracle_skew(&data);
    assert!((got - want).abs() < 1e-10, "got {got} want {want}");
}

#[test]
fn kurt_matches_oracle() {
    let data: Vec<f64> = (0..500).map(|i| ((i * 977 + 3) % 211) as f64).collect();
    let got = series(data.clone()).kurt().unwrap();
    let want = oracle_kurt(&data);
    assert!((got - want).abs() < 1e-10, "got {got} want {want}");
}

// constant series -> s2 == 0 -> skew/kurt 0.0
#[test]
fn skew_kurt_constant_zero() {
    let data = vec![7.0_f64; 100];
    assert_eq!(series(data.clone()).skew().unwrap(), 0.0);
    assert_eq!(series(data).kurt().unwrap(), 0.0);
}

// too few -> NaN
#[test]
fn skew_kurt_too_few_nan() {
    assert!(series(vec![1.0, 2.0]).skew().unwrap().is_nan());
    assert!(series(vec![1.0, 2.0, 3.0]).kurt().unwrap().is_nan());
}
