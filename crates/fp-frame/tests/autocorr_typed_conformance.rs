//! No-mock conformance guard for the typed all-valid-Float64 `Series::autocorr`
//! fast path: an `as_f64_slice` column computes lag-k autocorrelation via two
//! linear passes over the raw f64 buffer instead of materializing `values()`
//! (Vec<Scalar>) + per-pair x/y Vecs. Bit-identical to the Scalar path:
//! cov/sqrt(var_x*var_y) with the `< f64::EPSILON => NaN` guard.

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

// Independent oracle: the exact formula the production path implements.
fn oracle_autocorr(data: &[f64], lag: usize) -> f64 {
    if data.len() <= lag {
        return f64::NAN;
    }
    let n = data.len() - lag;
    if n < 2 {
        return f64::NAN;
    }
    let nf = n as f64;
    let mean_x = (0..n).map(|i| data[i]).sum::<f64>() / nf;
    let mean_y = (0..n).map(|i| data[i + lag]).sum::<f64>() / nf;
    let (mut cov, mut vx, mut vy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let dx = data[i] - mean_x;
        let dy = data[i + lag] - mean_y;
        cov += dx * dy;
        vx += dx * dx;
        vy += dy * dy;
    }
    let denom = (vx * vy).sqrt();
    if denom < f64::EPSILON {
        f64::NAN
    } else {
        cov / denom
    }
}

#[test]
fn autocorr_lag1_matches_oracle() {
    let data: Vec<f64> = (0..50).map(|i| ((i * 37 + 11) % 17) as f64 + 0.5).collect();
    let got = series(data.clone()).autocorr(1).unwrap();
    let want = oracle_autocorr(&data, 1);
    assert!((got - want).abs() < 1e-12, "got {got} want {want}");
}

#[test]
fn autocorr_lag5_matches_oracle() {
    let data: Vec<f64> = (0..200).map(|i| ((i * 911 + 7) % 101) as f64).collect();
    let got = series(data.clone()).autocorr(5).unwrap();
    let want = oracle_autocorr(&data, 5);
    assert!((got - want).abs() < 1e-12, "got {got} want {want}");
}

// perfectly linear -> autocorr(1) == 1.0
#[test]
fn autocorr_linear_is_one() {
    let data: Vec<f64> = (0..100).map(|i| i as f64).collect();
    let got = series(data).autocorr(1).unwrap();
    assert!((got - 1.0).abs() < 1e-9, "got {got}");
}

// constant series -> var 0 -> denom < EPSILON -> NaN
#[test]
fn autocorr_constant_is_nan() {
    let data = vec![3.0_f64; 100];
    assert!(series(data).autocorr(1).unwrap().is_nan());
}

// lag >= len -> NaN; n<2 -> NaN
#[test]
fn autocorr_short_is_nan() {
    assert!(series(vec![1.0, 2.0, 3.0]).autocorr(3).unwrap().is_nan());
    assert!(series(vec![1.0, 2.0, 3.0]).autocorr(2).unwrap().is_nan());
}
