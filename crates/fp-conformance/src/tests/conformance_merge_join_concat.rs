//! Merge/join/concat parity-matrix conformance suite (br-frankenpandas-t7no).
//!
//! Per /testing-conformance-harnesses Pattern 1, these fixtures compare
//! FrankenPandas against live upstream pandas for merge, join, and concat edge
//! cases: empty inputs, single rows, NaN-heavy keys, duplicate keys, mixed
//! dtypes, ordering, index alignment, and larger ordered blocks.

use serde_json::{Map, Value};

use super::{
    CaseStatus, HarnessConfig, HarnessError, OracleMode, PacketFixture, ResolvedExpected,
    SuiteOptions, capture_live_oracle_expected,
};

fn strict_config() -> HarnessConfig {
    let mut cfg = HarnessConfig::default_paths();
    // br-frankenpandas-l7r1p: without this every fixture in this file SKIPS.
    // The legacy oracle root (`legacy_pandas_code/pandas`) does not exist on
    // this host, so `capture_live_oracle_expected` returns OracleUnavailable
    // and the check helper returns before comparing anything -- green while
    // executing no differential at all. The live_oracle_* suites already opt
    // into the SYSTEM pandas (2.2.3); the conformance_* suites never did.
    cfg.allow_system_pandas_fallback = true;
    cfg
}

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Frame(_) | ResolvedExpected::Join(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping merge/join/concat conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("merge/join/concat oracle") {
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
        "pandas merge/join/concat parity drift for {}: {:?}",
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

fn b(value: bool) -> Value {
    serde_json::json!({ "kind": "bool", "value": value })
}

fn null() -> Value {
    serde_json::json!({ "kind": "null", "value": "null" })
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
    left: Value,
    right: Value,
    options: &[(&str, Value)],
) -> PacketFixture {
    let mut raw = serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": operation,
        "oracle_source": "live_legacy_pandas",
        "frame": left,
        "frame_right": right
    });
    let raw_object = raw.as_object_mut().expect("fixture object");
    for (key, value) in options {
        raw_object.insert((*key).to_owned(), value.clone());
    }
    serde_json::from_value(raw).expect("fixture")
}

fn series_join_fixture(
    packet_id: &str,
    case_id: &str,
    left: Value,
    right: Value,
    join_type: &str,
) -> PacketFixture {
    serde_json::from_value(serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": "series_join",
        "oracle_source": "live_legacy_pandas",
        "left": left,
        "right": right,
        "join_type": join_type
    }))
    .expect("fixture")
}

#[test]
fn conformance_merge_inner_empty_left() {
    let fixture = frame_fixture(
        "FP-CONF-MJC-001",
        "merge_inner_empty_left",
        "dataframe_merge",
        frame(
            vec![],
            &["k", "left_val"],
            &[("k", vec![]), ("left_val", vec![])],
        ),
        frame(
            vec![i(0), i(1)],
            &["k", "right_val"],
            &[
                ("k", vec![i(1), i(2)]),
                ("right_val", vec![f(10.0), f(20.0)]),
            ],
        ),
        &[
            ("join_type", serde_json::json!("inner")),
            ("merge_on", serde_json::json!("k")),
        ],
    );
    check_fixture(fixture);
}

#[test]
fn conformance_merge_left_single_row_mixed_dtypes() {
    let fixture = frame_fixture(
        "FP-CONF-MJC-002",
        "merge_left_single_row_mixed_dtypes",
        "dataframe_merge",
        frame(
            vec![i(0)],
            &["k", "left_int", "left_flag"],
            &[
                ("k", vec![s("id-1")]),
                ("left_int", vec![i(7)]),
                ("left_flag", vec![b(true)]),
            ],
        ),
        frame(
            vec![i(0)],
            &["k", "right_float", "right_text"],
            &[
                ("k", vec![s("id-1")]),
                ("right_float", vec![f(3.25)]),
                ("right_text", vec![s("ok")]),
            ],
        ),
        &[
            ("join_type", serde_json::json!("left")),
            ("merge_on", serde_json::json!("k")),
        ],
    );
    check_fixture(fixture);
}

