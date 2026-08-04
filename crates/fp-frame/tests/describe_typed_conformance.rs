//! No-mock conformance guard for the typed describe floats-collection lever
//! (br-frankenpandas-igqxd). Asserts describe over an Int64 column yields the same
//! summary-statistic values as the equivalent Float64 column (cross-dtype equality) and
//! known values. Compiled via `cargo check --tests`; full run batch-pending.

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

fn stat_f64s(s: &Series) -> Vec<f64> {
    s.values()
        .iter()
        .map(|v| match v {
            Scalar::Float64(x) => *x,
            Scalar::Int64(x) => *x as f64,
            _ => f64::NAN,
        })
        .collect()
}

#[test]
fn describe_int64_matches_float64() {
    let di = i64_series(&[2, 4, 4, 4, 5, 5, 7, 9]).describe().unwrap();
    let df = f64_series(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0])
        .describe()
        .unwrap();
    let vi = stat_f64s(&di);
    let vf = stat_f64s(&df);
    assert_eq!(vi.len(), vf.len());
    for (a, b) in vi.iter().zip(vf.iter()) {
        assert!((a - b).abs() < 1e-9, "describe stat mismatch: {a} vs {b}");
    }
}

#[test]
fn describe_int64_known_count_min_max() {
    // describe order: count, mean, std, min, 25%, 50%, 75%, max
    let d = i64_series(&[1, 2, 3, 4, 5]).describe().unwrap();
    let labels: Vec<String> = d
        .index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Utf8(s) => s.clone(),
            other => format!("{other:?}"),
        })
        .collect();
    let vals = stat_f64s(&d);
    let get = |name: &str| -> f64 {
        labels
            .iter()
            .position(|l| l == name)
            .map(|i| vals[i])
            .unwrap_or(f64::NAN)
    };
    assert!((get("count") - 5.0).abs() < 1e-12);
    assert!((get("mean") - 3.0).abs() < 1e-12);
    assert!((get("min") - 1.0).abs() < 1e-12);
    assert!((get("max") - 5.0).abs() < 1e-12);
}
