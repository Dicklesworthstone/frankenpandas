#[test]
fn live_oracle_series_to_numeric_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-074",
        "case_id": "series_to_numeric_live",
        "mode": "strict",
        "operation": "series_to_numeric",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "raw",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "1" },
                { "kind": "utf8", "value": "2.5" },
                { "kind": "utf8", "value": "bad" },
                { "kind": "bool", "value": true }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series to_numeric oracle test: {message}");
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

    let actual = super::execute_series_module_utility_fixture_operation(&fixture).expect("actual");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_cut_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-075",
        "case_id": "series_cut_live",
        "mode": "strict",
        "operation": "series_cut",
        "oracle_source": "live_legacy_pandas",
        "cut_bins": 3,
        "left": {
            "name": "nums",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 7 },
                { "kind": "null", "value": "na_n" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series cut oracle test: {message}");
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

    let actual = super::execute_series_module_utility_fixture_operation(&fixture).expect("actual");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_qcut_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-076",
        "case_id": "series_qcut_live",
        "mode": "strict",
        "operation": "series_qcut",
        "oracle_source": "live_legacy_pandas",
        "qcut_quantiles": 2,
        "left": {
            "name": "nums",
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
                { "kind": "null", "value": "na_n" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series qcut oracle test: {message}");
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

    let actual = super::execute_series_module_utility_fixture_operation(&fixture).expect("actual");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_at_time_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2C-010",
        "case_id": "series_at_time_live",
        "mode": "strict",
        "operation": "series_at_time",
        "oracle_source": "live_legacy_pandas",
        "time_value": "10:00:00",
        "left": {
            "name": "values",
            "index": [
                { "kind": "utf8", "value": "2024-01-01T10:00:00" },
                { "kind": "utf8", "value": "2024-01-02T10:00:00" },
                { "kind": "utf8", "value": "2024-01-03T14:30:00" }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series at_time oracle test: {message}");
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

    let left = super::require_left_series(&fixture).expect("left series");
    let series = super::build_series(left).expect("build series");
    let actual = series.at_time(fixture.time_value.as_deref().expect("time value"));
    super::compare_series_expected(&actual.expect("actual series"), &expected)
        .expect("pandas parity");
}

#[test]
fn live_oracle_series_between_time_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2C-010",
        "case_id": "series_between_time_live",
        "mode": "strict",
        "operation": "series_between_time",
        "oracle_source": "live_legacy_pandas",
        "start_time": "09:00:00",
        "end_time": "16:00:00",
        "left": {
            "name": "values",
            "index": [
                { "kind": "utf8", "value": "2024-01-01T08:00:00" },
                { "kind": "utf8", "value": "2024-01-01T12:30:00" },
                { "kind": "utf8", "value": "2024-01-01T15:00:00" },
                { "kind": "utf8", "value": "2024-01-01T20:00:00" }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series between_time oracle test: {message}");
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

    let left = super::require_left_series(&fixture).expect("left series");
    let series = super::build_series(left).expect("build series");
    let actual = series.between_time(
        fixture.start_time.as_deref().expect("start time"),
        fixture.end_time.as_deref().expect("end time"),
    );
    super::compare_series_expected(&actual.expect("actual series"), &expected)
        .expect("pandas parity");
}

fn autocorr_fixture(
    case_id: &str,
    lag: usize,
    values: Vec<serde_json::Value>,
) -> super::PacketFixture {
    let index = (0..values.len())
        .map(|value| serde_json::json!({ "kind": "int64", "value": value }))
        .collect::<Vec<_>>();

    serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-430",
        "case_id": case_id,
        "mode": "strict",
        "operation": "series_autocorr",
        "oracle_source": "live_legacy_pandas",
        "autocorr_lag": lag,
        "left": {
            "name": "values",
            "index": index,
            "values": values
        }
    }))
    .expect("fixture")
}

#[test]
fn live_oracle_series_autocorr_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;

    let long_mixed = (0..64)
        .map(|idx| {
            if idx % 17 == 0 {
                serde_json::json!({ "kind": "null", "value": "na_n" })
            } else {
                let value = ((idx * idx + 3 * idx) % 37) as f64 - 18.5;
                serde_json::json!({ "kind": "float64", "value": value })
            }
        })
        .collect::<Vec<_>>();
    let fixtures = [
        autocorr_fixture("series_autocorr_lag3_mixed_missing_live", 3, long_mixed),
        autocorr_fixture(
            "series_autocorr_lag0_self_live",
            0,
            vec![
                serde_json::json!({ "kind": "int64", "value": 1 }),
                serde_json::json!({ "kind": "int64", "value": 4 }),
                serde_json::json!({ "kind": "int64", "value": 9 }),
                serde_json::json!({ "kind": "int64", "value": 16 }),
            ],
        ),
        autocorr_fixture(
            "series_autocorr_constant_nan_live",
            1,
            vec![
                serde_json::json!({ "kind": "int64", "value": 7 }),
                serde_json::json!({ "kind": "int64", "value": 7 }),
                serde_json::json!({ "kind": "int64", "value": 7 }),
                serde_json::json!({ "kind": "int64", "value": 7 }),
            ],
        ),
        autocorr_fixture(
            "series_autocorr_lag_exceeds_len_live",
            5,
            vec![
                serde_json::json!({ "kind": "int64", "value": 2 }),
                serde_json::json!({ "kind": "int64", "value": 3 }),
            ],
        ),
    ];

    for fixture in fixtures {
        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series autocorr oracle test: {message}");
            return;
        }

        let expected = expected_result.expect("live oracle expected");
        assert!(
            matches!(&expected, super::ResolvedExpected::Scalar(_)),
            "expected live oracle scalar payload for {}, got {expected:?}",
            fixture.case_id
        );
        let super::ResolvedExpected::Scalar(expected) = expected else {
            return;
        };

        let actual =
            super::execute_series_autocorr_fixture_operation(&fixture).expect("actual scalar");
        let scalar_comparison = super::compare_scalar(&actual, &expected, "series_autocorr");
        assert!(
            scalar_comparison.is_ok(),
            "{}: {}",
            fixture.case_id,
            scalar_comparison.err().unwrap_or_default()
        );

        let diff = super::run_differential_fixture(
            &cfg,
            &fixture,
            &super::SuiteOptions {
                packet_filter: None,
                oracle_mode: super::OracleMode::LiveLegacyPandas,
            },
        )
        .expect("differential report");
        assert_eq!(diff.status, super::CaseStatus::Pass, "{diff:?}");
        assert!(
            diff.drift_records.is_empty(),
            "expected no autocorr drift for {}: {diff:?}",
            fixture.case_id
        );
    }
}