#[test]
fn conformance_merge_inner_duplicate_keys_multiply_rows() {
    let fixture = frame_fixture(
        "FP-CONF-MJC-003",
        "merge_inner_duplicate_keys_multiply_rows",
        "dataframe_merge",
        frame(
            vec![i(0), i(1), i(2)],
            &["k", "left_val"],
            &[
                ("k", vec![i(1), i(1), i(2)]),
                ("left_val", vec![s("a"), s("b"), s("c")]),
            ],
        ),
        frame(
            vec![i(0), i(1), i(2)],
            &["k", "right_val"],
            &[
                ("k", vec![i(1), i(1), i(3)]),
                ("right_val", vec![i(10), i(20), i(30)]),
            ],
        ),
        &[
            ("join_type", serde_json::json!("inner")),
            ("merge_on", serde_json::json!("k")),
        ],
    );
    check_fixture(fixture);
}

#[test]
fn conformance_merge_inner_null_keys_match() {
    let fixture = frame_fixture(
        "FP-CONF-MJC-004",
        "merge_inner_null_keys_match",
        "dataframe_merge",
        frame(
            vec![i(0), i(1), i(2)],
            &["k", "left_val"],
            &[
                ("k", vec![null(), s("x"), s("y")]),
                ("left_val", vec![f(1.0), f(2.0), f(3.0)]),
            ],
        ),
        frame(
            vec![i(0), i(1)],
            &["k", "right_val"],
            &[
                ("k", vec![null(), s("z")]),
                ("right_val", vec![s("missing"), s("tail")]),
            ],
        ),
        &[
            ("join_type", serde_json::json!("inner")),
            ("merge_on", serde_json::json!("k")),
        ],
    );
    check_fixture(fixture);
}

#[test]
fn conformance_merge_outer_sort_true_with_suffixes() {
    let fixture = frame_fixture(
        "FP-CONF-MJC-005",
        "merge_outer_sort_true_with_suffixes",
        "dataframe_merge",
        frame(
            vec![i(0), i(1)],
            &["k", "value"],
            &[("k", vec![i(2), i(1)]), ("value", vec![s("l2"), s("l1")])],
        ),
        frame(
            vec![i(0), i(1)],
            &["k", "value"],
            &[("k", vec![i(3), i(1)]), ("value", vec![s("r3"), s("r1")])],
        ),
        &[
            ("join_type", serde_json::json!("outer")),
            ("merge_on", serde_json::json!("k")),
            ("merge_sort", serde_json::json!(true)),
            ("merge_suffixes", serde_json::json!(["_left", "_right"])),
        ],
    );
    check_fixture(fixture);
}

#[test]
fn conformance_series_join_inner_misaligned_index() {
    let fixture = series_join_fixture(
        "FP-CONF-MJC-006",
        "series_join_inner_misaligned_index",
        series("left", vec![s("a"), s("c")], vec![f(1.0), f(3.0)]),
        series("right", vec![s("b"), s("c")], vec![f(20.0), f(30.0)]),
        "inner",
    );
    check_fixture(fixture);
}

#[test]
fn conformance_series_join_outer_sorts_union_index() {
    let fixture = series_join_fixture(
        "FP-CONF-MJC-006B",
        "series_join_outer_sorts_union_index",
        series("left", vec![s("a"), s("c")], vec![f(1.0), f(3.0)]),
        series("right", vec![s("b"), s("c")], vec![f(20.0), f(30.0)]),
        "outer",
    );
    check_fixture(fixture);
}

