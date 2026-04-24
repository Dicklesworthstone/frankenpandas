//! Reshape parity-matrix conformance suite (br-frankenpandas-6wa9).
//!
//! Per /testing-conformance-harnesses Pattern 1, these fixtures compare
//! FrankenPandas against live upstream pandas for melt, pivot, pivot_table,
//! stack, and unstack edge cases: empty inputs, single rows, missing-heavy
//! values, duplicate aggregate keys, mixed dtypes, and composite index labels.

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
        Ok(ResolvedExpected::Frame(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping reshape conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_reshape_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("reshape oracle") {
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
        "pandas reshape parity drift for {}: {:?}",
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

fn series(name: &str, index: Vec<Value>, values: Vec<Value>) -> Value {
    serde_json::json!({
        "name": name,
        "index": index,
        "values": values
    })
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

#[test]
fn conformance_reshape_melt_empty_frame() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-001",
        "reshape_melt_empty_frame",
        "dataframe_melt",
        frame(vec![], &["id", "x"], &[("id", vec![]), ("x", vec![])]),
        &[
            ("melt_id_vars", serde_json::json!(["id"])),
            ("melt_value_vars", serde_json::json!(["x"])),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_melt_single_row_custom_names() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-002",
        "reshape_melt_single_row_custom_names",
        "dataframe_melt",
        frame(
            vec![i(0)],
            &["key", "score"],
            &[("key", vec![s("row")]), ("score", vec![f(3.5)])],
        ),
        &[
            ("melt_id_vars", serde_json::json!(["key"])),
            ("melt_value_vars", serde_json::json!(["score"])),
            ("melt_var_name", serde_json::json!("metric")),
            ("melt_value_name", serde_json::json!("reading")),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_melt_nan_heavy_mixed_values() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-003",
        "reshape_melt_nan_heavy_mixed_values",
        "dataframe_melt",
        frame(
            vec![i(0), i(1), i(2)],
            &["id", "ints", "floats"],
            &[
                ("id", vec![s("a"), s("b"), s("c")]),
                ("ints", vec![i(1), null(), i(3)]),
                ("floats", vec![nan(), f(2.5), f(3.5)]),
            ],
        ),
        &[
            ("melt_id_vars", serde_json::json!(["id"])),
            ("melt_value_vars", serde_json::json!(["ints", "floats"])),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_pivot_single_row() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-004",
        "reshape_pivot_single_row",
        "dataframe_pivot",
        frame(
            vec![i(0)],
            &["row", "col", "value"],
            &[
                ("row", vec![s("r1")]),
                ("col", vec![s("c1")]),
                ("value", vec![f(10.0)]),
            ],
        ),
        &[
            ("pivot_index", serde_json::json!("row")),
            ("pivot_columns", serde_json::json!("col")),
            ("pivot_values", serde_json::json!(["value"])),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_pivot_wide_mixed_column_order() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-005",
        "reshape_pivot_wide_mixed_column_order",
        "dataframe_pivot",
        frame(
            vec![i(0), i(1), i(2), i(3)],
            &["row", "col", "value"],
            &[
                ("row", vec![s("r1"), s("r1"), s("r2"), s("r2")]),
                ("col", vec![s("b"), s("a"), s("b"), s("a")]),
                ("value", vec![i(20), i(10), i(40), i(30)]),
            ],
        ),
        &[
            ("pivot_index", serde_json::json!("row")),
            ("pivot_columns", serde_json::json!("col")),
            ("pivot_values", serde_json::json!(["value"])),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_pivot_table_duplicate_keys_mean() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-006",
        "reshape_pivot_table_duplicate_keys_mean",
        "dataframe_pivot_table",
        frame(
            vec![i(0), i(1), i(2), i(3)],
            &["region", "product", "sales"],
            &[
                ("region", vec![s("east"), s("east"), s("west"), s("east")]),
                ("product", vec![s("A"), s("A"), s("A"), s("B")]),
                ("sales", vec![f(10.0), f(14.0), f(7.0), f(5.0)]),
            ],
        ),
        &[
            ("pivot_index", serde_json::json!("region")),
            ("pivot_columns", serde_json::json!("product")),
            ("pivot_values", serde_json::json!(["sales"])),
            ("pivot_aggfunc", serde_json::json!("mean")),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_pivot_table_fill_missing_cells() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-007",
        "reshape_pivot_table_fill_missing_cells",
        "dataframe_pivot_table",
        frame(
            vec![i(0), i(1), i(2)],
            &["region", "product", "sales"],
            &[
                ("region", vec![s("east"), s("east"), s("west")]),
                ("product", vec![s("A"), s("B"), s("A")]),
                ("sales", vec![f(10.0), f(5.0), f(7.0)]),
            ],
        ),
        &[
            ("pivot_index", serde_json::json!("region")),
            ("pivot_columns", serde_json::json!("product")),
            ("pivot_values", serde_json::json!(["sales"])),
            ("pivot_aggfunc", serde_json::json!("sum")),
            (
                "fill_value",
                serde_json::json!({ "kind": "float64", "value": 0.0 }),
            ),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_stack_single_row_mixed_dtypes() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-008",
        "reshape_stack_single_row_mixed_dtypes",
        "dataframe_stack",
        frame(
            vec![s("r1")],
            &["int_col", "text_col"],
            &[("int_col", vec![i(1)]), ("text_col", vec![s("x")])],
        ),
        &[],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_stack_nan_heavy_values() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-009",
        "reshape_stack_nan_heavy_values",
        "dataframe_stack",
        frame(
            vec![s("r1"), s("r2")],
            &["a", "b"],
            &[("a", vec![f(1.0), nan()]), ("b", vec![null(), f(4.0)])],
        ),
        &[],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_series_unstack_composite_labels() {
    let fixture = series_fixture(
        "FP-CONF-RESHAPE-010",
        "reshape_series_unstack_composite_labels",
        "series_unstack",
        series(
            "value",
            vec![s("r1, a"), s("r1, b"), s("r2, a"), s("r2, b")],
            vec![f(1.0), f(2.0), f(3.0), f(4.0)],
        ),
        &[],
    );
    check_reshape_fixture(fixture);
}
