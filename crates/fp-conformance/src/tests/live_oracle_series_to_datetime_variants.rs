#[test]
fn live_oracle_series_to_datetime_unit_nanoseconds_preserves_precision() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-064",
        "case_id": "series_to_datetime_unit_nanoseconds_live",
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "datetime_unit": "ns",
        "left": {
            "name": "epoch_ns",
            "index": [
                { "kind": "int64", "value": 0 }
            ],
            "values": [
                { "kind": "int64", "value": 1490195805433502912_i64 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping to_datetime unit nanoseconds oracle test: {message}"
        );
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let actual = fp_frame::to_datetime_with_unit(
        &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
        fixture.datetime_unit.as_deref().expect("unit"),
    )
    .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_to_datetime_origin_days_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-064",
        "case_id": "series_to_datetime_origin_days_live",
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "datetime_unit": "D",
        "datetime_origin": "1960-01-01",
        "left": {
            "name": "epoch_d",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "float64", "value": 2.5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping to_datetime origin oracle test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let actual = fp_frame::to_datetime_with_options(
        &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
        fp_frame::ToDatetimeOptions {
            unit: fixture.datetime_unit.as_deref(),
            origin: super::resolve_datetime_origin_option(fixture.datetime_origin.as_ref())
                .expect("datetime origin"),
            ..fp_frame::ToDatetimeOptions::default()
        },
    )
    .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_to_datetime_timestamp_origin_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-091",
        "case_id": "series_to_datetime_timestamp_origin_live",
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "datetime_unit": "s",
        "datetime_origin": "1960-01-01 12:34:56.123456789",
        "left": {
            "name": "epoch_s",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 0 },
                { "kind": "float64", "value": 1.5 },
                { "kind": "int64", "value": -60 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping to_datetime timestamp-origin oracle test: {message}"
        );
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let actual = fp_frame::to_datetime_with_options(
        &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
        fp_frame::ToDatetimeOptions {
            unit: fixture.datetime_unit.as_deref(),
            origin: super::resolve_datetime_origin_option(fixture.datetime_origin.as_ref())
                .expect("datetime origin"),
            ..fp_frame::ToDatetimeOptions::default()
        },
    )
    .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_to_datetime_numeric_origin_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-064",
        "case_id": "series_to_datetime_numeric_origin_live",
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "datetime_unit": "D",
        "datetime_origin": 2,
        "left": {
            "name": "epoch_d",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "int64", "value": 0 },
                { "kind": "float64", "value": 1.5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping to_datetime numeric-origin oracle test: {message}"
        );
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let actual = fp_frame::to_datetime_with_options(
        &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
        fp_frame::ToDatetimeOptions {
            unit: fixture.datetime_unit.as_deref(),
            origin: super::resolve_datetime_origin_option(fixture.datetime_origin.as_ref())
                .expect("datetime origin"),
            ..fp_frame::ToDatetimeOptions::default()
        },
    )
    .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_to_datetime_julian_origin_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-064",
        "case_id": "series_to_datetime_julian_origin_live",
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "datetime_unit": "D",
        "datetime_origin": "julian",
        "left": {
            "name": "julian_d",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "float64", "value": 2451544.5 },
                { "kind": "float64", "value": 2451545.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping to_datetime julian-origin oracle test: {message}"
        );
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let actual = fp_frame::to_datetime_with_options(
        &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
        fp_frame::ToDatetimeOptions {
            unit: fixture.datetime_unit.as_deref(),
            origin: super::resolve_datetime_origin_option(fixture.datetime_origin.as_ref())
                .expect("datetime origin"),
            ..fp_frame::ToDatetimeOptions::default()
        },
    )
    .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_to_datetime_utc_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-064",
        "case_id": "series_to_datetime_utc_live",
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "datetime_utc": true,
        "left": {
            "name": "ts",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "2024-01-15 10:30:00" },
                { "kind": "utf8", "value": "2024-01-15 10:30:00+05:30" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping to_datetime utc oracle test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let actual = fp_frame::to_datetime_with_options(
        &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
        fp_frame::ToDatetimeOptions {
            utc: fixture.datetime_utc.unwrap_or(false),
            ..fp_frame::ToDatetimeOptions::default()
        },
    )
    .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_to_datetime_mixed_tz_strings_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-064",
        "case_id": "series_to_datetime_mixed_tz_strings_live",
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "ts",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "2024-01-15 10:30:00" },
                { "kind": "utf8", "value": "2024-01-15T10:30:00Z" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping to_datetime mixed-tz-string oracle test: {message}"
        );
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let actual = fp_frame::to_datetime(
        &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
    )
    .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}