#[test]
fn conformance_concat_axis0_outer_column_union() {
    let fixture = frame_fixture(
        "FP-CONF-MJC-007",
        "concat_axis0_outer_column_union",
        "dataframe_concat",
        frame(
            vec![i(0), i(1)],
            &["k", "left_val"],
            &[("k", vec![i(1), i(2)]), ("left_val", vec![s("a"), s("b")])],
        ),
        frame(
            vec![i(0)],
            &["right_val", "k"],
            &[("right_val", vec![f(9.5)]), ("k", vec![i(3)])],
        ),
        &[
            ("concat_axis", serde_json::json!(0)),
            ("concat_join", serde_json::json!("outer")),
        ],
    );
    check_fixture(fixture);
}

#[test]
fn conformance_concat_axis1_outer_index_alignment() {
    let fixture = frame_fixture(
        "FP-CONF-MJC-008",
        "concat_axis1_outer_index_alignment",
        "dataframe_concat",
        frame(
            vec![s("a"), s("c")],
            &["left_val"],
            &[("left_val", vec![f(1.0), f(3.0)])],
        ),
        frame(
            vec![s("b"), s("c")],
            &["right_val"],
            &[("right_val", vec![f(20.0), f(30.0)])],
        ),
        &[
            ("concat_axis", serde_json::json!(1)),
            ("concat_join", serde_json::json!("outer")),
        ],
    );
    check_fixture(fixture);
}

#[test]
fn conformance_concat_axis1_outer_int_columns_promote_to_float() {
    let fixture = frame_fixture(
        "FP-CONF-MJC-008B",
        "concat_axis1_outer_int_columns_promote_to_float",
        "dataframe_concat",
        frame(
            vec![s("a"), s("c")],
            &["left_val"],
            &[("left_val", vec![i(1), i(3)])],
        ),
        frame(
            vec![s("b"), s("c")],
            &["right_val"],
            &[("right_val", vec![i(20), i(30)])],
        ),
        &[
            ("concat_axis", serde_json::json!(1)),
            ("concat_join", serde_json::json!("outer")),
        ],
    );
    check_fixture(fixture);
}

#[test]
fn conformance_concat_axis0_inner_shared_columns() {
    let fixture = frame_fixture(
        "FP-CONF-MJC-009",
        "concat_axis0_inner_shared_columns",
        "dataframe_concat",
        frame(
            vec![i(0)],
            &["shared", "left_only"],
            &[("shared", vec![s("left")]), ("left_only", vec![i(1)])],
        ),
        frame(
            vec![i(0)],
            &["shared", "right_only"],
            &[("shared", vec![s("right")]), ("right_only", vec![i(2)])],
        ),
        &[
            ("concat_axis", serde_json::json!(0)),
            ("concat_join", serde_json::json!("inner")),
        ],
    );
    check_fixture(fixture);
}

#[test]
fn conformance_concat_axis0_large_ordered_blocks() {
    let left_index = (0..32).map(i).collect::<Vec<_>>();
    let right_index = (0..32).map(i).collect::<Vec<_>>();
    let left_ids = (0..32).map(i).collect::<Vec<_>>();
    let right_ids = (32..64).map(i).collect::<Vec<_>>();
    let left_values = (0..32)
        .map(|value| f(value as f64 * 1.5))
        .collect::<Vec<_>>();
    let right_values = (32..64)
        .map(|value| f(value as f64 * 1.5))
        .collect::<Vec<_>>();

    let fixture = frame_fixture(
        "FP-CONF-MJC-010",
        "concat_axis0_large_ordered_blocks",
        "dataframe_concat",
        frame(
            left_index,
            &["id", "value"],
            &[("id", left_ids), ("value", left_values)],
        ),
        frame(
            right_index,
            &["id", "value"],
            &[("id", right_ids), ("value", right_values)],
        ),
        &[
            ("concat_axis", serde_json::json!(0)),
            ("concat_join", serde_json::json!("outer")),
        ],
    );
    check_fixture(fixture);
}
