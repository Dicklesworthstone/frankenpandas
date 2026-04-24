//! DataFrame parity-matrix conformance suite (br-frankenpandas-mt0v).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares the
//! existing Rust DataFrame operation with live upstream pandas for an
//! edge-case input: empty frames, single-row frames, NaN-heavy columns,
//! duplicate row labels, mixed dtypes, and larger ordered slices.

use super::{
    CaseStatus, HarnessConfig, HarnessError, OracleMode, PacketFixture, ResolvedExpected,
    SuiteOptions, capture_live_oracle_expected,
};

fn strict_config() -> HarnessConfig {
    HarnessConfig::default_paths()
}

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Frame(_) | ResolvedExpected::Series(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping DataFrame conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_dataframe_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("dataframe oracle") {
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
        "pandas DataFrame parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

#[test]
fn conformance_dataframe_identity_empty_columns() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-ID-001",
        "case_id": "dataframe_identity_empty_columns",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [],
            "column_order": ["a", "b"],
            "columns": {
                "a": [],
                "b": []
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_identity_single_row_mixed_dtypes() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-ID-002",
        "case_id": "dataframe_identity_single_row_mixed_dtypes",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [{ "kind": "int64", "value": 0 }],
            "column_order": ["int_col", "float_col", "str_col", "bool_col"],
            "columns": {
                "int_col": [{ "kind": "int64", "value": 42 }],
                "float_col": [{ "kind": "float64", "value": 3.5 }],
                "str_col": [{ "kind": "utf8", "value": "x" }],
                "bool_col": [{ "kind": "bool", "value": true }]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_identity_duplicate_index_labels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-ID-003",
        "case_id": "dataframe_identity_duplicate_index_labels",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "dup" },
                { "kind": "utf8", "value": "dup" },
                { "kind": "utf8", "value": "tail" }
            ],
            "column_order": ["value", "label"],
            "columns": {
                "value": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "label": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "c" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_isna_nan_heavy() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-NULL-001",
        "case_id": "dataframe_isna_nan_heavy",
        "mode": "strict",
        "operation": "dataframe_isna",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "column_order": ["a", "b", "c"],
            "columns": {
                "a": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 4.0 }
                ],
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "null", "value": "null" },
                    { "kind": "utf8", "value": "z" },
                    { "kind": "null", "value": "na_n" }
                ],
                "c": [
                    { "kind": "bool", "value": true },
                    { "kind": "bool", "value": false },
                    { "kind": "null", "value": "null" },
                    { "kind": "bool", "value": true }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_count_mixed_nulls() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-COUNT-001",
        "case_id": "dataframe_count_mixed_nulls",
        "mode": "strict",
        "operation": "dataframe_count",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ],
            "column_order": ["num", "text", "all_missing"],
            "columns": {
                "num": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "int64", "value": 3 }
                ],
                "text": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "null", "value": "null" }
                ],
                "all_missing": [
                    { "kind": "null", "value": "null" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "null" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_head_duplicate_index_labels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-HEAD-001",
        "case_id": "dataframe_head_duplicate_index_labels",
        "mode": "strict",
        "operation": "dataframe_head",
        "oracle_source": "live_legacy_pandas",
        "head_n": 2,
        "frame": {
            "index": [
                { "kind": "utf8", "value": "x" },
                { "kind": "utf8", "value": "x" },
                { "kind": "utf8", "value": "y" }
            ],
            "column_order": ["a", "b"],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "b": [
                    { "kind": "utf8", "value": "first" },
                    { "kind": "utf8", "value": "second" },
                    { "kind": "utf8", "value": "third" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_tail_larger_ordered_slice() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-TAIL-001",
        "case_id": "dataframe_tail_larger_ordered_slice",
        "mode": "strict",
        "operation": "dataframe_tail",
        "oracle_source": "live_legacy_pandas",
        "tail_n": 4,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "column_order": ["value", "bucket"],
            "columns": {
                "value": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 40 },
                    { "kind": "int64", "value": 50 }
                ],
                "bucket": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "c" },
                    { "kind": "utf8", "value": "c" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_reindex_columns_with_missing_column() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-REINDEX-COLS-001",
        "case_id": "dataframe_reindex_columns_with_missing_column",
        "mode": "strict",
        "operation": "dataframe_reindex_columns",
        "oracle_source": "live_legacy_pandas",
        "reindex_columns": ["b", "missing", "a"],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a", "b"],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ],
                "b": [
                    { "kind": "utf8", "value": "left" },
                    { "kind": "utf8", "value": "right" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_sort_index_unsorted_duplicate_ints() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-SORT-001",
        "case_id": "dataframe_sort_index_unsorted_duplicate_ints",
        "mode": "strict",
        "operation": "dataframe_sort_index",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "frame": {
            "index": [
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["value", "tag"],
            "columns": {
                "value": [
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 11 },
                    { "kind": "int64", "value": 20 }
                ],
                "tag": [
                    { "kind": "utf8", "value": "c" },
                    { "kind": "utf8", "value": "a1" },
                    { "kind": "utf8", "value": "a2" },
                    { "kind": "utf8", "value": "b" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}