#[test]
fn live_oracle_series_extractall_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-086",
        "case_id": "series_extractall_live",
        "mode": "strict",
        "operation": "series_extractall",
        "oracle_source": "live_legacy_pandas",
        "regex_pattern": "([a-z])(\\d)",
        "left": {
            "name": "tokens",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "a1b2" },
                { "kind": "utf8", "value": "c3" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series extractall oracle test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected) = expected else {
        return;
    };

    let actual = super::execute_dataframe_fixture_operation(&fixture).expect("actual frame");
    super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_extract_df_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-087",
        "case_id": "series_extract_df_live",
        "mode": "strict",
        "operation": "series_extract_df",
        "oracle_source": "live_legacy_pandas",
        "regex_pattern": "(?P<prefix>[a-z]+)-(?P<number>\\d+)",
        "left": {
            "name": "tokens",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "abc-123" },
                { "kind": "utf8", "value": "xyz" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series extract_df oracle test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected) = expected else {
        return;
    };

    let actual = super::execute_dataframe_fixture_operation(&fixture).expect("actual frame");
    super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_partition_df_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-088",
        "case_id": "series_partition_df_live",
        "mode": "strict",
        "operation": "series_partition_df",
        "oracle_source": "live_legacy_pandas",
        "string_sep": "-",
        "left": {
            "name": "tokens",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "a-b-c" },
                { "kind": "utf8", "value": "solo" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series partition_df oracle test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected) = expected else {
        return;
    };

    let actual = super::execute_dataframe_fixture_operation(&fixture).expect("actual frame");
    super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_rpartition_df_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-089",
        "case_id": "series_rpartition_df_live",
        "mode": "strict",
        "operation": "series_rpartition_df",
        "oracle_source": "live_legacy_pandas",
        "string_sep": "-",
        "left": {
            "name": "tokens",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "a-b-c" },
                { "kind": "utf8", "value": "solo" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series rpartition_df oracle test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected) = expected else {
        return;
    };

    let actual = super::execute_dataframe_fixture_operation(&fixture).expect("actual frame");
    super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_bool_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-068",
        "case_id": "series_bool_live",
        "mode": "strict",
        "operation": "series_bool",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "flag",
            "index": [{ "kind": "int64", "value": 0 }],
            "values": [{ "kind": "bool", "value": true }]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series bool oracle test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Bool(_)),
        "expected live oracle bool payload, got {expected:?}"
    );
    let super::ResolvedExpected::Bool(expected) = expected else {
        return;
    };

    let actual = super::execute_series_bool_fixture_operation(&fixture).expect("actual bool");
    assert_eq!(actual, expected);
}

