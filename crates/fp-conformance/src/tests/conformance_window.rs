//! Window parity-matrix conformance suite (frankenpandas-trc6).
//!
//! Per /testing-conformance-harnesses Pattern 1, these fixtures compare
//! FrankenPandas against live upstream pandas for rolling, expanding, EWM, and
//! DataFrame rolling edge cases: empty inputs, single rows, missing-heavy
//! values, duplicate index labels, mixed numeric dtypes, min_periods, and
//! boundary-sized windows.

use serde_json::{Map, Value};

use super::{
    CaseStatus, HarnessConfig, HarnessError, OracleMode, PacketFixture, ResolvedExpected,
    SuiteOptions, capture_live_oracle_expected,
};

fn strict_config() -> HarnessConfig {
    HarnessConfig::default_paths()
}

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Series(_) | ResolvedExpected::Frame(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping window conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_window_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("window oracle") {
        return;
    }

    let report = super::run_differential_fixture(
        &cfg,
        &fixture,
        &SuiteOptions {
            packet_filter: None,
            oracle_mode: OracleMode::LiveLegacyPandas,
        },
    )
    .expect("differential report");

    assert_eq!(
        report.status,
        CaseStatus::Pass,
        "pandas window parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

fn i(value: i64) -> Value {
    serde_json::json!({ "kind": "int64", "value": value })
}

fn f(value: f64) -> Value {
    serde_json::json!({ "kind": "float64", "value": value })
}

fn s(value: &str) -> Value {
    serde_json::json!({ "kind": "utf8", "value": value })
}

fn null() -> Value {
    serde_json::json!({ "kind": "null", "value": "null" })
}

fn nan() -> Value {
    serde_json::json!({ "kind": "null", "value": "na_n" })
}

fn series(name: &str, index: Vec<Value>, values: Vec<Value>) -> Value {
    serde_json::json!({
        "name": name,
        "index": index,
        "values": values
    })
}

fn frame(index: Vec<Value>, column_order: &[&str], columns: &[(&str, Vec<Value>)]) -> Value {
    let mut column_map = Map::new();
    for (name, values) in columns {
        column_map.insert((*name).to_owned(), Value::Array(values.clone()));
    }
    serde_json::json!({
        "index": index,
        "column_order": column_order,
        "columns": column_map
    })
}

fn series_fixture(
    packet_id: &str,
    case_id: &str,
    operation: &str,
    left: Value,
    options: &[(&str, Value)],
) -> PacketFixture {
    let mut raw = serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": operation,
        "oracle_source": "live_legacy_pandas",
        "left": left
    });
    let raw_object = raw.as_object_mut().expect("fixture object");
    for (key, value) in options {
        raw_object.insert((*key).to_owned(), value.clone());
    }
    serde_json::from_value(raw).expect("fixture")
}

fn frame_fixture(
    packet_id: &str,
    case_id: &str,
    operation: &str,
    frame: Value,
    options: &[(&str, Value)],
) -> PacketFixture {
    let mut raw = serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": operation,
        "oracle_source": "live_legacy_pandas",
        "frame": frame
    });
    let raw_object = raw.as_object_mut().expect("fixture object");
    for (key, value) in options {
        raw_object.insert((*key).to_owned(), value.clone());
    }
    serde_json::from_value(raw).expect("fixture")
}

#[test]
fn conformance_window_series_rolling_mean_empty_input() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-001",
        "window_series_rolling_mean_empty_input",
        "series_rolling_mean",
        series("value", vec![], vec![]),
        &[("window_size", serde_json::json!(3))],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_series_rolling_sum_single_row_window_one() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-002",
        "window_series_rolling_sum_single_row_window_one",
        "series_rolling_sum",
        series("value", vec![i(0)], vec![f(5.0)]),
        &[("window_size", serde_json::json!(1))],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_series_rolling_min_nan_heavy_min_periods_one() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-003",
        "window_series_rolling_min_nan_heavy_min_periods_one",
        "series_rolling_min",
        series(
            "value",
            vec![i(0), i(1), i(2), i(3), i(4)],
            vec![f(5.0), nan(), f(3.0), null(), f(1.0)],
        ),
        &[
            ("window_size", serde_json::json!(3)),
            ("min_periods", serde_json::json!(1)),
        ],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_series_rolling_mean_duplicate_index_labels() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-004",
        "window_series_rolling_mean_duplicate_index_labels",
        "series_rolling_mean",
        series(
            "value",
            vec![s("dup"), s("dup"), s("tail")],
            vec![f(1.0), f(2.0), f(3.0)],
        ),
        &[("window_size", serde_json::json!(2))],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_series_rolling_count_duplicate_index_null_window() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-004B",
        "window_series_rolling_count_duplicate_index_null_window",
        "series_rolling_count",
        series(
            "value",
            vec![s("dup"), s("dup"), s("tail")],
            vec![f(1.0), null(), f(3.0)],
        ),
        &[("window_size", serde_json::json!(2))],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_series_rolling_var_boundary_window_equals_length() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-005",
        "window_series_rolling_var_boundary_window_equals_length",
        "series_rolling_var",
        series(
            "value",
            vec![i(0), i(1), i(2), i(3)],
            vec![f(1.0), f(2.0), f(3.0), f(4.0)],
        ),
        &[("window_size", serde_json::json!(4))],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_series_expanding_sum_empty_input() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-006",
        "window_series_expanding_sum_empty_input",
        "series_expanding_sum",
        series("value", vec![], vec![]),
        &[("min_periods", serde_json::json!(1))],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_series_expanding_mean_nan_heavy_min_periods_one() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-007",
        "window_series_expanding_mean_nan_heavy_min_periods_one",
        "series_expanding_mean",
        series(
            "value",
            vec![i(0), i(1), i(2), i(3)],
            vec![f(1.0), nan(), f(3.0), null()],
        ),
        &[("min_periods", serde_json::json!(1))],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_series_expanding_quantile_median_min_periods_one() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-008",
        "window_series_expanding_quantile_median_min_periods_one",
        "series_expanding_quantile",
        series(
            "value",
            vec![i(0), i(1), i(2)],
            vec![f(4.0), f(1.0), f(3.0)],
        ),
        &[
            ("min_periods", serde_json::json!(1)),
            ("quantile_value", serde_json::json!(0.5)),
        ],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_series_ewm_mean_span_three_clean_values() {
    let fixture = series_fixture(
        "FP-CONF-WINDOW-009",
        "window_series_ewm_mean_span_three_clean_values",
        "series_ewm_mean",
        series(
            "value",
            vec![i(0), i(1), i(2), i(3)],
            vec![f(1.0), f(2.0), f(3.0), f(4.0)],
        ),
        &[("ewm_span", serde_json::json!(3.0))],
    );
    check_window_fixture(fixture);
}

#[test]
fn conformance_window_dataframe_rolling_mean_mixed_numeric_dtypes() {
    let fixture = frame_fixture(
        "FP-CONF-WINDOW-010",
        "window_dataframe_rolling_mean_mixed_numeric_dtypes",
        "dataframe_rolling_mean",
        frame(
            vec![i(0), i(1), i(2), i(3)],
            &["ints", "floats"],
            &[
                ("ints", vec![i(1), i(2), i(3), i(4)]),
                ("floats", vec![f(1.5), f(2.5), nan(), f(4.5)]),
            ],
        ),
        &[("window_size", serde_json::json!(2))],
    );
    check_window_fixture(fixture);
}
