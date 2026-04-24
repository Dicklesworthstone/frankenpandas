//! Index parity-matrix conformance suite (br-frankenpandas-jl63).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares the
//! existing Rust Index behavior with live upstream pandas for an edge-case
//! input: empty indexes, single labels, duplicate labels, mixed labels,
//! NA-like string labels, and extreme integer labels.

use super::{
    CaseStatus, HarnessConfig, HarnessError, OracleMode, PacketFixture, ResolvedExpected,
    SuiteOptions, capture_live_oracle_expected,
};

fn strict_config() -> HarnessConfig {
    HarnessConfig::default_paths()
}

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(
            ResolvedExpected::Alignment(_)
            | ResolvedExpected::Bool(_)
            | ResolvedExpected::Positions(_),
        ) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping Index conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_index_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("index oracle") {
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
        "pandas Index parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

#[test]
fn conformance_index_align_union_empty_pair() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-ALIGN-001",
        "case_id": "index_align_union_empty_pair",
        "mode": "strict",
        "operation": "index_align_union",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "left", "index": [], "values": [] },
        "right": { "name": "right", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_align_union_single_label() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-ALIGN-002",
        "case_id": "index_align_union_single_label",
        "mode": "strict",
        "operation": "index_align_union",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "left",
            "index": [{ "kind": "int64", "value": 7 }],
            "values": []
        },
        "right": {
            "name": "right",
            "index": [{ "kind": "int64", "value": 7 }],
            "values": []
        }
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_align_union_mixed_labels_preserves_order() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-ALIGN-003",
        "case_id": "index_align_union_mixed_labels_preserves_order",
        "mode": "strict",
        "operation": "index_align_union",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "left",
            "index": [
                { "kind": "utf8", "value": "b" },
                { "kind": "int64", "value": 1 }
            ],
            "values": []
        },
        "right": {
            "name": "right",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" }
            ],
            "values": []
        }
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_has_duplicates_empty() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-DUPS-001",
        "case_id": "index_has_duplicates_empty",
        "mode": "strict",
        "operation": "index_has_duplicates",
        "oracle_source": "live_legacy_pandas",
        "index": []
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_has_duplicates_na_like_strings() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-DUPS-002",
        "case_id": "index_has_duplicates_na_like_strings",
        "mode": "strict",
        "operation": "index_has_duplicates",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "utf8", "value": "NaN" },
            { "kind": "utf8", "value": "x" },
            { "kind": "utf8", "value": "NaN" }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_first_positions_duplicate_ints() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-POS-001",
        "case_id": "index_first_positions_duplicate_ints",
        "mode": "strict",
        "operation": "index_first_positions",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "int64", "value": 2 },
            { "kind": "int64", "value": 1 },
            { "kind": "int64", "value": 2 },
            { "kind": "int64", "value": 3 }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_first_positions_mixed_labels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-POS-002",
        "case_id": "index_first_positions_mixed_labels",
        "mode": "strict",
        "operation": "index_first_positions",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "utf8", "value": "alpha" },
            { "kind": "int64", "value": 1 },
            { "kind": "utf8", "value": "alpha" },
            { "kind": "utf8", "value": "missing" },
            { "kind": "int64", "value": 1 }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_monotonic_increasing_duplicate_plateau() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-MONO-001",
        "case_id": "index_monotonic_increasing_duplicate_plateau",
        "mode": "strict",
        "operation": "index_is_monotonic_increasing",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "int64", "value": -1 },
            { "kind": "int64", "value": -1 },
            { "kind": "int64", "value": 0 },
            { "kind": "int64", "value": 5 }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_monotonic_decreasing_extreme_ints() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-MONO-002",
        "case_id": "index_monotonic_decreasing_extreme_ints",
        "mode": "strict",
        "operation": "index_is_monotonic_decreasing",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "int64", "value": i64::MAX },
            { "kind": "int64", "value": 0 },
            { "kind": "int64", "value": i64::MIN }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}