#[test]
fn live_oracle_series_bool_non_boolean_errors_like_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-068",
        "case_id": "series_bool_non_boolean_live",
        "mode": "strict",
        "operation": "series_bool",
        "oracle_source": "live_legacy_pandas",
        "expected_error_contains": "boolean scalar",
        "left": {
            "name": "flag",
            "index": [{ "kind": "int64", "value": 0 }],
            "values": [{ "kind": "int64", "value": 1 }]
        }
    }))
    .expect("fixture");

    let expected = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected {
        eprintln!("live pandas unavailable; skipping series bool error oracle test: {message}");
        return;
    }

    let expected = expected.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::ErrorAny),
        "expected live oracle error payload, got {expected:?}"
    );

    let actual = super::execute_series_bool_fixture_operation(&fixture);
    assert!(
        actual
            .as_ref()
            .err()
            .is_some_and(|message| message.contains("boolean scalar")),
        "{actual:?}"
    );
}

#[test]
fn live_oracle_dataframe_bool_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-068",
        "case_id": "dataframe_bool_live",
        "mode": "strict",
        "operation": "dataframe_bool",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [{ "kind": "int64", "value": 0 }],
            "column_order": ["flag"],
            "columns": {
                "flag": [{ "kind": "bool", "value": false }]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dataframe bool oracle test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Bool(_)),
        "expected live oracle bool payload, got {expected:?}"
    );
    let super::ResolvedExpected::Bool(expected) = expected else {
        return;
    };

    let actual = super::execute_dataframe_bool_fixture_operation(&fixture).expect("actual bool");
    assert_eq!(actual, expected);
}

#[test]
fn live_oracle_dataframe_bool_non_boolean_errors_like_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-068",
        "case_id": "dataframe_bool_non_boolean_live",
        "mode": "strict",
        "operation": "dataframe_bool",
        "oracle_source": "live_legacy_pandas",
        "expected_error_contains": "boolean scalar",
        "frame": {
            "index": [{ "kind": "int64", "value": 0 }],
            "column_order": ["flag"],
            "columns": {
                "flag": [{ "kind": "int64", "value": 1 }]
            }
        }
    }))
    .expect("fixture");

    let expected = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected {
        eprintln!("live pandas unavailable; skipping dataframe bool error oracle test: {message}");
        return;
    }

    let expected = expected.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::ErrorAny),
        "expected live oracle error payload, got {expected:?}"
    );

    let actual = super::execute_dataframe_bool_fixture_operation(&fixture);
    assert!(
        actual
            .as_ref()
            .err()
            .is_some_and(|message| message.contains("boolean scalar")),
        "{actual:?}"
    );
}
