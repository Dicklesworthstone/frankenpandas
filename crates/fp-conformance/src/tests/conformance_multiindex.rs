//! MultiIndex parity-matrix conformance suite (br-frankenpandas-m8cp).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares existing
//! row-MultiIndex DataFrame behavior with live upstream pandas for edge-case
//! inputs: empty named levels, single tuples, duplicate tuples, mixed level
//! dtypes, partial tuple keys, exact duplicate tuple keys, and level-dropping
//! operations.

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
        Ok(ResolvedExpected::Frame(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping MultiIndex conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_multiindex_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("multiindex oracle") {
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
        "pandas MultiIndex parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

fn empty_named_levels_frame_json() -> serde_json::Value {
    serde_json::json!({
        "index": [],
        "row_multiindex": {
            "tuples": [],
            "names": ["outer", "inner"]
        },
        "column_order": ["value"],
        "columns": {
            "value": []
        }
    })
}

fn mixed_three_level_frame_json() -> serde_json::Value {
    serde_json::json!({
        "index": [
            { "kind": "utf8", "value": "east|apple|2023" },
            { "kind": "utf8", "value": "east|apple|2024" },
            { "kind": "utf8", "value": "east|pear|2023" },
            { "kind": "utf8", "value": "west|apple|2023" },
            { "kind": "utf8", "value": "west|pear|2024" }
        ],
        "row_multiindex": {
            "tuples": [
                [
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "apple" },
                    { "kind": "int64", "value": 2023 }
                ],
                [
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "apple" },
                    { "kind": "int64", "value": 2024 }
                ],
                [
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "pear" },
                    { "kind": "int64", "value": 2023 }
                ],
                [
                    { "kind": "utf8", "value": "west" },
                    { "kind": "utf8", "value": "apple" },
                    { "kind": "int64", "value": 2023 }
                ],
                [
                    { "kind": "utf8", "value": "west" },
                    { "kind": "utf8", "value": "pear" },
                    { "kind": "int64", "value": 2024 }
                ]
            ],
            "names": ["region", "product", "year"]
        },
        "column_order": ["sales", "cost"],
        "columns": {
            "sales": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 15 },
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 25 }
            ],
            "cost": [
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 7 },
                { "kind": "int64", "value": 6 },
                { "kind": "int64", "value": 12 },
                { "kind": "int64", "value": 9 }
            ]
        }
    })
}

fn duplicate_tuple_frame_json() -> serde_json::Value {
    serde_json::json!({
        "index": [
            { "kind": "utf8", "value": "a|1" },
            { "kind": "utf8", "value": "a|1" },
            { "kind": "utf8", "value": "a|2" },
            { "kind": "utf8", "value": "b|1" }
        ],
        "row_multiindex": {
            "tuples": [
                [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "int64", "value": 1 }
                ],
                [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "int64", "value": 1 }
                ],
                [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "int64", "value": 2 }
                ],
                [
                    { "kind": "utf8", "value": "b" },
                    { "kind": "int64", "value": 1 }
                ]
            ],
            "names": ["letter", "bucket"]
        },
        "column_order": ["value", "label"],
        "columns": {
            "value": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 11 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ],
            "label": [
                { "kind": "utf8", "value": "first" },
                { "kind": "utf8", "value": "second" },
                { "kind": "utf8", "value": "third" },
                { "kind": "utf8", "value": "fourth" }
            ]
        }
    })
}

#[test]
fn conformance_multiindex_identity_empty_named_levels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-ID-001",
        "case_id": "multiindex_identity_empty_named_levels",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": empty_named_levels_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_identity_single_tuple_mixed_dtypes() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-ID-002",
        "case_id": "multiindex_identity_single_tuple_mixed_dtypes",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [{ "kind": "utf8", "value": "north|2024" }],
            "row_multiindex": {
                "tuples": [[
                    { "kind": "utf8", "value": "north" },
                    { "kind": "int64", "value": 2024 }
                ]],
                "names": ["region", "year"]
            },
            "column_order": ["sales", "note"],
            "columns": {
                "sales": [{ "kind": "int64", "value": 42 }],
                "note": [{ "kind": "utf8", "value": "single" }]
            }
        }
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_identity_duplicate_tuples() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-ID-003",
        "case_id": "multiindex_identity_duplicate_tuples",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": duplicate_tuple_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_loc_partial_prefix_mixed_levels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-LOC-001",
        "case_id": "multiindex_loc_partial_prefix_mixed_levels",
        "mode": "strict",
        "operation": "dataframe_loc",
        "oracle_source": "live_legacy_pandas",
        "loc_labels": [{ "kind": "utf8", "value": "east" }],
        "frame": mixed_three_level_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_loc_exact_duplicate_tuple() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-LOC-002",
        "case_id": "multiindex_loc_exact_duplicate_tuple",
        "mode": "strict",
        "operation": "dataframe_loc",
        "oracle_source": "live_legacy_pandas",
        "loc_labels": [
            { "kind": "utf8", "value": "a" },
            { "kind": "int64", "value": 1 }
        ],
        "frame": duplicate_tuple_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_xs_first_level_drops_level() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-XS-001",
        "case_id": "multiindex_xs_first_level_drops_level",
        "mode": "strict",
        "operation": "dataframe_xs",
        "oracle_source": "live_legacy_pandas",
        "xs_key": { "kind": "utf8", "value": "east" },
        "xs_level": 0,
        "frame": mixed_three_level_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_xs_middle_level_reorders_remaining_levels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-XS-002",
        "case_id": "multiindex_xs_middle_level_reorders_remaining_levels",
        "mode": "strict",
        "operation": "dataframe_xs",
        "oracle_source": "live_legacy_pandas",
        "xs_key": { "kind": "utf8", "value": "apple" },
        "xs_level": 1,
        "frame": mixed_three_level_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_reset_index_preserves_level_order() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-RESET-001",
        "case_id": "multiindex_reset_index_preserves_level_order",
        "mode": "strict",
        "operation": "dataframe_reset_index",
        "oracle_source": "live_legacy_pandas",
        "reset_index_drop": false,
        "frame": mixed_three_level_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}
