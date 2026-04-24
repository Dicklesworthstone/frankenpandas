//! GroupBy parity-matrix conformance suite (br-frankenpandas-yzh4).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares an
//! existing Series GroupBy aggregate operation with live upstream pandas for
//! edge-case inputs: empty groups, single groups, NaN-heavy values, duplicate
//! keys, mixed key dtypes, multi-key grouping, and ordered larger inputs.

use super::{
    CaseStatus, HarnessConfig, HarnessError, OracleMode, PacketFixture, ResolvedExpected,
    SuiteOptions, capture_live_oracle_expected,
};

fn strict_config() -> HarnessConfig {
    HarnessConfig::default_paths()
}

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Series(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping GroupBy conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_groupby_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("groupby oracle") {
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
        "pandas GroupBy parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

#[test]
fn conformance_groupby_sum_empty_input() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-SUM-001",
        "case_id": "groupby_sum_empty_input",
        "mode": "strict",
        "operation": "groupby_sum",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "key", "index": [], "values": [] },
        "right": { "name": "value", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_sum_single_group() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-SUM-002",
        "case_id": "groupby_sum_single_group",
        "mode": "strict",
        "operation": "groupby_sum",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "utf8", "value": "only" },
                { "kind": "utf8", "value": "only" },
                { "kind": "utf8", "value": "only" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_sum_nan_heavy_values() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-SUM-003",
        "case_id": "groupby_sum_nan_heavy_values",
        "mode": "strict",
        "operation": "groupby_sum",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 5 },
                { "kind": "null", "value": "na_n" },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "na_n" }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_mean_duplicate_keys_float_values() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-MEAN-001",
        "case_id": "groupby_mean_duplicate_keys_float_values",
        "mode": "strict",
        "operation": "groupby_mean",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 14.0 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_count_null_heavy_values() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-COUNT-001",
        "case_id": "groupby_count_null_heavy_values",
        "mode": "strict",
        "operation": "groupby_count",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_min_negative_values() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-MIN-001",
        "case_id": "groupby_min_negative_values",
        "mode": "strict",
        "operation": "groupby_min",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": -1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 5 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_max_duplicate_keys() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-MAX-001",
        "case_id": "groupby_max_duplicate_keys",
        "mode": "strict",
        "operation": "groupby_max",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": -1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 5 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_first_and_last_skip_missing_values() {
    let first_fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-FIRST-001",
        "case_id": "groupby_first_skip_missing_values",
        "mode": "strict",
        "operation": "groupby_first",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "c" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 6.0 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(first_fixture);

    let last_fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-LAST-001",
        "case_id": "groupby_last_skip_missing_values",
        "mode": "strict",
        "operation": "groupby_last",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "c" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 6.0 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(last_fixture);
}

#[test]
fn conformance_groupby_sum_multi_key_mixed_dtypes() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-MULTI-001",
        "case_id": "groupby_sum_multi_key_mixed_dtypes",
        "mode": "strict",
        "operation": "groupby_sum",
        "oracle_source": "live_legacy_pandas",
        "groupby_keys": [
            {
                "name": "region",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "values": [
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "west" },
                    { "kind": "utf8", "value": "west" }
                ]
            },
            {
                "name": "bucket",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "values": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ]
            }
        ],
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 40 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}
