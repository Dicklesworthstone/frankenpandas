//! No-mock conformance guard for the typed all-valid EWM var/std fast path
//! (ewm_var_all_observed over a raw &[f64]). It must be bit-identical to the
//! generic Scalar path. Trick: append ONE NaN so the series is NOT all-valid ->
//! as_f64_slice returns None -> the Scalar path runs; its OBSERVED prefix (rows
//! 0..k) processes exactly the same values the typed path does on the all-valid
//! series, so the two must agree bit-for-bit over that prefix.

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
fn scalar_series_with_trailing_nan(vals: &[f64]) -> Series {
    let mut sc: Vec<Scalar> = vals.iter().map(|&v| Scalar::Float64(v)).collect();
    sc.push(Scalar::Float64(f64::NAN)); // forces non-all-valid -> Scalar path
    let labels: Vec<IndexLabel> = (0..sc.len() as i64).map(IndexLabel::Int64).collect();
    Series::from_values("v", labels, sc).unwrap()
}

// Conformance invariant: a row is either MISSING in both paths, or PRESENT with
// a bit-equal f64. A missing EWM-var slot is is_missing()==true whether the
// column materializes it as Null (explicit Scalar path) or as a validity-masked
// Float64(NaN) (typed from_f64_values path) — both render as pandas NaN — so map
// every missing repr to the same sentinel and bit-compare only present values.
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
        (0..500).map(|i| (sm(i, 0) % 100_000) as f64).collect(),
        vec![5.0; 300], // constant run (ewm_mean==x guard)
        (0..200).map(|i| i as f64 * 0.5 - 50.0).collect(), // monotone, negatives
        vec![1.0, 1.0, 2.0, 2.0, 3.0], // tiny, ties
    ]
}

#[test]
fn ewm_var_typed_matches_scalar_path() {
    for vals in cases() {
        let k = vals.len();
        for &span in &[5.0_f64, 20.0, 2.0] {
            let vt = typed_series(&vals).ewm(Some(span), None).var().unwrap();
            let vs = scalar_series_with_trailing_nan(&vals)
                .ewm(Some(span), None)
                .var()
                .unwrap();
            for i in 0..k {
                assert_eq!(
                    bits(&vt.values()[i]),
                    bits(&vs.values()[i]),
                    "var i={i} span={span}"
                );
            }
            let st = typed_series(&vals).ewm(Some(span), None).std().unwrap();
            let ss = scalar_series_with_trailing_nan(&vals)
                .ewm(Some(span), None)
                .std()
                .unwrap();
            for i in 0..k {
                assert_eq!(
                    bits(&st.values()[i]),
                    bits(&ss.values()[i]),
                    "std i={i} span={span}"
                );
            }

            // cov/corr: second series (a perturbation of the first). Typed path
            // fires only when BOTH are all-valid; the trailing-NaN versions force
            // the Scalar path, whose observed prefix must match bit-for-bit.
            let other: Vec<f64> = vals
                .iter()
                .enumerate()
                .map(|(j, &v)| v * 1.5 + j as f64)
                .collect();
            let ct = typed_series(&vals)
                .ewm(Some(span), None)
                .cov(&typed_series(&other))
                .unwrap();
            let cs = scalar_series_with_trailing_nan(&vals)
                .ewm(Some(span), None)
                .cov(&scalar_series_with_trailing_nan(&other))
                .unwrap();
            for i in 0..k {
                assert_eq!(
                    bits(&ct.values()[i]),
                    bits(&cs.values()[i]),
                    "cov i={i} span={span}"
                );
            }
            let rt = typed_series(&vals)
                .ewm(Some(span), None)
                .corr(&typed_series(&other))
                .unwrap();
            let rs = scalar_series_with_trailing_nan(&vals)
                .ewm(Some(span), None)
                .corr(&scalar_series_with_trailing_nan(&other))
                .unwrap();
            for i in 0..k {
                assert_eq!(
                    bits(&rt.values()[i]),
                    bits(&rs.values()[i]),
                    "corr i={i} span={span}"
                );
            }
        }
    }
}
