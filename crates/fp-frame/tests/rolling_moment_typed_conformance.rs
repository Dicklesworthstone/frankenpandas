//! No-mock conformance guard for the typed all-valid rolling skew/kurt fast path
//! (raw &[f64] power-sum recurrence). Must match the generic Scalar path. A
//! trailing NaN makes the series non-all-valid -> as_f64_slice is None -> the
//! Scalar path runs; for every row i < k the rolling window [i-w+1, i] lies
//! entirely in the observed prefix (never the appended NaN at index k), so the
//! two paths must agree bit-for-bit there (present values; missing-equivalent).

use fp_columnar::Column;
use fp_frame::Series;
use fp_index::{Index, IndexLabel};
use fp_types::Scalar;

fn sm(i: usize, s: u64) -> u64 {
    let mut h = (i as u64).wrapping_add(s).wrapping_mul(0x9E3779B97F4A7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    h ^= h >> 31;
    h
}
fn typed_series(vals: &[f64]) -> Series {
    let labels: Vec<IndexLabel> = (0..vals.len() as i64).map(IndexLabel::Int64).collect();
    Series::new(
        "v",
        Index::new(labels),
        Column::from_f64_values(vals.to_vec()),
    )
    .unwrap()
}
fn scalar_series_trailing_nan(vals: &[f64]) -> Series {
    let mut sc: Vec<Scalar> = vals.iter().map(|&v| Scalar::Float64(v)).collect();
    sc.push(Scalar::Float64(f64::NAN));
    let labels: Vec<IndexLabel> = (0..sc.len() as i64).map(IndexLabel::Int64).collect();
    Series::from_values("v", labels, sc).unwrap()
}
fn bits(s: &Scalar) -> u64 {
    if s.is_missing() {
        return u64::MAX;
    }
    match s {
        Scalar::Float64(f) => f.to_bits(),
        other => panic!("unexpected present {other:?}"),
    }
}
fn cases() -> Vec<Vec<f64>> {
    vec![
        (0..600).map(|i| (sm(i, 0) % 100_000) as f64).collect(),
        {
            // runs of constants (exercises the consecutive-equal counter)
            let mut v = vec![3.0; 50];
            v.extend(vec![7.0; 50]);
            v.extend((0..100).map(|i| (sm(i, 1) % 1000) as f64));
            v
        },
        (0..200).map(|i| i as f64 * 0.25 - 25.0).collect(),
    ]
}
#[test]
fn rolling_skew_kurt_typed_matches_scalar_path() {
    for vals in cases() {
        let k = vals.len();
        for &w in &[10usize, 100, 3] {
            for kurt in [false, true] {
                let t = typed_series(&vals);
                let sc = scalar_series_trailing_nan(&vals);
                let (rt, rs) = if kurt {
                    (
                        t.rolling(w, None).kurt().unwrap(),
                        sc.rolling(w, None).kurt().unwrap(),
                    )
                } else {
                    (
                        t.rolling(w, None).skew().unwrap(),
                        sc.rolling(w, None).skew().unwrap(),
                    )
                };
                for i in 0..k {
                    assert_eq!(
                        bits(&rt.values()[i]),
                        bits(&rs.values()[i]),
                        "kurt={kurt} w={w} i={i}"
                    );
                }
            }

            // cov/corr: second axis (perturbation). Typed path needs BOTH all-
            // valid; trailing-NaN forces the Scalar path, prefix must match.
            let other: Vec<f64> = vals
                .iter()
                .enumerate()
                .map(|(j, &v)| v * 0.75 - j as f64)
                .collect();
            for corr in [false, true] {
                let t = typed_series(&vals);
                let to = typed_series(&other);
                let sc = scalar_series_trailing_nan(&vals);
                let so = scalar_series_trailing_nan(&other);
                let (rt, rs) = if corr {
                    (
                        t.rolling(w, None).corr(&to).unwrap(),
                        sc.rolling(w, None).corr(&so).unwrap(),
                    )
                } else {
                    (
                        t.rolling(w, None).cov(&to).unwrap(),
                        sc.rolling(w, None).cov(&so).unwrap(),
                    )
                };
                for i in 0..k {
                    assert_eq!(
                        bits(&rt.values()[i]),
                        bits(&rs.values()[i]),
                        "corr={corr} w={w} i={i}"
                    );
                }
            }
        }
    }
}
