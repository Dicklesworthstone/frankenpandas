//! Categorical parity-matrix conformance suite (br-frankenpandas-vic4).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares
//! `pd.Categorical.from_codes` materialization behavior with live upstream
//! pandas for edge-case inputs: empty categoricals, single values, repeated
//! categories, missing codes, ordered categories, unused categories, numeric
//! categories, and mixed label categories.

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
        Ok(ResolvedExpected::Series(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping Categorical conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_categorical_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("categorical oracle") {
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
        "pandas Categorical parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

fn scalar_utf8(value: &str) -> serde_json::Value {
    serde_json::json!({ "kind": "utf8", "value": value })
}

fn scalar_int(value: i64) -> serde_json::Value {
    serde_json::json!({ "kind": "int64", "value": value })
}

fn scalar_bool(value: bool) -> serde_json::Value {
    serde_json::json!({ "kind": "bool", "value": value })
}

fn categorical_fixture(
    packet_id: &str,
    case_id: &str,
    codes: &[i64],
    categories: Vec<serde_json::Value>,
    ordered: bool,
) -> PacketFixture {
    let index = (0..codes.len())
        .map(|idx| serde_json::json!({ "kind": "int64", "value": idx as i64 }))
        .collect::<Vec<_>>();
    let values = codes
        .iter()
        .map(|code| serde_json::json!({ "kind": "int64", "value": *code }))
        .collect::<Vec<_>>();

    serde_json::from_value(serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": "series_categorical_from_codes",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "cat",
            "index": index,
            "values": values
        },
        "categorical_categories": categories,
        "categorical_ordered": ordered
    }))
    .expect("fixture")
}

#[test]
fn conformance_categorical_empty_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-001",
        "categorical_empty_categories",
        &[],
        Vec::new(),
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_single_value() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-002",
        "categorical_single_value",
        &[0],
        vec![scalar_utf8("only")],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_repeated_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-003",
        "categorical_repeated_categories",
        &[0, 1, 0, 2, 1],
        vec![
            scalar_utf8("red"),
            scalar_utf8("green"),
            scalar_utf8("blue"),
        ],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_missing_codes() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-004",
        "categorical_missing_codes",
        &[0, -1, 1, -1],
        vec![scalar_utf8("seen"), scalar_utf8("other")],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_ordered_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-005",
        "categorical_ordered_categories",
        &[2, 0, 1, 2],
        vec![
            scalar_utf8("low"),
            scalar_utf8("medium"),
            scalar_utf8("high"),
        ],
        true,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_unused_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-006",
        "categorical_unused_categories",
        &[0, 0, 2],
        vec![
            scalar_utf8("active"),
            scalar_utf8("unused"),
            scalar_utf8("also_active"),
        ],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_numeric_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-007",
        "categorical_numeric_categories",
        &[1, 0, 1, -1],
        vec![scalar_int(10), scalar_int(20)],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_mixed_label_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-008",
        "categorical_mixed_label_categories",
        &[0, 1, 2, -1, 0],
        vec![scalar_utf8("x"), scalar_int(7), scalar_bool(true)],
        false,
    );
    check_categorical_fixture(fixture);
}
