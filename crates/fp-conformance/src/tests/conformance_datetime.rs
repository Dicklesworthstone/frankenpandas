//! Datetime parity-matrix conformance suite (br-frankenpandas-ow57).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares
//! `pd.to_datetime` materialization behavior with live upstream pandas for
//! edge-case inputs: empty inputs, single timestamps, NaT-heavy coercion,
//! format variation, timezone normalization, epoch boundaries, nanosecond
//! precision, and custom origins.

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
                "live pandas unavailable; skipping datetime conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_datetime_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("datetime oracle") {
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
        "pandas datetime parity drift for {}: {:?}",
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

fn scalar_float(value: f64) -> serde_json::Value {
    serde_json::json!({ "kind": "float64", "value": value })
}

fn scalar_null() -> serde_json::Value {
    serde_json::json!({ "kind": "null", "value": "null" })
}

fn datetime_fixture(
    packet_id: &str,
    case_id: &str,
    name: &str,
    values: Vec<serde_json::Value>,
    options: &[(&str, serde_json::Value)],
) -> PacketFixture {
    let index = (0..values.len())
        .map(|idx| serde_json::json!({ "kind": "int64", "value": idx as i64 }))
        .collect::<Vec<_>>();
    let mut raw = serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": name,
            "index": index,
            "values": values
        }
    });
    let raw_object = raw.as_object_mut().expect("fixture object");
    for (key, value) in options {
        raw_object.insert((*key).to_owned(), value.clone());
    }

    serde_json::from_value(raw).expect("fixture")
}

#[test]
fn conformance_datetime_empty_input() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-001",
        "datetime_empty_input",
        "ts",
        Vec::new(),
        &[],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_single_iso_timestamp() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-002",
        "datetime_single_iso_timestamp",
        "ts",
        vec![scalar_utf8("2024-01-02T03:04:05")],
        &[],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_nat_heavy_coercion() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-003",
        "datetime_nat_heavy_coercion",
        "ts",
        vec![
            scalar_utf8("2024-01-01"),
            scalar_null(),
            scalar_utf8("not-a-date"),
            scalar_utf8("2024-02-29"),
        ],
        &[],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_slash_format_variation() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-004",
        "datetime_slash_format_variation",
        "ts",
        vec![scalar_utf8("2024/03/25"), scalar_utf8("2024/03/26")],
        &[],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_utc_timezone_normalization() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-005",
        "datetime_utc_timezone_normalization",
        "ts",
        vec![
            scalar_utf8("2024-01-15 10:30:00+05:30"),
            scalar_utf8("2024-01-15 10:30:00+00:00"),
        ],
        &[("datetime_utc", serde_json::json!(true))],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_utc_mixed_naive_offset_coercion() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-009",
        "datetime_utc_mixed_naive_offset_coercion",
        "ts",
        vec![
            scalar_utf8("2024-01-15 10:30:00"),
            scalar_utf8("2024-01-15 10:30:00+05:30"),
        ],
        &[("datetime_utc", serde_json::json!(true))],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_epoch_seconds_boundary_values() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-006",
        "datetime_epoch_seconds_boundary_values",
        "epoch_s",
        vec![
            scalar_int(0),
            scalar_int(1),
            scalar_int(-1),
            scalar_int(2_147_483_647),
        ],
        &[("datetime_unit", serde_json::json!("s"))],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_epoch_nanoseconds_precision() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-007",
        "datetime_epoch_nanoseconds_precision",
        "epoch_ns",
        vec![scalar_int(1_490_195_805_433_502_912)],
        &[("datetime_unit", serde_json::json!("ns"))],
    );
    check_datetime_fixture(fixture);
}

#[test]
fn conformance_datetime_custom_origin_day_offsets() {
    let fixture = datetime_fixture(
        "FP-CONF-DATETIME-008",
        "datetime_custom_origin_day_offsets",
        "epoch_d",
        vec![scalar_int(0), scalar_int(1), scalar_float(2.5)],
        &[
            ("datetime_unit", serde_json::json!("D")),
            ("datetime_origin", serde_json::json!("1960-01-01")),
        ],
    );
    check_datetime_fixture(fixture);
}
