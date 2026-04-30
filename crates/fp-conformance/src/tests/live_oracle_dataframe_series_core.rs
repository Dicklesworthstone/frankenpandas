#[test]
fn live_oracle_series_constructor_mixed_utf8_numeric_reports_object_values() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-017",
        "case_id": "series_constructor_utf8_numeric_object_live",
        "mode": "strict",
        "operation": "series_constructor",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "bad_mix",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "x" },
                { "kind": "int64", "value": 1 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping mixed object series oracle test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle to return series payload: {expected:?}"
    );
    let series = if let super::ResolvedExpected::Series(series) = expected {
        series
    } else {
        return;
    };

    assert_eq!(series.index, vec![0_i64.into(), 1_i64.into()]);
    assert_eq!(
        series.values,
        vec![
            fp_types::Scalar::Utf8("x".to_owned()),
            fp_types::Scalar::Int64(1),
        ]
    );
}

#[test]
fn live_oracle_dataframe_from_series_mixed_utf8_numeric_matches_object_values() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-017",
        "case_id": "dataframe_from_series_utf8_numeric_object_live",
        "mode": "strict",
        "operation": "dataframe_from_series",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "bad",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "x" },
                { "kind": "int64", "value": 1 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping mixed object dataframe oracle test: {message}"
        );
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle to return dataframe payload: {expected:?}"
    );
    let frame = if let super::ResolvedExpected::Frame(frame) = expected {
        frame
    } else {
        return;
    };

    assert_eq!(frame.index, vec![0_i64.into(), 1_i64.into()]);
    assert_eq!(
        frame.columns.get("bad"),
        Some(&vec![
            fp_types::Scalar::Utf8("x".to_owned()),
            fp_types::Scalar::Int64(1),
        ])
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
    assert_eq!(diff.status, super::CaseStatus::Pass);
    assert_eq!(
        diff.oracle_source,
        super::FixtureOracleSource::LiveLegacyPandas
    );
    assert!(
        diff.drift_records.is_empty(),
        "expected no drift for mixed object constructor parity: {diff:?}"
    );
}

#[test]
fn live_oracle_series_combine_first_utf8_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-090",
        "case_id": "series_combine_first_utf8_live",
        "mode": "strict",
        "operation": "series_combine_first",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "primary",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "alpha" },
                { "kind": "null", "value": "null" }
            ]
        },
        "right": {
            "name": "fallback",
            "index": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "utf8", "value": "beta" },
                { "kind": "utf8", "value": "gamma" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series combine_first oracle test: {message}");
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

    let actual =
        super::execute_series_combine_first_fixture_operation(&fixture).expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_combine_first_object_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-090",
        "case_id": "dataframe_combine_first_object_live",
        "mode": "strict",
        "operation": "dataframe_combine_first",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a"],
            "columns": {
                "a": [
                    { "kind": "utf8", "value": "alpha" },
                    { "kind": "null", "value": "null" }
                ]
            }
        },
        "frame_right": {
            "index": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["a", "b"],
            "columns": {
                "a": [
                    { "kind": "utf8", "value": "beta" },
                    { "kind": "utf8", "value": "gamma" }
                ],
                "b": [
                    { "kind": "utf8", "value": "bee" },
                    { "kind": "utf8", "value": "cee" }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe combine_first oracle test: {message}"
        );
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

fn assert_live_oracle_dataframe_mode_parity(fixture: super::PacketFixture, context: &str) {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping {context}: {message}");
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
fn live_oracle_dataframe_mode_single_mode_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-057",
        "case_id": "dataframe_mode_single_mode_live",
        "mode": "strict",
        "operation": "dataframe_mode",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "column_order": ["a", "b"],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "b": [
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 6 },
                    { "kind": "int64", "value": 7 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_mode_parity(fixture, "dataframe mode single-mode oracle test");
}

#[test]
fn live_oracle_dataframe_mode_ties_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-057",
        "case_id": "dataframe_mode_ties_live",
        "mode": "strict",
        "operation": "dataframe_mode",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "column_order": ["a", "b"],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 2 }
                ],
                "b": [
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 4 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_mode_parity(fixture, "dataframe mode ties oracle test");
}

#[test]
fn live_oracle_series_to_datetime_unit_seconds_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-064",
        "case_id": "series_to_datetime_unit_seconds_live",
        "mode": "strict",
        "operation": "series_to_datetime",
        "oracle_source": "live_legacy_pandas",
        "datetime_unit": "s",
        "left": {
            "name": "epoch_s",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "utf8", "value": "bad" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping to_datetime unit seconds oracle test: {message}"
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
fn live_oracle_series_rank_pct_dense_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SERIES-RANK-PCT",
        "case_id": "series_rank_pct_dense_live",
        "mode": "strict",
        "operation": "series_rank",
        "oracle_source": "live_legacy_pandas",
        "rank_method": "dense",
        "rank_na_option": "keep",
        "rank_pct": true,
        "sort_ascending": true,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "na_n" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series rank pct oracle test: {message}");
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

    let actual = super::build_series(fixture.left.as_ref().expect("left"))
        .expect("series")
        .rank_with_pct(
            fixture.rank_method.as_deref().expect("rank_method"),
            super::resolve_sort_ascending(&fixture),
            fixture.rank_na_option.as_deref().expect("rank_na_option"),
            super::resolve_rank_pct(&fixture),
        )
        .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_rank_axis1_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-065",
        "case_id": "dataframe_rank_axis1_live",
        "mode": "strict",
        "operation": "dataframe_rank",
        "oracle_source": "live_legacy_pandas",
        "rank_axis": 1,
        "rank_method": "average",
        "rank_na_option": "keep",
        "sort_ascending": true,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["a", "b", "c"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 5.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 }
                ],
                "c": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 4.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dataframe rank axis=1 oracle test: {message}");
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
fn live_oracle_dataframe_rank_axis1_pct_dense_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DATAFRAME-RANK-PCT",
        "case_id": "dataframe_rank_axis1_pct_dense_live",
        "mode": "strict",
        "operation": "dataframe_rank",
        "oracle_source": "live_legacy_pandas",
        "rank_axis": 1,
        "rank_method": "dense",
        "rank_na_option": "keep",
        "rank_pct": true,
        "sort_ascending": true,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a", "b", "c"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "null", "value": "na_n" }
                ],
                "b": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 5.0 }
                ],
                "c": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 7.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe rank pct axis=1 oracle test: {message}"
        );
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
fn live_oracle_dataframe_shift_axis1_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-066",
        "case_id": "dataframe_shift_axis1_live",
        "mode": "strict",
        "operation": "dataframe_shift",
        "oracle_source": "live_legacy_pandas",
        "shift_periods": 1,
        "shift_axis": 1,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a", "b", "c"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 }
                ],
                "c": [
                    { "kind": "float64", "value": 100.0 },
                    { "kind": "float64", "value": 200.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe shift axis=1 oracle test: {message}"
        );
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

fn assert_live_oracle_dataframe_diff_parity(fixture: super::PacketFixture, context: &str) {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping {context}: {message}");
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
fn live_oracle_dataframe_diff_axis1_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-056",
        "case_id": "dataframe_diff_axis1_live",
        "mode": "strict",
        "operation": "dataframe_diff",
        "oracle_source": "live_legacy_pandas",
        "diff_periods": 1,
        "diff_axis": 1,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["a", "b", "c", "d"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": -2.0 },
                    { "kind": "float64", "value": 5.5 }
                ],
                "b": [
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": -2.0 },
                    { "kind": "float64", "value": 3.5 }
                ],
                "c": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 0.0 },
                    { "kind": "float64", "value": 3.5 }
                ],
                "d": [
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "float64", "value": 8.0 },
                    { "kind": "float64", "value": -1.5 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_diff_parity(fixture, "dataframe diff axis=1 oracle test");
}

#[test]
fn live_oracle_dataframe_diff_axis1_negative_periods_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-056",
        "case_id": "dataframe_diff_axis1_negative_periods_live",
        "mode": "strict",
        "operation": "dataframe_diff",
        "oracle_source": "live_legacy_pandas",
        "diff_periods": -1,
        "diff_axis": 1,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a", "b", "c", "d"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": -5.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 15.0 },
                    { "kind": "float64", "value": -1.0 }
                ],
                "c": [
                    { "kind": "float64", "value": 15.0 },
                    { "kind": "float64", "value": 4.0 }
                ],
                "d": [
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 4.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_diff_parity(
        fixture,
        "dataframe diff axis=1 negative-period oracle test",
    );
}

fn assert_live_oracle_dataframe_pct_change_parity(fixture: super::PacketFixture, context: &str) {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping {context}: {message}");
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
fn live_oracle_dataframe_pct_change_axis1_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-062",
        "case_id": "dataframe_pct_change_axis1_live",
        "mode": "strict",
        "operation": "dataframe_pct_change",
        "oracle_source": "live_legacy_pandas",
        "diff_periods": 1,
        "diff_axis": 1,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["a", "b", "c"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 100.0 },
                    { "kind": "float64", "value": 50.0 },
                    { "kind": "float64", "value": -20.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 110.0 },
                    { "kind": "float64", "value": 40.0 },
                    { "kind": "float64", "value": -10.0 }
                ],
                "c": [
                    { "kind": "float64", "value": 121.0 },
                    { "kind": "float64", "value": 80.0 },
                    { "kind": "float64", "value": -5.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_pct_change_parity(
        fixture,
        "dataframe pct_change axis=1 oracle test",
    );
}

#[test]
fn live_oracle_dataframe_pct_change_alias_fields_match_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-056",
        "case_id": "dataframe_pct_change_alias_fields_live",
        "mode": "strict",
        "operation": "dataframe_pct_change",
        "oracle_source": "live_legacy_pandas",
        "pct_change_periods": 2,
        "pct_change_axis": 1,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a", "b", "c", "d"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 10.0 }
                ],
                "c": [
                    { "kind": "float64", "value": 40.0 },
                    { "kind": "float64", "value": 30.0 }
                ],
                "d": [
                    { "kind": "float64", "value": 80.0 },
                    { "kind": "float64", "value": 15.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let actual = super::execute_dataframe_fixture_operation(&fixture).expect("actual frame");
    assert!(actual.column("a").unwrap().values()[0].is_missing());
    assert!(actual.column("b").unwrap().values()[1].is_missing());
    assert_eq!(
        actual.column("c").unwrap().values(),
        &[
            fp_types::Scalar::Float64(3.0),
            fp_types::Scalar::Float64(0.5)
        ]
    );
    assert_eq!(
        actual.column("d").unwrap().values(),
        &[
            fp_types::Scalar::Float64(3.0),
            fp_types::Scalar::Float64(0.5)
        ]
    );

    assert_live_oracle_dataframe_pct_change_parity(
        fixture,
        "dataframe pct_change alias-field oracle test",
    );
}

#[test]
fn live_oracle_dataframe_pct_change_axis1_negative_periods_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-062",
        "case_id": "dataframe_pct_change_axis1_negative_periods_live",
        "mode": "strict",
        "operation": "dataframe_pct_change",
        "oracle_source": "live_legacy_pandas",
        "diff_periods": -1,
        "diff_axis": 1,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["a", "b", "c", "d"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 100.0 },
                    { "kind": "float64", "value": 50.0 },
                    { "kind": "float64", "value": 10.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 120.0 },
                    { "kind": "float64", "value": 25.0 },
                    { "kind": "float64", "value": 20.0 }
                ],
                "c": [
                    { "kind": "float64", "value": 60.0 },
                    { "kind": "float64", "value": 100.0 },
                    { "kind": "float64", "value": 40.0 }
                ],
                "d": [
                    { "kind": "float64", "value": 30.0 },
                    { "kind": "float64", "value": 50.0 },
                    { "kind": "float64", "value": 80.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_pct_change_parity(
        fixture,
        "dataframe pct_change axis=1 negative-period oracle test",
    );
}

#[test]
fn live_oracle_dataframe_take_axis0_negative_indices_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-067",
        "case_id": "dataframe_take_axis0_negative_indices_live",
        "mode": "strict",
        "operation": "dataframe_take",
        "oracle_source": "live_legacy_pandas",
        "take_indices": [-1, -3],
        "take_axis": 0,
        "frame": {
            "index": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ],
            "column_order": ["a", "b"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 }
                ],
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" },
                    { "kind": "utf8", "value": "z" }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dataframe take axis=0 oracle test: {message}");
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
fn live_oracle_dataframe_take_axis1_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-067",
        "case_id": "dataframe_take_axis1_live",
        "mode": "strict",
        "operation": "dataframe_take",
        "oracle_source": "live_legacy_pandas",
        "take_indices": [1, 2],
        "take_axis": 1,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a", "b", "c"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 }
                ],
                "c": [
                    { "kind": "float64", "value": 100.0 },
                    { "kind": "float64", "value": 200.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dataframe take axis=1 oracle test: {message}");
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
fn live_oracle_series_first_valid_index_with_leading_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FIRST-VALID-INDEX",
        "case_id": "series_first_valid_index_leading_nulls",
        "mode": "strict",
        "operation": "series_first_valid_index",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.125 },
                { "kind": "float64", "value": 2.71 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping first_valid_index test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let label_opt = series.first_valid_index();
    let actual = super::optional_index_label_to_scalar(label_opt);
    super::compare_scalar(&actual, &expected, "series_first_valid_index").expect("pandas parity");
}

#[test]
fn live_oracle_series_last_valid_index_with_trailing_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-LAST-VALID-INDEX",
        "case_id": "series_last_valid_index_trailing_nulls",
        "mode": "strict",
        "operation": "series_last_valid_index",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping last_valid_index test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let label_opt = series.last_valid_index();
    let actual = super::optional_index_label_to_scalar(label_opt);
    super::compare_scalar(&actual, &expected, "series_last_valid_index").expect("pandas parity");
}

#[test]
fn live_oracle_series_first_valid_index_all_null_returns_none() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FIRST-VALID-INDEX-ALLNULL",
        "case_id": "series_first_valid_index_all_null",
        "mode": "strict",
        "operation": "series_first_valid_index",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "all_null",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping first_valid_index all_null test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let label_opt = series.first_valid_index();
    let actual = super::optional_index_label_to_scalar(label_opt);
    super::compare_scalar(&actual, &expected, "series_first_valid_index").expect("pandas parity");
}

#[test]
fn live_oracle_series_first_valid_index_string_index() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FIRST-VALID-INDEX-STR",
        "case_id": "series_first_valid_index_string_index",
        "mode": "strict",
        "operation": "series_first_valid_index",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "str_idx",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "int64", "value": 42 },
                { "kind": "int64", "value": 99 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping first_valid_index string_index test: {message}"
        );
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let label_opt = series.first_valid_index();
    let actual = super::optional_index_label_to_scalar(label_opt);
    super::compare_scalar(&actual, &expected, "series_first_valid_index").expect("pandas parity");
}

#[test]
fn live_oracle_series_idxmin_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-IDXMIN",
        "case_id": "series_idxmin_basic",
        "mode": "strict",
        "operation": "series_idxmin",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping idxmin test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let label = series.idxmin().expect("idxmin");
    let actual = super::index_label_to_scalar(&label);
    super::compare_scalar(&actual, &expected, "series_idxmin").expect("pandas parity");
}

#[test]
fn live_oracle_series_idxmax_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-IDXMAX",
        "case_id": "series_idxmax_basic",
        "mode": "strict",
        "operation": "series_idxmax",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping idxmax test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let label = series.idxmax().expect("idxmax");
    let actual = super::index_label_to_scalar(&label);
    super::compare_scalar(&actual, &expected, "series_idxmax").expect("pandas parity");
}

#[test]
fn live_oracle_series_idxmin_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-IDXMIN-NULLS",
        "case_id": "series_idxmin_with_nulls",
        "mode": "strict",
        "operation": "series_idxmin",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping idxmin with nulls test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let label = series.idxmin().expect("idxmin");
    let actual = super::index_label_to_scalar(&label);
    super::compare_scalar(&actual, &expected, "series_idxmin").expect("pandas parity");
}

#[test]
fn live_oracle_series_idxmax_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-IDXMAX-NULLS",
        "case_id": "series_idxmax_with_nulls",
        "mode": "strict",
        "operation": "series_idxmax",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping idxmax with nulls test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let label = series.idxmax().expect("idxmax");
    let actual = super::index_label_to_scalar(&label);
    super::compare_scalar(&actual, &expected, "series_idxmax").expect("pandas parity");
}

fn assert_series_idx_skipna_false_nan_fixture(fixture: &super::PacketFixture, context: &str) {
    let actual = super::execute_series_idx_fixture_scalar(fixture).expect("actual idx scalar");
    super::compare_scalar(
        &actual,
        &fp_types::Scalar::Null(fp_types::NullKind::NaN),
        fixture.operation.operation_name(),
    )
    .expect("skipna=false with missing values should produce NaN");

    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;
    let expected_result = super::capture_live_oracle_expected(&cfg, fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping {context}: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };
    super::compare_scalar(&actual, &expected, fixture.operation.operation_name())
        .expect("pandas parity");
}

#[test]
fn live_oracle_series_idxmin_skipna_false_with_null_returns_nan() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-IDXMIN-SKIPNA-FALSE",
        "case_id": "series_idxmin_skipna_false_with_null",
        "mode": "strict",
        "operation": "series_idxmin",
        "oracle_source": "live_legacy_pandas",
        "idxmin_skipna": false,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 2.0 }
            ]
        }
    }))
    .expect("fixture");

    assert_series_idx_skipna_false_nan_fixture(
        &fixture,
        "idxmin skipna=false with-null oracle test",
    );
}

#[test]
fn live_oracle_series_idxmax_skipna_false_with_null_returns_nan() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-IDXMAX-SKIPNA-FALSE",
        "case_id": "series_idxmax_skipna_false_with_null",
        "mode": "strict",
        "operation": "series_idxmax",
        "oracle_source": "live_legacy_pandas",
        "idxmax_skipna": false,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": [
                { "kind": "float64", "value": 5.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 2.0 }
            ]
        }
    }))
    .expect("fixture");

    assert_series_idx_skipna_false_nan_fixture(
        &fixture,
        "idxmax skipna=false with-null oracle test",
    );
}

fn assert_series_mode_dropna_false_fixture(
    fixture: &super::PacketFixture,
    context: &str,
    run_differential: bool,
) {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;

    let expected_result = super::capture_live_oracle_expected(&cfg, fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping {context}: {message}");
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

    let actual = super::build_series(fixture.left.as_ref().expect("left"))
        .expect("series")
        .mode_with_dropna(fixture.mode_dropna.unwrap_or(true))
        .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");

    if run_differential {
        let diff = super::run_differential_fixture(
            &cfg,
            fixture,
            &super::SuiteOptions {
                packet_filter: None,
                oracle_mode: super::OracleMode::LiveLegacyPandas,
            },
        )
        .expect("differential report");
        assert_eq!(diff.status, super::CaseStatus::Pass, "{diff:?}");
        assert!(
            diff.drift_records.is_empty(),
            "expected no mode(dropna=false) drift: {diff:?}"
        );
    }
}

#[test]
fn live_oracle_series_mode_dropna_false_counts_nan_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SERIES-MODE-DROPNA-FALSE",
        "case_id": "series_mode_dropna_false_counts_nan_live",
        "mode": "strict",
        "operation": "series_mode",
        "oracle_source": "live_legacy_pandas",
        "mode_dropna": false,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 4 },
                { "kind": "null", "value": "na_n" },
                { "kind": "null", "value": "na_n" },
                { "kind": "int64", "value": 4 },
                { "kind": "null", "value": "na_n" }
            ]
        }
    }))
    .expect("fixture");

    assert_series_mode_dropna_false_fixture(
        &fixture,
        "series mode dropna=false missing-value mode oracle test",
        true,
    );
}

#[test]
fn live_oracle_series_mode_dropna_false_tie_sorts_nan_last_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SERIES-MODE-DROPNA-FALSE",
        "case_id": "series_mode_dropna_false_tie_nan_last_live",
        "mode": "strict",
        "operation": "series_mode",
        "oracle_source": "live_legacy_pandas",
        "mode_dropna": false,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 1 },
                { "kind": "null", "value": "na_n" },
                { "kind": "null", "value": "na_n" }
            ]
        }
    }))
    .expect("fixture");

    assert_series_mode_dropna_false_fixture(
        &fixture,
        "series mode dropna=false tied missing-value oracle test",
        false,
    );
}

#[test]
fn live_oracle_series_argsort_ascending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ARGSORT-ASC",
        "case_id": "series_argsort_ascending",
        "mode": "strict",
        "operation": "series_argsort",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 2.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping argsort ascending test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.argsort(true).expect("argsort");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_argsort_descending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ARGSORT-DESC",
        "case_id": "series_argsort_descending",
        "mode": "strict",
        "operation": "series_argsort",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": false,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 2.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping argsort descending test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.argsort(false).expect("argsort");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_argsort_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ARGSORT-NULLS",
        "case_id": "series_argsort_with_nulls",
        "mode": "strict",
        "operation": "series_argsort",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping argsort with nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.argsort(true).expect("argsort");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_argsort_integers() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ARGSORT-INT",
        "case_id": "series_argsort_integers",
        "mode": "strict",
        "operation": "series_argsort",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "left": {
            "name": "int_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": -5 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping argsort integers test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.argsort(true).expect("argsort");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_argmin_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ARGMIN",
        "case_id": "series_argmin_basic",
        "mode": "strict",
        "operation": "series_argmin",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping argmin test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let idx = series.argmin().expect("argmin");
    let actual = super::Scalar::Int64(idx);
    super::compare_scalar(&actual, &expected, "series_argmin").expect("pandas parity");
}

#[test]
fn live_oracle_series_argmax_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ARGMAX",
        "case_id": "series_argmax_basic",
        "mode": "strict",
        "operation": "series_argmax",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping argmax test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let idx = series.argmax().expect("argmax");
    let actual = super::Scalar::Int64(idx);
    super::compare_scalar(&actual, &expected, "series_argmax").expect("pandas parity");
}

#[test]
fn live_oracle_series_argmin_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ARGMIN-NULL",
        "case_id": "series_argmin_with_nulls",
        "mode": "strict",
        "operation": "series_argmin",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping argmin nulls test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let idx = series.argmin().expect("argmin");
    let actual = super::Scalar::Int64(idx);
    super::compare_scalar(&actual, &expected, "series_argmin").expect("pandas parity");
}

#[test]
fn live_oracle_series_argmax_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ARGMAX-NULL",
        "case_id": "series_argmax_with_nulls",
        "mode": "strict",
        "operation": "series_argmax",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping argmax nulls test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let idx = series.argmax().expect("argmax");
    let actual = super::Scalar::Int64(idx);
    super::compare_scalar(&actual, &expected, "series_argmax").expect("pandas parity");
}

#[test]
fn live_oracle_series_searchsorted_left() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SEARCHSORTED-L",
        "case_id": "series_searchsorted_left",
        "mode": "strict",
        "operation": "series_searchsorted",
        "oracle_source": "live_legacy_pandas",
        "searchsorted_value": { "kind": "float64", "value": 2.5 },
        "searchsorted_side": "left",
        "left": {
            "name": "sorted_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping searchsorted left test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let needle = fixture.searchsorted_value.as_ref().expect("needle");
    let pos = series.searchsorted(needle, "left").expect("searchsorted");
    let actual = super::Scalar::Int64(pos as i64);
    super::compare_scalar(&actual, &expected, "series_searchsorted").expect("pandas parity");
}

#[test]
fn live_oracle_series_searchsorted_right() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SEARCHSORTED-R",
        "case_id": "series_searchsorted_right",
        "mode": "strict",
        "operation": "series_searchsorted",
        "oracle_source": "live_legacy_pandas",
        "searchsorted_value": { "kind": "float64", "value": 2.0 },
        "searchsorted_side": "right",
        "left": {
            "name": "sorted_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping searchsorted right test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let needle = fixture.searchsorted_value.as_ref().expect("needle");
    let pos = series.searchsorted(needle, "right").expect("searchsorted");
    let actual = super::Scalar::Int64(pos as i64);
    super::compare_scalar(&actual, &expected, "series_searchsorted").expect("pandas parity");
}

#[test]
fn live_oracle_series_searchsorted_exact_match() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SEARCHSORTED-EXACT",
        "case_id": "series_searchsorted_exact_match",
        "mode": "strict",
        "operation": "series_searchsorted",
        "oracle_source": "live_legacy_pandas",
        "searchsorted_value": { "kind": "float64", "value": 3.0 },
        "searchsorted_side": "left",
        "left": {
            "name": "sorted_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping searchsorted exact test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let needle = fixture.searchsorted_value.as_ref().expect("needle");
    let pos = series.searchsorted(needle, "left").expect("searchsorted");
    let actual = super::Scalar::Int64(pos as i64);
    super::compare_scalar(&actual, &expected, "series_searchsorted").expect("pandas parity");
}

#[test]
fn live_oracle_series_searchsorted_integers() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SEARCHSORTED-INT",
        "case_id": "series_searchsorted_integers",
        "mode": "strict",
        "operation": "series_searchsorted",
        "oracle_source": "live_legacy_pandas",
        "searchsorted_value": { "kind": "int64", "value": 25 },
        "searchsorted_side": "left",
        "left": {
            "name": "int_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 40 },
                { "kind": "int64", "value": 50 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping searchsorted integers test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let needle = fixture.searchsorted_value.as_ref().expect("needle");
    let pos = series.searchsorted(needle, "left").expect("searchsorted");
    let actual = super::Scalar::Int64(pos as i64);
    super::compare_scalar(&actual, &expected, "series_searchsorted").expect("pandas parity");
}

#[test]
fn live_oracle_series_dot_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DOT-BASIC",
        "case_id": "series_dot_basic",
        "mode": "strict",
        "operation": "series_dot",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "a",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 }
            ]
        },
        "right": {
            "name": "b",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 1.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dot basic test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let left = super::build_series(fixture.left.as_ref().expect("left")).expect("left");
    let right = super::build_series(fixture.right.as_ref().expect("right")).expect("right");
    let result = left.dot(&right).expect("dot");
    let actual = super::Scalar::Float64(result);
    super::compare_scalar(&actual, &expected, "series_dot").expect("pandas parity");
}

#[test]
fn live_oracle_series_dot_integers() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DOT-INT",
        "case_id": "series_dot_integers",
        "mode": "strict",
        "operation": "series_dot",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "a",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ]
        },
        "right": {
            "name": "b",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 6 },
                { "kind": "int64", "value": 7 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dot integers test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let left = super::build_series(fixture.left.as_ref().expect("left")).expect("left");
    let right = super::build_series(fixture.right.as_ref().expect("right")).expect("right");
    let result = left.dot(&right).expect("dot");
    let actual = super::Scalar::Float64(result);
    super::compare_scalar(&actual, &expected, "series_dot").expect("pandas parity");
}

#[test]
fn live_oracle_series_dot_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DOT-NULL",
        "case_id": "series_dot_with_nulls",
        "mode": "strict",
        "operation": "series_dot",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "a",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 }
            ]
        },
        "right": {
            "name": "b",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 2.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dot nulls test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let left = super::build_series(fixture.left.as_ref().expect("left")).expect("left");
    let right = super::build_series(fixture.right.as_ref().expect("right")).expect("right");
    let result = left.dot(&right).expect("dot");
    let actual = super::Scalar::Float64(result);
    super::compare_scalar(&actual, &expected, "series_dot").expect("pandas parity");
}

#[test]
fn live_oracle_series_dot_negative_values() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DOT-NEG",
        "case_id": "series_dot_negative",
        "mode": "strict",
        "operation": "series_dot",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "a",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "float64", "value": -1.5 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": -3.5 }
            ]
        },
        "right": {
            "name": "b",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": -2.0 },
                { "kind": "float64", "value": 1.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dot negative test: {message}");
        return;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let left = super::build_series(fixture.left.as_ref().expect("left")).expect("left");
    let right = super::build_series(fixture.right.as_ref().expect("right")).expect("right");
    let result = left.dot(&right).expect("dot");
    let actual = super::Scalar::Float64(result);
    super::compare_scalar(&actual, &expected, "series_dot").expect("pandas parity");
}

#[test]
fn live_oracle_series_nlargest_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NLARGEST-BASIC",
        "case_id": "series_nlargest_basic",
        "mode": "strict",
        "operation": "series_nlargest",
        "oracle_source": "live_legacy_pandas",
        "nlargest_n": 3,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" },
                { "kind": "utf8", "value": "e" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 4.5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping nlargest basic test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.nlargest(3).expect("nlargest");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_nlargest_with_keep_last() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NLARGEST-KEEP-LAST",
        "case_id": "series_nlargest_keep_last",
        "mode": "strict",
        "operation": "series_nlargest",
        "oracle_source": "live_legacy_pandas",
        "nlargest_n": 2,
        "keep": "last",
        "left": {
            "name": "dup_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 3.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping nlargest keep=last test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.nlargest_keep(2, "last").expect("nlargest_keep");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_nsmallest_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NSMALLEST-BASIC",
        "case_id": "series_nsmallest_basic",
        "mode": "strict",
        "operation": "series_nsmallest",
        "oracle_source": "live_legacy_pandas",
        "nlargest_n": 3,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" },
                { "kind": "utf8", "value": "e" }
            ],
            "values": [
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 4.5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping nsmallest basic test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.nsmallest(3).expect("nsmallest");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_nsmallest_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NSMALLEST-NULLS",
        "case_id": "series_nsmallest_with_nulls",
        "mode": "strict",
        "operation": "series_nsmallest",
        "oracle_source": "live_legacy_pandas",
        "nlargest_n": 2,
        "left": {
            "name": "null_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping nsmallest nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.nsmallest(2).expect("nsmallest");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_describe_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DESCRIBE-BASIC",
        "case_id": "series_describe_basic",
        "mode": "strict",
        "operation": "series_describe",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping describe basic test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.describe().expect("describe");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_describe_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DESCRIBE-NULLS",
        "case_id": "series_describe_with_nulls",
        "mode": "strict",
        "operation": "series_describe",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "null_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 20.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 30.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping describe nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.describe().expect("describe");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_describe_integers() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DESCRIBE-INTS",
        "case_id": "series_describe_integers",
        "mode": "strict",
        "operation": "series_describe",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "int_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "int64", "value": 100 },
                { "kind": "int64", "value": 200 },
                { "kind": "int64", "value": 300 },
                { "kind": "int64", "value": 400 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping describe integers test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.describe().expect("describe");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_describe_single_value() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DESCRIBE-SINGLE",
        "case_id": "series_describe_single_value",
        "mode": "strict",
        "operation": "series_describe",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "single_series",
            "index": [
                { "kind": "int64", "value": 0 }
            ],
            "values": [
                { "kind": "float64", "value": 42.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping describe single value test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.describe().expect("describe");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_between_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-BETWEEN-BASIC",
        "case_id": "series_between_basic",
        "mode": "strict",
        "operation": "series_between",
        "oracle_source": "live_legacy_pandas",
        "between_left": { "kind": "float64", "value": 2.0 },
        "between_right": { "kind": "float64", "value": 4.0 },
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping between basic test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .between(
            &super::Scalar::Float64(2.0),
            &super::Scalar::Float64(4.0),
            "both",
        )
        .expect("between");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_between_inclusive_neither() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-BETWEEN-NEITHER",
        "case_id": "series_between_inclusive_neither",
        "mode": "strict",
        "operation": "series_between",
        "oracle_source": "live_legacy_pandas",
        "between_left": { "kind": "float64", "value": 2.0 },
        "between_right": { "kind": "float64", "value": 4.0 },
        "between_inclusive": "neither",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping between neither test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .between(
            &super::Scalar::Float64(2.0),
            &super::Scalar::Float64(4.0),
            "neither",
        )
        .expect("between");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_between_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-BETWEEN-NULLS",
        "case_id": "series_between_with_nulls",
        "mode": "strict",
        "operation": "series_between",
        "oracle_source": "live_legacy_pandas",
        "between_left": { "kind": "float64", "value": 0.0 },
        "between_right": { "kind": "float64", "value": 10.0 },
        "left": {
            "name": "null_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 15.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping between nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .between(
            &super::Scalar::Float64(0.0),
            &super::Scalar::Float64(10.0),
            "both",
        )
        .expect("between");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_between_integers() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-BETWEEN-INTS",
        "case_id": "series_between_integers",
        "mode": "strict",
        "operation": "series_between",
        "oracle_source": "live_legacy_pandas",
        "between_left": { "kind": "int64", "value": 10 },
        "between_right": { "kind": "int64", "value": 30 },
        "between_inclusive": "left",
        "left": {
            "name": "int_series",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping between integers test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .between(&super::Scalar::Int64(10), &super::Scalar::Int64(30), "left")
        .expect("between");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_duplicated_keep_first() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DUPLICATED-FIRST",
        "case_id": "series_duplicated_keep_first",
        "mode": "strict",
        "operation": "series_duplicated",
        "oracle_source": "live_legacy_pandas",
        "keep": "first",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 2 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping duplicated first test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .duplicated_keep(super::DuplicateKeep::First)
        .expect("duplicated");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_duplicated_keep_last() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DUPLICATED-LAST",
        "case_id": "series_duplicated_keep_last",
        "mode": "strict",
        "operation": "series_duplicated",
        "oracle_source": "live_legacy_pandas",
        "keep": "last",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 2 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping duplicated last test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .duplicated_keep(super::DuplicateKeep::Last)
        .expect("duplicated");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_duplicated_keep_none() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DUPLICATED-NONE",
        "case_id": "series_duplicated_keep_none",
        "mode": "strict",
        "operation": "series_duplicated",
        "oracle_source": "live_legacy_pandas",
        "keep": "none",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 2 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping duplicated none test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .duplicated_keep(super::DuplicateKeep::None)
        .expect("duplicated");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_duplicated_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DUPLICATED-NULLS",
        "case_id": "series_duplicated_with_nulls",
        "mode": "strict",
        "operation": "series_duplicated",
        "oracle_source": "live_legacy_pandas",
        "keep": "first",
        "left": {
            "name": "null_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping duplicated nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .duplicated_keep(super::DuplicateKeep::First)
        .expect("duplicated");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_cumsum_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CUMSUM-INT",
        "case_id": "series_cumsum_int",
        "mode": "strict",
        "operation": "series_cumsum",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping cumsum int test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.cumsum().expect("cumsum");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_cumsum_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CUMSUM-NULLS",
        "case_id": "series_cumsum_with_nulls",
        "mode": "strict",
        "operation": "series_cumsum",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping cumsum nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.cumsum().expect("cumsum");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_cumprod_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CUMPROD-INT",
        "case_id": "series_cumprod_int",
        "mode": "strict",
        "operation": "series_cumprod",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
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
        eprintln!("live pandas unavailable; skipping cumprod int test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.cumprod().expect("cumprod");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_cumprod_float() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CUMPROD-FLOAT",
        "case_id": "series_cumprod_float",
        "mode": "strict",
        "operation": "series_cumprod",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping cumprod float test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.cumprod().expect("cumprod");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_cummax_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CUMMAX-INT",
        "case_id": "series_cummax_int",
        "mode": "strict",
        "operation": "series_cummax",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping cummax int test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.cummax().expect("cummax");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_cummax_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CUMMAX-NULLS",
        "case_id": "series_cummax_with_nulls",
        "mode": "strict",
        "operation": "series_cummax",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping cummax nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.cummax().expect("cummax");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_cummin_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CUMMIN-INT",
        "case_id": "series_cummin_int",
        "mode": "strict",
        "operation": "series_cummin",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 7 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 8 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping cummin int test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.cummin().expect("cummin");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_cummin_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CUMMIN-NULLS",
        "case_id": "series_cummin_with_nulls",
        "mode": "strict",
        "operation": "series_cummin",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 4.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 6.0 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping cummin nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.cummin().expect("cummin");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_drop_duplicates_keep_first() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DROP-DUP-FIRST",
        "case_id": "series_drop_duplicates_keep_first",
        "mode": "strict",
        "operation": "series_drop_duplicates",
        "oracle_source": "live_legacy_pandas",
        "keep": "first",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 4 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping drop_duplicates first test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .drop_duplicates_keep(super::DuplicateKeep::First)
        .expect("drop_duplicates");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_drop_duplicates_keep_last() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DROP-DUP-LAST",
        "case_id": "series_drop_duplicates_keep_last",
        "mode": "strict",
        "operation": "series_drop_duplicates",
        "oracle_source": "live_legacy_pandas",
        "keep": "last",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 4 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping drop_duplicates last test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .drop_duplicates_keep(super::DuplicateKeep::Last)
        .expect("drop_duplicates");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_drop_duplicates_keep_none() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DROP-DUP-NONE",
        "case_id": "series_drop_duplicates_keep_none",
        "mode": "strict",
        "operation": "series_drop_duplicates",
        "oracle_source": "live_legacy_pandas",
        "keep": "none",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 4 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping drop_duplicates none test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .drop_duplicates_keep(super::DuplicateKeep::None)
        .expect("drop_duplicates");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_drop_duplicates_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DROP-DUP-NULLS",
        "case_id": "series_drop_duplicates_with_nulls",
        "mode": "strict",
        "operation": "series_drop_duplicates",
        "oracle_source": "live_legacy_pandas",
        "keep": "first",
        "left": {
            "name": "null_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping drop_duplicates nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series
        .drop_duplicates_keep(super::DuplicateKeep::First)
        .expect("drop_duplicates");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_abs_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ABS-INT",
        "case_id": "series_abs_int",
        "mode": "strict",
        "operation": "series_abs",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": -5 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": -1 },
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": -100 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping abs int test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.abs().expect("abs");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_abs_float() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ABS-FLOAT",
        "case_id": "series_abs_float",
        "mode": "strict",
        "operation": "series_abs",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": -3.125 },
                { "kind": "float64", "value": 2.71 },
                { "kind": "float64", "value": -0.0 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping abs float test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.abs().expect("abs");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_round_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ROUND-DEFAULT",
        "case_id": "series_round_default",
        "mode": "strict",
        "operation": "series_round",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.4 },
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": -1.5 },
                { "kind": "float64", "value": 3.6 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping round default test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.round(0).expect("round");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_round_decimals_2() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ROUND-DEC2",
        "case_id": "series_round_decimals_2",
        "mode": "strict",
        "operation": "series_round",
        "oracle_source": "live_legacy_pandas",
        "round_decimals": 2,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.234 },
                { "kind": "float64", "value": 1.235 },
                { "kind": "float64", "value": -3.15159 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping round decimals 2 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.round(2).expect("round");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_round_negative_decimals() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ROUND-NEG",
        "case_id": "series_round_negative_decimals",
        "mode": "strict",
        "operation": "series_round",
        "oracle_source": "live_legacy_pandas",
        "round_decimals": -1,
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 14.0 },
                { "kind": "float64", "value": 15.0 },
                { "kind": "float64", "value": 25.0 },
                { "kind": "float64", "value": 123.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping round negative decimals test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.round(-1).expect("round");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_replace_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-REPLACE-INT",
        "case_id": "series_replace_int",
        "mode": "strict",
        "operation": "series_replace",
        "oracle_source": "live_legacy_pandas",
        "replace_to_find": [
            { "kind": "int64", "value": 1 },
            { "kind": "int64", "value": 3 }
        ],
        "replace_to_value": [
            { "kind": "int64", "value": 100 },
            { "kind": "int64", "value": 300 }
        ],
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 1 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping replace int test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let replacements: Vec<(super::Scalar, super::Scalar)> = fixture
        .replace_to_find
        .as_ref()
        .expect("to_find")
        .iter()
        .zip(fixture.replace_to_value.as_ref().expect("to_value").iter())
        .map(|(f, v)| (f.clone(), v.clone()))
        .collect();
    let actual = series.replace(&replacements).expect("replace");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_replace_with_null() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-REPLACE-NULL",
        "case_id": "series_replace_with_null",
        "mode": "strict",
        "operation": "series_replace",
        "oracle_source": "live_legacy_pandas",
        "replace_to_find": [
            { "kind": "int64", "value": 2 }
        ],
        "replace_to_value": [
            { "kind": "null", "value": "null" }
        ],
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 2 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping replace with null test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let replacements: Vec<(super::Scalar, super::Scalar)> = fixture
        .replace_to_find
        .as_ref()
        .expect("to_find")
        .iter()
        .zip(fixture.replace_to_value.as_ref().expect("to_value").iter())
        .map(|(f, v)| (f.clone(), v.clone()))
        .collect();
    let actual = series.replace(&replacements).expect("replace");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_replace_float() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-REPLACE-FLOAT",
        "case_id": "series_replace_float",
        "mode": "strict",
        "operation": "series_replace",
        "oracle_source": "live_legacy_pandas",
        "replace_to_find": [
            { "kind": "float64", "value": 1.5 },
            { "kind": "float64", "value": 3.5 }
        ],
        "replace_to_value": [
            { "kind": "float64", "value": 10.5 },
            { "kind": "float64", "value": 30.5 }
        ],
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 4.5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping replace float test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let replacements: Vec<(super::Scalar, super::Scalar)> = fixture
        .replace_to_find
        .as_ref()
        .expect("to_find")
        .iter()
        .zip(fixture.replace_to_value.as_ref().expect("to_value").iter())
        .map(|(f, v)| (f.clone(), v.clone()))
        .collect();
    let actual = series.replace(&replacements).expect("replace");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_replace_no_match() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-REPLACE-NOMATCH",
        "case_id": "series_replace_no_match",
        "mode": "strict",
        "operation": "series_replace",
        "oracle_source": "live_legacy_pandas",
        "replace_to_find": [
            { "kind": "int64", "value": 99 }
        ],
        "replace_to_value": [
            { "kind": "int64", "value": 999 }
        ],
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
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
        eprintln!("live pandas unavailable; skipping replace no match test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let replacements: Vec<(super::Scalar, super::Scalar)> = fixture
        .replace_to_find
        .as_ref()
        .expect("to_find")
        .iter()
        .zip(fixture.replace_to_value.as_ref().expect("to_value").iter())
        .map(|(f, v)| (f.clone(), v.clone()))
        .collect();
    let actual = series.replace(&replacements).expect("replace");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_unique_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-UNIQUE",
        "case_id": "series_unique_basic",
        "mode": "strict",
        "operation": "series_unique",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 2 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping unique test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let unique_values = series.unique();
    let unique_series = super::Series::from_values(
        "unique".to_owned(),
        (0..unique_values.len())
            .map(|i| super::IndexLabel::Int64(i as i64))
            .collect(),
        unique_values,
    )
    .expect("unique series");
    super::compare_series_expected(&unique_series, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_unique_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-UNIQUE-NULLS",
        "case_id": "series_unique_with_nulls",
        "mode": "strict",
        "operation": "series_unique",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping unique with nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let unique_values = series.unique();
    let unique_series = super::Series::from_values(
        "unique".to_owned(),
        (0..unique_values.len())
            .map(|i| super::IndexLabel::Int64(i as i64))
            .collect(),
        unique_values,
    )
    .expect("unique series");
    super::compare_series_expected(&unique_series, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_unique_floats() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-UNIQUE-FLOATS",
        "case_id": "series_unique_floats",
        "mode": "strict",
        "operation": "series_unique",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "float_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 3.5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping unique floats test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let unique_values = series.unique();
    let unique_series = super::Series::from_values(
        "unique".to_owned(),
        (0..unique_values.len())
            .map(|i| super::IndexLabel::Int64(i as i64))
            .collect(),
        unique_values,
    )
    .expect("unique series");
    super::compare_series_expected(&unique_series, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_factorize_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FACTORIZE",
        "case_id": "series_factorize_basic",
        "mode": "strict",
        "operation": "series_factorize",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "b" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping factorize test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let (codes, _uniques) = series.factorize().expect("factorize");
    super::compare_series_expected(&codes, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_factorize_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FACTORIZE-NULLS",
        "case_id": "series_factorize_with_nulls",
        "mode": "strict",
        "operation": "series_factorize",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "test_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "null", "value": "null" },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 10 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping factorize with nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let (codes, _uniques) = series.factorize().expect("factorize");
    super::compare_series_expected(&codes, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_factorize_ints() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FACTORIZE-INTS",
        "case_id": "series_factorize_ints",
        "mode": "strict",
        "operation": "series_factorize",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "int_series",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "int64", "value": 100 },
                { "kind": "int64", "value": 200 },
                { "kind": "int64", "value": 100 },
                { "kind": "int64", "value": 300 },
                { "kind": "int64", "value": 200 },
                { "kind": "int64", "value": 100 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping factorize ints test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let (codes, _uniques) = series.factorize().expect("factorize");
    super::compare_series_expected(&codes, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_astype_int_to_float() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ASTYPE-INT-TO-FLOAT",
        "case_id": "series_astype_int_to_float",
        "mode": "strict",
        "operation": "series_astype",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "nums",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ]
        },
        "dtype": "float64"
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping astype int to float test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.astype(fp_types::DType::Float64).expect("astype");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_astype_float_to_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ASTYPE-FLOAT-TO-INT",
        "case_id": "series_astype_float_to_int",
        "mode": "strict",
        "operation": "series_astype",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "floats",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 3.9 }
            ]
        },
        "dtype": "int64"
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping astype float to int test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.astype(fp_types::DType::Int64).expect("astype");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_astype_int_to_string() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ASTYPE-INT-TO-STR",
        "case_id": "series_astype_int_to_string",
        "mode": "strict",
        "operation": "series_astype",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "ints",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "int64", "value": 42 },
                { "kind": "int64", "value": -7 }
            ]
        },
        "dtype": "str"
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping astype int to string test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.astype(fp_types::DType::Utf8).expect("astype");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_astype_bool_to_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ASTYPE-BOOL-TO-INT",
        "case_id": "series_astype_bool_to_int",
        "mode": "strict",
        "operation": "series_astype",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "flags",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "bool", "value": true },
                { "kind": "bool", "value": false },
                { "kind": "bool", "value": true }
            ]
        },
        "dtype": "int64"
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping astype bool to int test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.astype(fp_types::DType::Int64).expect("astype");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_astype_int_to_bool() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ASTYPE-INT-TO-BOOL",
        "case_id": "series_astype_int_to_bool",
        "mode": "strict",
        "operation": "series_astype",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "ints",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": -5 }
            ]
        },
        "dtype": "bool"
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping astype int to bool test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.astype(fp_types::DType::Bool).expect("astype");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_astype_float_to_string() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ASTYPE-FLOAT-TO-STR",
        "case_id": "series_astype_float_to_string",
        "mode": "strict",
        "operation": "series_astype",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "floats",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": -4.25 }
            ]
        },
        "dtype": "str"
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping astype float to string test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.astype(fp_types::DType::Utf8).expect("astype");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_clip_both_bounds() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CLIP-BOTH",
        "case_id": "series_clip_both_bounds",
        "mode": "strict",
        "operation": "series_clip",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "nums",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": -5.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 7.0 },
                { "kind": "float64", "value": 15.0 },
                { "kind": "float64", "value": 3.5 }
            ]
        },
        "clip_lower": 0.0,
        "clip_upper": 10.0
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping clip both bounds test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.clip(Some(0.0), Some(10.0)).expect("clip");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_clip_lower_only() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CLIP-LOWER",
        "case_id": "series_clip_lower_only",
        "mode": "strict",
        "operation": "series_clip",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "float64", "value": -10.0 },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 100.0 }
            ]
        },
        "clip_lower": 0.0
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping clip lower only test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.clip(Some(0.0), None).expect("clip");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_clip_upper_only() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CLIP-UPPER",
        "case_id": "series_clip_upper_only",
        "mode": "strict",
        "operation": "series_clip",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "float64", "value": -10.0 },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 100.0 }
            ]
        },
        "clip_upper": 50.0
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping clip upper only test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.clip(None, Some(50.0)).expect("clip");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_clip_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CLIP-NULLS",
        "case_id": "series_clip_with_nulls",
        "mode": "strict",
        "operation": "series_clip",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "sparse",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": -5.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 25.0 },
                { "kind": "null", "value": "null" }
            ]
        },
        "clip_lower": 0.0,
        "clip_upper": 10.0
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping clip with nulls test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.clip(Some(0.0), Some(10.0)).expect("clip");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_clip_negative_bounds() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-CLIP-NEG",
        "case_id": "series_clip_negative_bounds",
        "mode": "strict",
        "operation": "series_clip",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "temps",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": -100.0 },
                { "kind": "float64", "value": -15.0 },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 50.0 }
            ]
        },
        "clip_lower": -20.0,
        "clip_upper": -5.0
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping clip negative bounds test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.clip(Some(-20.0), Some(-5.0)).expect("clip");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_fillna_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FILLNA-BASIC",
        "case_id": "series_fillna_basic",
        "mode": "strict",
        "operation": "series_fillna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "sparse",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" }
            ]
        },
        "fill_value": { "kind": "float64", "value": 0.0 }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping fillna basic test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series
        .fillna(&fp_types::Scalar::Float64(0.0))
        .expect("fillna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_fillna_no_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FILLNA-NONULLS",
        "case_id": "series_fillna_no_nulls",
        "mode": "strict",
        "operation": "series_fillna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "complete",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 }
            ]
        },
        "fill_value": { "kind": "float64", "value": 99.0 }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping fillna no nulls test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series
        .fillna(&fp_types::Scalar::Float64(99.0))
        .expect("fillna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_fillna_all_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FILLNA-ALLNULLS",
        "case_id": "series_fillna_all_nulls",
        "mode": "strict",
        "operation": "series_fillna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "empty",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" }
            ]
        },
        "fill_value": { "kind": "float64", "value": -1.0 }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping fillna all nulls test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series
        .fillna(&fp_types::Scalar::Float64(-1.0))
        .expect("fillna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_fillna_int_with_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FILLNA-INT",
        "case_id": "series_fillna_int_with_int",
        "mode": "strict",
        "operation": "series_fillna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "ints",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "null", "value": "null" },
                { "kind": "int64", "value": 30 },
                { "kind": "null", "value": "null" }
            ]
        },
        "fill_value": { "kind": "int64", "value": 0 }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping fillna int with int test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.fillna(&fp_types::Scalar::Int64(0)).expect("fillna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_dropna_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DROPNA-BASIC",
        "case_id": "series_dropna_basic",
        "mode": "strict",
        "operation": "series_dropna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "sparse",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dropna basic test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.dropna().expect("dropna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_dropna_no_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DROPNA-NONULLS",
        "case_id": "series_dropna_no_nulls",
        "mode": "strict",
        "operation": "series_dropna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "complete",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dropna no nulls test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.dropna().expect("dropna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_dropna_all_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DROPNA-ALLNULLS",
        "case_id": "series_dropna_all_nulls",
        "mode": "strict",
        "operation": "series_dropna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "empty",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dropna all nulls test: {message}");
        return;
    }
    assert!(
        expected_result.is_ok(),
        "live oracle expected: {expected_result:?}"
    );
    let expected = match expected_result {
        Ok(expected) => expected,
        Err(super::HarnessError::OracleUnavailable(_)) => return,
        Err(_) => return,
    };
    assert!(
        matches!(&expected, super::ResolvedExpected::Series(_)),
        "expected live oracle series payload, got {expected:?}"
    );
    let super::ResolvedExpected::Series(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.dropna().expect("dropna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_count_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-COUNT-BASIC",
        "case_id": "series_count_basic",
        "mode": "strict",
        "operation": "series_count",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "mixed",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping count basic test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = fp_types::Scalar::Int64(series.count() as i64);
    super::compare_scalar(&actual, &expected, "series_count").expect("pandas parity");
}

#[test]
fn live_oracle_series_count_no_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-COUNT-NONULLS",
        "case_id": "series_count_no_nulls",
        "mode": "strict",
        "operation": "series_count",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "complete",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping count no nulls test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = fp_types::Scalar::Int64(series.count() as i64);
    super::compare_scalar(&actual, &expected, "series_count").expect("pandas parity");
}

#[test]
fn live_oracle_series_count_all_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-COUNT-ALLNULLS",
        "case_id": "series_count_all_nulls",
        "mode": "strict",
        "operation": "series_count",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "empty",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping count all nulls test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = fp_types::Scalar::Int64(series.count() as i64);
    super::compare_scalar(&actual, &expected, "series_count").expect("pandas parity");
}

#[test]
fn live_oracle_series_nunique_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NUNIQUE-BASIC",
        "case_id": "series_nunique_basic",
        "mode": "strict",
        "operation": "series_nunique",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping nunique basic test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = fp_types::Scalar::Int64(series.nunique() as i64);
    super::compare_scalar(&actual, &expected, "series_nunique").expect("pandas parity");
}

#[test]
fn live_oracle_series_nunique_with_duplicates() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NUNIQUE-DUPS",
        "case_id": "series_nunique_with_duplicates",
        "mode": "strict",
        "operation": "series_nunique",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "dups",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 3 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping nunique dups test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = fp_types::Scalar::Int64(series.nunique() as i64);
    super::compare_scalar(&actual, &expected, "series_nunique").expect("pandas parity");
}

#[test]
fn live_oracle_series_nunique_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NUNIQUE-NULLS",
        "case_id": "series_nunique_with_nulls",
        "mode": "strict",
        "operation": "series_nunique",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "with_nulls",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 1.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping nunique with nulls test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Scalar(_)),
        "expected live oracle scalar payload, got {expected:?}"
    );
    let super::ResolvedExpected::Scalar(expected) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = fp_types::Scalar::Int64(series.nunique() as i64);
    super::compare_scalar(&actual, &expected, "series_nunique").expect("pandas parity");
}

#[test]
fn live_oracle_series_isna_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ISNA-BASIC",
        "case_id": "series_isna_basic",
        "mode": "strict",
        "operation": "series_isna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "mixed",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping isna basic test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.isna().expect("isna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_isna_no_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ISNA-NONULLS",
        "case_id": "series_isna_no_nulls",
        "mode": "strict",
        "operation": "series_isna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "complete",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping isna no nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.isna().expect("isna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_isna_all_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ISNA-ALLNULLS",
        "case_id": "series_isna_all_nulls",
        "mode": "strict",
        "operation": "series_isna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "empty",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping isna all nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.isna().expect("isna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_notna_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NOTNA-BASIC",
        "case_id": "series_notna_basic",
        "mode": "strict",
        "operation": "series_notna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "mixed",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping notna basic test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.notna().expect("notna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_notna_no_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NOTNA-NONULLS",
        "case_id": "series_notna_no_nulls",
        "mode": "strict",
        "operation": "series_notna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "complete",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping notna no nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.notna().expect("notna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_notna_all_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NOTNA-ALLNULLS",
        "case_id": "series_notna_all_nulls",
        "mode": "strict",
        "operation": "series_notna",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "empty",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping notna all nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.notna().expect("notna");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_head_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-HEAD-DEFAULT",
        "case_id": "series_head_default_5",
        "mode": "strict",
        "operation": "series_head",
        "oracle_source": "live_legacy_pandas",
        "head_n": 5,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 6 },
                { "kind": "int64", "value": 7 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 40 },
                { "kind": "int64", "value": 50 },
                { "kind": "int64", "value": 60 },
                { "kind": "int64", "value": 70 },
                { "kind": "int64", "value": 80 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping head default test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.head(5).expect("head");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_head_n_3() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-HEAD-N3",
        "case_id": "series_head_n_3",
        "mode": "strict",
        "operation": "series_head",
        "oracle_source": "live_legacy_pandas",
        "head_n": 3,
        "left": {
            "name": "letters",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" },
                { "kind": "utf8", "value": "e" }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 4.5 },
                { "kind": "float64", "value": 5.5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping head n=3 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.head(3).expect("head");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_head_larger_than_len() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-HEAD-OVER",
        "case_id": "series_head_n_larger_than_len",
        "mode": "strict",
        "operation": "series_head",
        "oracle_source": "live_legacy_pandas",
        "head_n": 100,
        "left": {
            "name": "small",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 7 },
                { "kind": "int64", "value": 8 },
                { "kind": "int64", "value": 9 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping head over-len test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.head(100).expect("head");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_tail_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-TAIL-DEFAULT",
        "case_id": "series_tail_default_5",
        "mode": "strict",
        "operation": "series_tail",
        "oracle_source": "live_legacy_pandas",
        "tail_n": 5,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 6 },
                { "kind": "int64", "value": 7 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 40 },
                { "kind": "int64", "value": 50 },
                { "kind": "int64", "value": 60 },
                { "kind": "int64", "value": 70 },
                { "kind": "int64", "value": 80 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping tail default test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.tail(5).expect("tail");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_tail_n_2() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-TAIL-N2",
        "case_id": "series_tail_n_2",
        "mode": "strict",
        "operation": "series_tail",
        "oracle_source": "live_legacy_pandas",
        "tail_n": 2,
        "left": {
            "name": "letters",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" },
                { "kind": "utf8", "value": "e" }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 3.5 },
                { "kind": "float64", "value": 4.5 },
                { "kind": "float64", "value": 5.5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping tail n=2 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.tail(2).expect("tail");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_tail_negative_n() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-TAIL-NEG",
        "case_id": "series_tail_negative_n",
        "mode": "strict",
        "operation": "series_tail",
        "oracle_source": "live_legacy_pandas",
        "tail_n": -2,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 40 },
                { "kind": "int64", "value": 50 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping tail negative n test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.tail(-2).expect("tail");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_diff_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DIFF-DEFAULT",
        "case_id": "series_diff_default",
        "mode": "strict",
        "operation": "series_diff",
        "oracle_source": "live_legacy_pandas",
        "diff_periods": 1,
        "left": {
            "name": "monotonic",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 9.0 },
                { "kind": "float64", "value": 16.0 },
                { "kind": "float64", "value": 25.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping diff default test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.diff(1).expect("diff");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_diff_periods_2() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DIFF-P2",
        "case_id": "series_diff_periods_2",
        "mode": "strict",
        "operation": "series_diff",
        "oracle_source": "live_legacy_pandas",
        "diff_periods": 2,
        "left": {
            "name": "monotonic",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 20.0 },
                { "kind": "float64", "value": 30.0 },
                { "kind": "float64", "value": 40.0 },
                { "kind": "float64", "value": 50.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping diff periods=2 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.diff(2).expect("diff");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_diff_negative_periods() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DIFF-NEG",
        "case_id": "series_diff_negative_periods",
        "mode": "strict",
        "operation": "series_diff",
        "oracle_source": "live_legacy_pandas",
        "diff_periods": -1,
        "left": {
            "name": "monotonic",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 15.0 },
                { "kind": "float64", "value": 20.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping diff negative test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.diff(-1).expect("diff");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_shift_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SHIFT-DEFAULT",
        "case_id": "series_shift_default",
        "mode": "strict",
        "operation": "series_shift",
        "oracle_source": "live_legacy_pandas",
        "shift_periods": 1,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping shift default test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.shift(1).expect("shift");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_shift_periods_3() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SHIFT-P3",
        "case_id": "series_shift_periods_3",
        "mode": "strict",
        "operation": "series_shift",
        "oracle_source": "live_legacy_pandas",
        "shift_periods": 3,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 20.0 },
                { "kind": "float64", "value": 30.0 },
                { "kind": "float64", "value": 40.0 },
                { "kind": "float64", "value": 50.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping shift periods=3 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.shift(3).expect("shift");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_shift_negative_periods() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SHIFT-NEG",
        "case_id": "series_shift_negative",
        "mode": "strict",
        "operation": "series_shift",
        "oracle_source": "live_legacy_pandas",
        "shift_periods": -2,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping shift negative test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.shift(-2).expect("shift");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_pct_change_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-PCTCHANGE-DEFAULT",
        "case_id": "series_pct_change_default",
        "mode": "strict",
        "operation": "series_pct_change",
        "oracle_source": "live_legacy_pandas",
        "pct_change_periods": 1,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 100.0 },
                { "kind": "float64", "value": 110.0 },
                { "kind": "float64", "value": 121.0 },
                { "kind": "float64", "value": 133.1 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping pct_change default test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.pct_change(1).expect("pct_change");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_pct_change_periods_2() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-PCTCHANGE-P2",
        "case_id": "series_pct_change_periods_2",
        "mode": "strict",
        "operation": "series_pct_change",
        "oracle_source": "live_legacy_pandas",
        "pct_change_periods": 2,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 50.0 },
                { "kind": "float64", "value": 75.0 },
                { "kind": "float64", "value": 100.0 },
                { "kind": "float64", "value": 150.0 },
                { "kind": "float64", "value": 200.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping pct_change periods=2 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.pct_change(2).expect("pct_change");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_pct_change_with_zero_baseline() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-PCTCHANGE-ZERO",
        "case_id": "series_pct_change_with_zero_baseline",
        "mode": "strict",
        "operation": "series_pct_change",
        "oracle_source": "live_legacy_pandas",
        "pct_change_periods": 1,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 0.0 },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 20.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping pct_change zero baseline test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.pct_change(1).expect("pct_change");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_any_with_truthy_value() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ANY-TRUTHY",
        "case_id": "series_any_with_truthy",
        "mode": "strict",
        "operation": "series_any",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping any truthy test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Bool(_)),
        "expected live oracle bool payload, got {expected:?}"
    );
    let super::ResolvedExpected::Bool(expected_bool) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.any().expect("any");
    assert_eq!(
        actual, expected_bool,
        "series_any: actual={actual}, expected={expected_bool}"
    );
}

#[test]
fn live_oracle_series_any_all_zeros() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ANY-ZEROS",
        "case_id": "series_any_all_zeros",
        "mode": "strict",
        "operation": "series_any",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping any all-zeros test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Bool(_)),
        "expected live oracle bool payload, got {expected:?}"
    );
    let super::ResolvedExpected::Bool(expected_bool) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.any().expect("any");
    assert_eq!(
        actual, expected_bool,
        "series_any: actual={actual}, expected={expected_bool}"
    );
}

#[test]
fn live_oracle_series_any_with_nulls_only() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ANY-NULLS",
        "case_id": "series_any_nulls_only",
        "mode": "strict",
        "operation": "series_any",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping any nulls test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Bool(_)),
        "expected live oracle bool payload, got {expected:?}"
    );
    let super::ResolvedExpected::Bool(expected_bool) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.any().expect("any");
    assert_eq!(
        actual, expected_bool,
        "series_any: actual={actual}, expected={expected_bool}"
    );
}

#[test]
fn live_oracle_series_all_with_truthy_values() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ALL-TRUTHY",
        "case_id": "series_all_with_truthy",
        "mode": "strict",
        "operation": "series_all",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
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
        eprintln!("live pandas unavailable; skipping all truthy test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Bool(_)),
        "expected live oracle bool payload, got {expected:?}"
    );
    let super::ResolvedExpected::Bool(expected_bool) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.all().expect("all");
    assert_eq!(
        actual, expected_bool,
        "series_all: actual={actual}, expected={expected_bool}"
    );
}

#[test]
fn live_oracle_series_all_with_zero_value() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ALL-ZERO",
        "case_id": "series_all_with_zero",
        "mode": "strict",
        "operation": "series_all",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 3 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping all-zero test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Bool(_)),
        "expected live oracle bool payload, got {expected:?}"
    );
    let super::ResolvedExpected::Bool(expected_bool) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.all().expect("all");
    assert_eq!(
        actual, expected_bool,
        "series_all: actual={actual}, expected={expected_bool}"
    );
}

#[test]
fn live_oracle_series_all_with_nulls_skipped() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ALL-NULLS",
        "case_id": "series_all_with_nulls",
        "mode": "strict",
        "operation": "series_all",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping all nulls test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Bool(_)),
        "expected live oracle bool payload, got {expected:?}"
    );
    let super::ResolvedExpected::Bool(expected_bool) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let actual = series.all().expect("all");
    assert_eq!(
        actual, expected_bool,
        "series_all: actual={actual}, expected={expected_bool}"
    );
}

#[test]
fn live_oracle_series_sort_values_ascending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SORTVALS-ASC",
        "case_id": "series_sort_values_ascending",
        "mode": "strict",
        "operation": "series_sort_values",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 2.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping sort_values asc test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.sort_values(true).expect("sort_values");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_sort_values_descending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SORTVALS-DESC",
        "case_id": "series_sort_values_descending",
        "mode": "strict",
        "operation": "series_sort_values",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": false,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 40 },
                { "kind": "int64", "value": 20 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping sort_values desc test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.sort_values(false).expect("sort_values");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_sort_values_with_nulls_last() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SORTVALS-NULLS",
        "case_id": "series_sort_values_with_nulls",
        "mode": "strict",
        "operation": "series_sort_values",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 2.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping sort_values nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.sort_values_na(true, "last").expect("sort_values_na");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_sort_index_ascending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SORTIDX-ASC",
        "case_id": "series_sort_index_ascending",
        "mode": "strict",
        "operation": "series_sort_index",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 0 }
            ],
            "values": [
                { "kind": "float64", "value": 30.0 },
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 20.0 },
                { "kind": "float64", "value": 0.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping sort_index asc test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.sort_index(true).expect("sort_index");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_sort_index_descending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SORTIDX-DESC",
        "case_id": "series_sort_index_descending",
        "mode": "strict",
        "operation": "series_sort_index",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": false,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "d" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": [
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 3 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping sort_index desc test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.sort_index(false).expect("sort_index");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_sort_index_already_sorted() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-SORTIDX-NOOP",
        "case_id": "series_sort_index_already_sorted",
        "mode": "strict",
        "operation": "series_sort_index",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 100 },
                { "kind": "int64", "value": 200 },
                { "kind": "int64", "value": 300 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping sort_index noop test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.sort_index(true).expect("sort_index");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_value_counts_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-VC-BASIC",
        "case_id": "series_value_counts_basic",
        "mode": "strict",
        "operation": "series_value_counts",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "fruits",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 6 }
            ],
            "values": [
                { "kind": "utf8", "value": "apple" },
                { "kind": "utf8", "value": "banana" },
                { "kind": "utf8", "value": "apple" },
                { "kind": "utf8", "value": "cherry" },
                { "kind": "utf8", "value": "banana" },
                { "kind": "utf8", "value": "apple" },
                { "kind": "utf8", "value": "cherry" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping value_counts basic test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.value_counts().expect("value_counts");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_value_counts_integers() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-VC-INT",
        "case_id": "series_value_counts_integers",
        "mode": "strict",
        "operation": "series_value_counts",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "ints",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "int64", "value": 7 },
                { "kind": "int64", "value": 7 },
                { "kind": "int64", "value": 7 },
                { "kind": "int64", "value": 8 },
                { "kind": "int64", "value": 8 },
                { "kind": "int64", "value": 9 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping value_counts ints test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.value_counts().expect("value_counts");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_value_counts_drops_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-VC-NULLS",
        "case_id": "series_value_counts_drops_nulls",
        "mode": "strict",
        "operation": "series_value_counts",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "with_nulls",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 1.5 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 1.5 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping value_counts nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.value_counts().expect("value_counts");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_rolling_mean_window_3() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ROLL-MEAN-3",
        "case_id": "series_rolling_mean_window_3",
        "mode": "strict",
        "operation": "series_rolling_mean",
        "oracle_source": "live_legacy_pandas",
        "window_size": 3,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 6.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping rolling_mean window=3 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.rolling(3, None).mean().expect("rolling mean");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_rolling_mean_min_periods_1() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ROLL-MEAN-MP1",
        "case_id": "series_rolling_mean_min_periods_1",
        "mode": "strict",
        "operation": "series_rolling_mean",
        "oracle_source": "live_legacy_pandas",
        "window_size": 3,
        "min_periods": 1,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 20.0 },
                { "kind": "float64", "value": 30.0 },
                { "kind": "float64", "value": 40.0 },
                { "kind": "float64", "value": 50.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping rolling_mean min_periods=1 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.rolling(3, Some(1)).mean().expect("rolling mean");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_rolling_sum_window_2() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ROLL-SUM-2",
        "case_id": "series_rolling_sum_window_2",
        "mode": "strict",
        "operation": "series_rolling_sum",
        "oracle_source": "live_legacy_pandas",
        "window_size": 2,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping rolling_sum window=2 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.rolling(2, None).sum().expect("rolling sum");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_rolling_min_window_3() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ROLL-MIN-3",
        "case_id": "series_rolling_min_window_3",
        "mode": "strict",
        "operation": "series_rolling_min",
        "oracle_source": "live_legacy_pandas",
        "window_size": 3,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping rolling_min window=3 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.rolling(3, None).min().expect("rolling min");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_rolling_max_window_3() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ROLL-MAX-3",
        "case_id": "series_rolling_max_window_3",
        "mode": "strict",
        "operation": "series_rolling_max",
        "oracle_source": "live_legacy_pandas",
        "window_size": 3,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping rolling_max window=3 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.rolling(3, None).max().expect("rolling max");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_rolling_count_window_3() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ROLL-CNT-3",
        "case_id": "series_rolling_count_window_3",
        "mode": "strict",
        "operation": "series_rolling_count",
        "oracle_source": "live_legacy_pandas",
        "window_size": 3,
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping rolling_count window=3 test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.rolling(3, None).count().expect("rolling count");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_expanding_mean_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-EXP-MEAN",
        "case_id": "series_expanding_mean_default",
        "mode": "strict",
        "operation": "series_expanding_mean",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 4.0 },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping expanding_mean test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.expanding(None).mean().expect("expanding mean");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_expanding_sum_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-EXP-SUM",
        "case_id": "series_expanding_sum_default",
        "mode": "strict",
        "operation": "series_expanding_sum",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 20.0 },
                { "kind": "float64", "value": 30.0 },
                { "kind": "float64", "value": 40.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping expanding_sum test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.expanding(None).sum().expect("expanding sum");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_expanding_min_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-EXP-MIN",
        "case_id": "series_expanding_min_default",
        "mode": "strict",
        "operation": "series_expanding_min",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 0.5 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping expanding_min test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.expanding(None).min().expect("expanding min");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_expanding_max_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-EXP-MAX",
        "case_id": "series_expanding_max_default",
        "mode": "strict",
        "operation": "series_expanding_max",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 5.0 },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 3.0 },
                { "kind": "float64", "value": 7.0 },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping expanding_max test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.expanding(None).max().expect("expanding max");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_expanding_count_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-EXP-CNT",
        "case_id": "series_expanding_count_with_nulls",
        "mode": "strict",
        "operation": "series_expanding_count",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "vals",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 5.0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping expanding_count test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.expanding(None).count().expect("expanding count");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_str_lower_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-STR-LOWER",
        "case_id": "series_str_lower_basic",
        "mode": "strict",
        "operation": "series_str_lower",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "names",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "utf8", "value": "Alice" },
                { "kind": "utf8", "value": "BOB" },
                { "kind": "utf8", "value": "Carol" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping str.lower test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.str().lower().expect("str lower");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_str_upper_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-STR-UPPER",
        "case_id": "series_str_upper_basic",
        "mode": "strict",
        "operation": "series_str_upper",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "names",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "utf8", "value": "alice" },
                { "kind": "utf8", "value": "BOB" },
                { "kind": "utf8", "value": "Carol" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping str.upper test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.str().upper().expect("str upper");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_str_len_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-STR-LEN",
        "case_id": "series_str_len_basic",
        "mode": "strict",
        "operation": "series_str_len",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "names",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "" },
                { "kind": "utf8", "value": "abc" },
                { "kind": "utf8", "value": "hello world" },
                { "kind": "utf8", "value": "x" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping str.len test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.str().len().expect("str len");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_str_strip_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-STR-STRIP",
        "case_id": "series_str_strip_basic",
        "mode": "strict",
        "operation": "series_str_strip",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "padded",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "utf8", "value": "  hello  " },
                { "kind": "utf8", "value": "\tworld\n" },
                { "kind": "utf8", "value": "no_pad" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping str.strip test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.str().strip().expect("str strip");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_str_capitalize_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-STR-CAP",
        "case_id": "series_str_capitalize_basic",
        "mode": "strict",
        "operation": "series_str_capitalize",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "names",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "utf8", "value": "alice" },
                { "kind": "utf8", "value": "BOB SMITH" },
                { "kind": "utf8", "value": "carol jones" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping str.capitalize test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.str().capitalize().expect("str capitalize");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_str_title_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-STR-TITLE",
        "case_id": "series_str_title_basic",
        "mode": "strict",
        "operation": "series_str_title",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "names",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "alice smith" },
                { "kind": "utf8", "value": "BOB JONES" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping str.title test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.str().title().expect("str title");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_isnull_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ISNULL-BASIC",
        "case_id": "series_isnull_basic",
        "mode": "strict",
        "operation": "series_isnull",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "mixed",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping isnull basic test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.isnull().expect("isnull");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_isnull_no_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-ISNULL-NONULLS",
        "case_id": "series_isnull_no_nulls",
        "mode": "strict",
        "operation": "series_isnull",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "complete",
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

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping isnull no nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.isnull().expect("isnull");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_notnull_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NOTNULL-BASIC",
        "case_id": "series_notnull_basic",
        "mode": "strict",
        "operation": "series_notnull",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "mixed",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping notnull basic test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.notnull().expect("notnull");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_notnull_all_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-NOTNULL-ALLNULLS",
        "case_id": "series_notnull_all_nulls",
        "mode": "strict",
        "operation": "series_notnull",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "empty",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "null" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping notnull all nulls test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.notnull().expect("notnull");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_filter_mostly_true() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FILTER-TRUE",
        "case_id": "series_filter_mostly_true",
        "mode": "strict",
        "operation": "series_filter",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "data",
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
        },
        "right": {
            "name": "mask",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "bool", "value": true },
                { "kind": "bool", "value": false },
                { "kind": "bool", "value": true },
                { "kind": "bool", "value": true }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping filter mostly-true test: {message}");
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

    let data = super::build_series(fixture.left.as_ref().expect("left")).expect("data series");
    let mask = super::build_series(fixture.right.as_ref().expect("right")).expect("mask series");
    let result = data.filter(&mask).expect("filter");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_filter_all_true() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FILTER-ALL-TRUE",
        "case_id": "series_filter_all_true",
        "mode": "strict",
        "operation": "series_filter",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "data",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 }
            ]
        },
        "right": {
            "name": "mask",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "bool", "value": true },
                { "kind": "bool", "value": true },
                { "kind": "bool", "value": true }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping filter all-true test: {message}");
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

    let data = super::build_series(fixture.left.as_ref().expect("left")).expect("data series");
    let mask = super::build_series(fixture.right.as_ref().expect("right")).expect("mask series");
    let result = data.filter(&mask).expect("filter");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_filter_all_false() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-FILTER-ALL-FALSE",
        "case_id": "series_filter_all_false",
        "mode": "strict",
        "operation": "series_filter",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "data",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "float64", "value": 3.0 }
            ]
        },
        "right": {
            "name": "mask",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "bool", "value": false },
                { "kind": "bool", "value": false },
                { "kind": "bool", "value": false }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping filter all-false test: {message}");
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

    let data = super::build_series(fixture.left.as_ref().expect("left")).expect("data series");
    let mask = super::build_series(fixture.right.as_ref().expect("right")).expect("mask series");
    let result = data.filter(&mask).expect("filter");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_to_numeric_clean_strings() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-TONUM-CLEAN",
        "case_id": "series_to_numeric_clean_strings",
        "mode": "strict",
        "operation": "series_to_numeric",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "as_str",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "utf8", "value": "1" },
                { "kind": "utf8", "value": "2.5" },
                { "kind": "utf8", "value": "100" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping to_numeric clean strings test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = fp_frame::to_numeric(&series).expect("to_numeric");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_to_numeric_with_invalid_coerces_to_nan() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-TONUM-INVALID",
        "case_id": "series_to_numeric_with_invalid_coerces_to_nan",
        "mode": "strict",
        "operation": "series_to_numeric",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "as_str",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "1" },
                { "kind": "utf8", "value": "abc" },
                { "kind": "utf8", "value": "3.14" },
                { "kind": "utf8", "value": "xyz" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping to_numeric invalid test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = fp_frame::to_numeric(&series).expect("to_numeric");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_to_numeric_already_numeric() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-TONUM-NUM",
        "case_id": "series_to_numeric_already_numeric",
        "mode": "strict",
        "operation": "series_to_numeric",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "ints",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 42 },
                { "kind": "int64", "value": -7 },
                { "kind": "int64", "value": 0 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping to_numeric already-numeric test: {message}");
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

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = fp_frame::to_numeric(&series).expect("to_numeric");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_series_join_inner_overlap() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-JOIN-INNER",
        "case_id": "series_join_inner_overlap",
        "mode": "strict",
        "operation": "series_join",
        "oracle_source": "live_legacy_pandas",
        "join_type": "inner",
        "left": {
            "name": "left",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ]
        },
        "right": {
            "name": "right",
            "index": [
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 40 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series_join inner test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Join(_)),
        "expected live oracle join payload, got {expected:?}"
    );
    let super::ResolvedExpected::Join(expected_join) = expected else {
        return;
    };

    let left = super::build_series(fixture.left.as_ref().expect("left")).expect("left series");
    let right = super::build_series(fixture.right.as_ref().expect("right")).expect("right series");
    let joined = fp_join::join_series(&left, &right, fp_join::JoinType::Inner).expect("join");
    super::compare_join_expected(&joined, &expected_join).expect("pandas parity");
}

#[test]
fn live_oracle_series_join_left_with_missing_right() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-JOIN-LEFT",
        "case_id": "series_join_left_with_missing_right",
        "mode": "strict",
        "operation": "series_join",
        "oracle_source": "live_legacy_pandas",
        "join_type": "left",
        "left": {
            "name": "left",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ]
        },
        "right": {
            "name": "right",
            "index": [
                { "kind": "utf8", "value": "b" }
            ],
            "values": [
                { "kind": "int64", "value": 20 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series_join left test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Join(_)),
        "expected live oracle join payload, got {expected:?}"
    );
    let super::ResolvedExpected::Join(expected_join) = expected else {
        return;
    };

    let left = super::build_series(fixture.left.as_ref().expect("left")).expect("left series");
    let right = super::build_series(fixture.right.as_ref().expect("right")).expect("right series");
    let joined = fp_join::join_series(&left, &right, fp_join::JoinType::Left).expect("join");
    super::compare_join_expected(&joined, &expected_join).expect("pandas parity");
}

#[test]
fn live_oracle_series_join_outer_disjoint() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-JOIN-OUTER",
        "case_id": "series_join_outer_disjoint",
        "mode": "strict",
        "operation": "series_join",
        "oracle_source": "live_legacy_pandas",
        "join_type": "outer",
        "left": {
            "name": "left",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" }
            ],
            "values": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ]
        },
        "right": {
            "name": "right",
            "index": [
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" }
            ],
            "values": [
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 40 }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping series_join outer test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Join(_)),
        "expected live oracle join payload, got {expected:?}"
    );
    let super::ResolvedExpected::Join(expected_join) = expected else {
        return;
    };

    let left = super::build_series(fixture.left.as_ref().expect("left")).expect("left series");
    let right = super::build_series(fixture.right.as_ref().expect("right")).expect("right series");
    let joined = fp_join::join_series(&left, &right, fp_join::JoinType::Outer).expect("join");
    super::compare_join_expected(&joined, &expected_join).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_isna_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-ISNA",
        "case_id": "dataframe_isna_basic",
        "mode": "strict",
        "operation": "dataframe_isna",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 3.0 }
                ],
                "b": [
                    { "kind": "null", "value": "null" },
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 6 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df isna basic test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.isna().expect("isna");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_notna_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-NOTNA",
        "case_id": "dataframe_notna_basic",
        "mode": "strict",
        "operation": "dataframe_notna",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 3.0 }
                ],
                "b": [
                    { "kind": "null", "value": "null" },
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 6 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df notna basic test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.notna().expect("notna");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_isna_no_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-ISNA-NONULLS",
        "case_id": "dataframe_isna_no_nulls",
        "mode": "strict",
        "operation": "dataframe_isna",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "columns": {
                "x": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 }
                ],
                "y": [
                    { "kind": "float64", "value": 1.5 },
                    { "kind": "float64", "value": 2.5 }
                ]
            },
            "column_order": ["x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df isna no-nulls test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.isna().expect("isna");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_count_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-COUNT-BASIC",
        "case_id": "dataframe_count_basic",
        "mode": "strict",
        "operation": "dataframe_count",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "null", "value": "null" }
                ],
                "b": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 40 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df count basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.count().expect("count");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_count_no_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-COUNT-NONULLS",
        "case_id": "dataframe_count_no_nulls",
        "mode": "strict",
        "operation": "dataframe_count",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "x": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "y": [
                    { "kind": "float64", "value": 1.5 },
                    { "kind": "float64", "value": 2.5 },
                    { "kind": "float64", "value": 3.5 }
                ]
            },
            "column_order": ["x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df count no-nulls test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.count().expect("count");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_count_all_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-COUNT-ALLNULLS",
        "case_id": "dataframe_count_all_nulls",
        "mode": "strict",
        "operation": "dataframe_count",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "columns": {
                "p": [
                    { "kind": "null", "value": "null" },
                    { "kind": "null", "value": "null" }
                ],
                "q": [
                    { "kind": "null", "value": "null" },
                    { "kind": "null", "value": "null" }
                ]
            },
            "column_order": ["p", "q"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df count all-nulls test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.count().expect("count");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_sum_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-SUM",
        "case_id": "dataframe_sum_basic",
        "mode": "strict",
        "operation": "dataframe_sum",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "x": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 }
                ],
                "y": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 30.0 }
                ]
            },
            "column_order": ["x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df sum basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.sum().expect("sum");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_sum_with_nulls_skipna() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-SUM-NULLS",
        "case_id": "dataframe_sum_with_nulls_skipna",
        "mode": "strict",
        "operation": "dataframe_sum",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.5 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 2.5 },
                    { "kind": "float64", "value": 6.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 30.0 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df sum with nulls test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.sum().expect("sum");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_mean_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-MEAN",
        "case_id": "dataframe_mean_basic",
        "mode": "strict",
        "operation": "dataframe_mean",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "x": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 }
                ],
                "y": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 30.0 },
                    { "kind": "float64", "value": 40.0 }
                ]
            },
            "column_order": ["x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df mean basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.mean().expect("mean");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_mean_with_nulls_skipna() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-MEAN-NULLS",
        "case_id": "dataframe_mean_with_nulls_skipna",
        "mode": "strict",
        "operation": "dataframe_mean",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "p": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 4.0 }
                ],
                "q": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 30.0 }
                ]
            },
            "column_order": ["p", "q"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df mean with nulls test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.mean().expect("mean");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_min_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-MIN",
        "case_id": "dataframe_min_basic",
        "mode": "strict",
        "operation": "dataframe_min",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "x": [
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 3.0 }
                ],
                "y": [
                    { "kind": "float64", "value": 0.5 },
                    { "kind": "float64", "value": 9.0 },
                    { "kind": "float64", "value": 2.0 }
                ]
            },
            "column_order": ["x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df min basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.min_agg().expect("min");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_min_with_nulls_skipna() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-MIN-NULLS",
        "case_id": "dataframe_min_with_nulls_skipna",
        "mode": "strict",
        "operation": "dataframe_min",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "float64", "value": 2.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 1.0 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df min with nulls test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.min_agg().expect("min");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_max_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-MAX",
        "case_id": "dataframe_max_basic",
        "mode": "strict",
        "operation": "dataframe_max",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "x": [
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 3.0 }
                ],
                "y": [
                    { "kind": "float64", "value": 0.5 },
                    { "kind": "float64", "value": 9.0 },
                    { "kind": "float64", "value": 2.0 }
                ]
            },
            "column_order": ["x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df max basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.max_agg().expect("max");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_max_with_nulls_skipna() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-MAX-NULLS",
        "case_id": "dataframe_max_with_nulls_skipna",
        "mode": "strict",
        "operation": "dataframe_max",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "float64", "value": 2.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 11.0 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df max with nulls test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.max_agg().expect("max");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_median_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-MEDIAN",
        "case_id": "dataframe_median_basic",
        "mode": "strict",
        "operation": "dataframe_median",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "columns": {
                "x": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 5.0 }
                ],
                "y": [
                    { "kind": "float64", "value": 50.0 },
                    { "kind": "float64", "value": 30.0 },
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 40.0 }
                ]
            },
            "column_order": ["x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df median basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.median_agg().expect("median");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_std_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-STD",
        "case_id": "dataframe_std_basic",
        "mode": "strict",
        "operation": "dataframe_std",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 6.0 },
                    { "kind": "float64", "value": 8.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 1.0 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df std basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.std_agg().expect("std");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_var_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-VAR",
        "case_id": "dataframe_var_basic",
        "mode": "strict",
        "operation": "dataframe_var",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 6.0 },
                    { "kind": "float64", "value": 8.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 5.0 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df var basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.var_agg().expect("var");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_any_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-ANY",
        "case_id": "dataframe_any_basic",
        "mode": "strict",
        "operation": "dataframe_any",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 0 }
                ],
                "b": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 0 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df any basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.any().expect("any");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_all_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-ALL",
        "case_id": "dataframe_all_basic",
        "mode": "strict",
        "operation": "dataframe_all",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 1 }
                ],
                "b": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df all basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.all().expect("all");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_head_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-HEAD",
        "case_id": "dataframe_head_default_5",
        "mode": "strict",
        "operation": "dataframe_head",
        "oracle_source": "live_legacy_pandas",
        "head_n": 5,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 6 },
                { "kind": "int64", "value": 7 }
            ],
            "columns": {
                "x": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 40 },
                    { "kind": "int64", "value": 50 },
                    { "kind": "int64", "value": 60 },
                    { "kind": "int64", "value": 70 },
                    { "kind": "int64", "value": 80 }
                ]
            },
            "column_order": ["x"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df head default test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.head(5).expect("head");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_head_n_3() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-HEAD-3",
        "case_id": "dataframe_head_n_3",
        "mode": "strict",
        "operation": "dataframe_head",
        "oracle_source": "live_legacy_pandas",
        "head_n": 3,
        "frame": {
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "d" },
                { "kind": "utf8", "value": "e" }
            ],
            "columns": {
                "v": [
                    { "kind": "float64", "value": 1.5 },
                    { "kind": "float64", "value": 2.5 },
                    { "kind": "float64", "value": 3.5 },
                    { "kind": "float64", "value": 4.5 },
                    { "kind": "float64", "value": 5.5 }
                ]
            },
            "column_order": ["v"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df head n=3 test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.head(3).expect("head");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_tail_default() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-TAIL",
        "case_id": "dataframe_tail_default_5",
        "mode": "strict",
        "operation": "dataframe_tail",
        "oracle_source": "live_legacy_pandas",
        "tail_n": 5,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 6 },
                { "kind": "int64", "value": 7 }
            ],
            "columns": {
                "x": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 40 },
                    { "kind": "int64", "value": 50 },
                    { "kind": "int64", "value": 60 },
                    { "kind": "int64", "value": 70 },
                    { "kind": "int64", "value": 80 }
                ]
            },
            "column_order": ["x"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df tail default test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.tail(5).expect("tail");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_head_larger_than_len() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-HEAD-OVER",
        "case_id": "dataframe_head_over_len",
        "mode": "strict",
        "operation": "dataframe_head",
        "oracle_source": "live_legacy_pandas",
        "head_n": 100,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "x": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ]
            },
            "column_order": ["x"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df head over-len test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.head(100).expect("head");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_dropna_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-DROPNA",
        "case_id": "dataframe_dropna_basic",
        "mode": "strict",
        "operation": "dataframe_dropna",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 }
                ],
                "b": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 40.0 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df dropna basic test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.dropna().expect("dropna");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_dropna_no_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-DROPNA-NONE",
        "case_id": "dataframe_dropna_no_nulls",
        "mode": "strict",
        "operation": "dataframe_dropna",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "x": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "y": [
                    { "kind": "float64", "value": 1.5 },
                    { "kind": "float64", "value": 2.5 },
                    { "kind": "float64", "value": 3.5 }
                ]
            },
            "column_order": ["x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df dropna no-nulls test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.dropna().expect("dropna");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_fillna_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-FILLNA",
        "case_id": "dataframe_fillna_basic",
        "mode": "strict",
        "operation": "dataframe_fillna",
        "oracle_source": "live_legacy_pandas",
        "fill_value": { "kind": "float64", "value": 0.0 },
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 3.0 }
                ],
                "b": [
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 30.0 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df fillna basic test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .fillna(&fp_types::Scalar::Float64(0.0))
        .expect("fillna");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_nunique_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-NUNIQUE",
        "case_id": "dataframe_nunique_basic",
        "mode": "strict",
        "operation": "dataframe_nunique",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 3 }
                ],
                "b": [
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 5 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df nunique basic test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.nunique().expect("nunique");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_nunique_with_nulls() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-NUNIQUE-NULLS",
        "case_id": "dataframe_nunique_with_nulls",
        "mode": "strict",
        "operation": "dataframe_nunique",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.5 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 2.5 },
                    { "kind": "float64", "value": 1.5 }
                ],
                "b": [
                    { "kind": "null", "value": "null" },
                    { "kind": "null", "value": "null" },
                    { "kind": "null", "value": "null" },
                    { "kind": "null", "value": "null" }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df nunique with nulls test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.nunique().expect("nunique");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_duplicated_basic_first() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-DUP-FIRST",
        "case_id": "dataframe_duplicated_basic_first",
        "mode": "strict",
        "operation": "dataframe_duplicated",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 2 }
                ],
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" },
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "z" },
                    { "kind": "utf8", "value": "y" }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df duplicated first test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .duplicated(None, super::DuplicateKeep::First)
        .expect("duplicated");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_duplicated_no_dups() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-DUP-NONE",
        "case_id": "dataframe_duplicated_no_dups",
        "mode": "strict",
        "operation": "dataframe_duplicated",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 30 }
                ]
            },
            "column_order": ["a"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df duplicated no-dups test: {message}");
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

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .duplicated(None, super::DuplicateKeep::First)
        .expect("duplicated");
    super::compare_series_expected(&result, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_drop_duplicates_keep_first() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-DROPDUP-FIRST",
        "case_id": "dataframe_drop_duplicates_keep_first",
        "mode": "strict",
        "operation": "dataframe_drop_duplicates",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 2 }
                ],
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" },
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "z" },
                    { "kind": "utf8", "value": "y" }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df drop_duplicates first test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .drop_duplicates(None, super::DuplicateKeep::First, false)
        .expect("drop_duplicates");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_drop_duplicates_keep_none() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-DROPDUP-NONE",
        "case_id": "dataframe_drop_duplicates_keep_none",
        "mode": "strict",
        "operation": "dataframe_drop_duplicates",
        "oracle_source": "live_legacy_pandas",
        "keep": "none",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 2 }
                ],
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" },
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "z" },
                    { "kind": "utf8", "value": "y" }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df drop_duplicates none test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .drop_duplicates(None, super::DuplicateKeep::None, false)
        .expect("drop_duplicates");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_sort_index_ascending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-SORTIDX-ASC",
        "case_id": "dataframe_sort_index_ascending",
        "mode": "strict",
        "operation": "dataframe_sort_index",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "frame": {
            "index": [
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 0 }
            ],
            "columns": {
                "x": [
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 0 }
                ]
            },
            "column_order": ["x"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df sort_index asc test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.sort_index(true).expect("sort_index");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_sort_index_descending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-SORTIDX-DESC",
        "case_id": "dataframe_sort_index_descending",
        "mode": "strict",
        "operation": "dataframe_sort_index",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": false,
        "frame": {
            "index": [
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "d" },
                { "kind": "utf8", "value": "c" }
            ],
            "columns": {
                "v": [
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 4 },
                    { "kind": "int64", "value": 3 }
                ]
            },
            "column_order": ["v"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df sort_index desc test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.sort_index(false).expect("sort_index");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_sort_values_ascending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-SORTVALS-ASC",
        "case_id": "dataframe_sort_values_ascending",
        "mode": "strict",
        "operation": "dataframe_sort_values",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "sort_column": "score",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "name": [
                    { "kind": "utf8", "value": "alice" },
                    { "kind": "utf8", "value": "bob" },
                    { "kind": "utf8", "value": "carol" },
                    { "kind": "utf8", "value": "dave" }
                ],
                "score": [
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 40 },
                    { "kind": "int64", "value": 20 }
                ]
            },
            "column_order": ["name", "score"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df sort_values asc test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.sort_values("score", true).expect("sort_values");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_sort_values_descending() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-SORTVALS-DESC",
        "case_id": "dataframe_sort_values_descending",
        "mode": "strict",
        "operation": "dataframe_sort_values",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": false,
        "sort_column": "score",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "name": [
                    { "kind": "utf8", "value": "alice" },
                    { "kind": "utf8", "value": "bob" },
                    { "kind": "utf8", "value": "carol" },
                    { "kind": "utf8", "value": "dave" }
                ],
                "score": [
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 40 },
                    { "kind": "int64", "value": 20 }
                ]
            },
            "column_order": ["name", "score"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df sort_values desc test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.sort_values("score", false).expect("sort_values");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_drop_columns_single() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-DROPCOLS-1",
        "case_id": "dataframe_drop_columns_single",
        "mode": "strict",
        "operation": "dataframe_drop_columns",
        "oracle_source": "live_legacy_pandas",
        "drop_columns": ["b"],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ],
                "b": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 }
                ],
                "c": [
                    { "kind": "float64", "value": 1.5 },
                    { "kind": "float64", "value": 2.5 }
                ]
            },
            "column_order": ["a", "b", "c"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df drop_columns single test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.drop_columns(&["b"]).expect("drop_columns");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_drop_columns_multiple() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-DROPCOLS-N",
        "case_id": "dataframe_drop_columns_multiple",
        "mode": "strict",
        "operation": "dataframe_drop_columns",
        "oracle_source": "live_legacy_pandas",
        "drop_columns": ["a", "c"],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" },
                    { "kind": "utf8", "value": "z" }
                ],
                "c": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 }
                ]
            },
            "column_order": ["a", "b", "c"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df drop_columns multi test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.drop_columns(&["a", "c"]).expect("drop_columns");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_rename_columns_single() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-RENAME-1",
        "case_id": "dataframe_rename_columns_single",
        "mode": "strict",
        "operation": "dataframe_rename_columns",
        "oracle_source": "live_legacy_pandas",
        "rename_columns": [{ "from": "a", "to": "alpha" }],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ],
                "b": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df rename single test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .rename_columns(&[("a", "alpha")])
        .expect("rename_columns");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_rename_columns_multiple() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-RENAME-N",
        "case_id": "dataframe_rename_columns_multiple",
        "mode": "strict",
        "operation": "dataframe_rename_columns",
        "oracle_source": "live_legacy_pandas",
        "rename_columns": [
            { "from": "a", "to": "alpha" },
            { "from": "c", "to": "gamma" }
        ],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" },
                    { "kind": "utf8", "value": "z" }
                ],
                "c": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 }
                ]
            },
            "column_order": ["a", "b", "c"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df rename multi test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .rename_columns(&[("a", "alpha"), ("c", "gamma")])
        .expect("rename_columns");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_assign_new_column() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-ASSIGN-NEW",
        "case_id": "dataframe_assign_new_column",
        "mode": "strict",
        "operation": "dataframe_assign",
        "oracle_source": "live_legacy_pandas",
        "assignments": [
            {
                "name": "c",
                "values": [
                    { "kind": "int64", "value": 100 },
                    { "kind": "int64", "value": 200 },
                    { "kind": "int64", "value": 300 }
                ]
            }
        ],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "b": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 30 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df assign new test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let new_col = fp_columnar::Column::from_values(vec![
        fp_types::Scalar::Int64(100),
        fp_types::Scalar::Int64(200),
        fp_types::Scalar::Int64(300),
    ])
    .expect("column");
    let result = frame.assign(vec![("c", new_col)]).expect("assign");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_assign_overwrite_column() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-ASSIGN-OVERWRITE",
        "case_id": "dataframe_assign_overwrite_column",
        "mode": "strict",
        "operation": "dataframe_assign",
        "oracle_source": "live_legacy_pandas",
        "assignments": [
            {
                "name": "a",
                "values": [
                    { "kind": "int64", "value": -1 },
                    { "kind": "int64", "value": -2 },
                    { "kind": "int64", "value": -3 }
                ]
            }
        ],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" },
                    { "kind": "utf8", "value": "z" }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df assign overwrite test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let new_col = fp_columnar::Column::from_values(vec![
        fp_types::Scalar::Int64(-1),
        fp_types::Scalar::Int64(-2),
        fp_types::Scalar::Int64(-3),
    ])
    .expect("column");
    let result = frame.assign(vec![("a", new_col)]).expect("assign");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_replace_int_to_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-REPLACE-INT",
        "case_id": "dataframe_replace_int_to_int",
        "mode": "strict",
        "operation": "dataframe_replace",
        "oracle_source": "live_legacy_pandas",
        "replace_to_find": [
            { "kind": "int64", "value": 1 },
            { "kind": "int64", "value": 2 }
        ],
        "replace_to_value": [
            { "kind": "int64", "value": 100 },
            { "kind": "int64", "value": 200 }
        ],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 1 }
                ],
                "b": [
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 4 }
                ]
            },
            "column_order": ["a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df replace int test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let replacements = vec![
        (fp_types::Scalar::Int64(1), fp_types::Scalar::Int64(100)),
        (fp_types::Scalar::Int64(2), fp_types::Scalar::Int64(200)),
    ];
    let result = frame.replace(&replacements).expect("replace");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_replace_str_to_str() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-REPLACE-STR",
        "case_id": "dataframe_replace_str_to_str",
        "mode": "strict",
        "operation": "dataframe_replace",
        "oracle_source": "live_legacy_pandas",
        "replace_to_find": [
            { "kind": "utf8", "value": "old" }
        ],
        "replace_to_value": [
            { "kind": "utf8", "value": "new" }
        ],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "name": [
                    { "kind": "utf8", "value": "old" },
                    { "kind": "utf8", "value": "fresh" },
                    { "kind": "utf8", "value": "old" }
                ],
                "tag": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "old" },
                    { "kind": "utf8", "value": "y" }
                ]
            },
            "column_order": ["name", "tag"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df replace str test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let replacements = vec![(
        fp_types::Scalar::Utf8("old".to_owned()),
        fp_types::Scalar::Utf8("new".to_owned()),
    )];
    let result = frame.replace(&replacements).expect("replace");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_set_index_drop_true() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-SETIDX-DROP",
        "case_id": "dataframe_set_index_drop_true",
        "mode": "strict",
        "operation": "dataframe_set_index",
        "oracle_source": "live_legacy_pandas",
        "set_index_column": "id",
        "set_index_drop": true,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "id": [
                    { "kind": "int64", "value": 100 },
                    { "kind": "int64", "value": 200 },
                    { "kind": "int64", "value": 300 }
                ],
                "name": [
                    { "kind": "utf8", "value": "alice" },
                    { "kind": "utf8", "value": "bob" },
                    { "kind": "utf8", "value": "carol" }
                ]
            },
            "column_order": ["id", "name"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df set_index drop test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.set_index("id", true).expect("set_index");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_set_index_drop_false() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-SETIDX-KEEP",
        "case_id": "dataframe_set_index_drop_false",
        "mode": "strict",
        "operation": "dataframe_set_index",
        "oracle_source": "live_legacy_pandas",
        "set_index_column": "key",
        "set_index_drop": false,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "key": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "c" }
                ],
                "value": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ]
            },
            "column_order": ["key", "value"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df set_index keep test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.set_index("key", false).expect("set_index");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_reset_index_drop_false() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-RESETIDX-KEEP",
        "case_id": "dataframe_reset_index_drop_false",
        "mode": "strict",
        "operation": "dataframe_reset_index",
        "oracle_source": "live_legacy_pandas",
        "reset_index_drop": false,
        "frame": {
            "index": [
                { "kind": "utf8", "value": "alpha" },
                { "kind": "utf8", "value": "beta" },
                { "kind": "utf8", "value": "gamma" }
            ],
            "columns": {
                "value": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ]
            },
            "column_order": ["value"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df reset_index keep test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.reset_index(false).expect("reset_index");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_reset_index_drop_true() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-RESETIDX-DROP",
        "case_id": "dataframe_reset_index_drop_true",
        "mode": "strict",
        "operation": "dataframe_reset_index",
        "oracle_source": "live_legacy_pandas",
        "reset_index_drop": true,
        "frame": {
            "index": [
                { "kind": "utf8", "value": "alpha" },
                { "kind": "utf8", "value": "beta" },
                { "kind": "utf8", "value": "gamma" }
            ],
            "columns": {
                "value": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ]
            },
            "column_order": ["value"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df reset_index drop test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.reset_index(true).expect("reset_index");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_series_str_get_dummies_pipe() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-STR-DUMMIES",
        "case_id": "series_str_get_dummies_pipe",
        "mode": "strict",
        "operation": "series_str_get_dummies",
        "oracle_source": "live_legacy_pandas",
        "string_sep": "|",
        "left": {
            "name": "tags",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "utf8", "value": "a|b" },
                { "kind": "utf8", "value": "b|c" },
                { "kind": "utf8", "value": "a" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping str.get_dummies pipe test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.str().get_dummies("|").expect("get_dummies");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_series_str_get_dummies_comma() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-STR-DUMMIES-COMMA",
        "case_id": "series_str_get_dummies_comma",
        "mode": "strict",
        "operation": "series_str_get_dummies",
        "oracle_source": "live_legacy_pandas",
        "string_sep": ",",
        "left": {
            "name": "tags",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "values": [
                { "kind": "utf8", "value": "red,green" },
                { "kind": "utf8", "value": "green,blue,red" }
            ]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping str.get_dummies comma test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let series = super::build_series(fixture.left.as_ref().expect("left")).expect("series");
    let result = series.str().get_dummies(",").expect("get_dummies");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_transpose_homogeneous_int() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-TRANSPOSE-INT",
        "case_id": "dataframe_transpose_homogeneous_int",
        "mode": "strict",
        "operation": "dataframe_transpose",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ],
                "b": [
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 4 }
                ],
                "c": [
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 6 }
                ]
            },
            "column_order": ["a", "b", "c"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df transpose int test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.transpose().expect("transpose");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_transpose_homogeneous_float() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-TRANSPOSE-FLOAT",
        "case_id": "dataframe_transpose_homogeneous_float",
        "mode": "strict",
        "operation": "dataframe_transpose",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "columns": {
                "x": [
                    { "kind": "float64", "value": 1.5 },
                    { "kind": "float64", "value": 2.5 },
                    { "kind": "float64", "value": 3.5 }
                ],
                "y": [
                    { "kind": "float64", "value": 10.5 },
                    { "kind": "float64", "value": 20.5 },
                    { "kind": "float64", "value": 30.5 }
                ]
            },
            "column_order": ["x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df transpose float test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.transpose().expect("transpose");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_melt_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-MELT",
        "case_id": "dataframe_melt_basic",
        "mode": "strict",
        "operation": "dataframe_melt",
        "oracle_source": "live_legacy_pandas",
        "melt_id_vars": ["id"],
        "melt_value_vars": ["a", "b"],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "columns": {
                "id": [
                    { "kind": "utf8", "value": "row0" },
                    { "kind": "utf8", "value": "row1" }
                ],
                "a": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 }
                ],
                "b": [
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 40 }
                ]
            },
            "column_order": ["id", "a", "b"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df melt basic test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .melt(&["id"], &["a", "b"], None, None)
        .expect("melt");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_melt_with_var_value_names() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-MELT-NAMES",
        "case_id": "dataframe_melt_with_var_value_names",
        "mode": "strict",
        "operation": "dataframe_melt",
        "oracle_source": "live_legacy_pandas",
        "melt_id_vars": ["key"],
        "melt_value_vars": ["x", "y"],
        "melt_var_name": "metric",
        "melt_value_name": "score",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "columns": {
                "key": [
                    { "kind": "utf8", "value": "alpha" },
                    { "kind": "utf8", "value": "beta" }
                ],
                "x": [
                    { "kind": "float64", "value": 1.5 },
                    { "kind": "float64", "value": 2.5 }
                ],
                "y": [
                    { "kind": "float64", "value": 10.5 },
                    { "kind": "float64", "value": 20.5 }
                ]
            },
            "column_order": ["key", "x", "y"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df melt names test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .melt(&["key"], &["x", "y"], Some("metric"), Some("score"))
        .expect("melt");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_pivot_basic() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-PIVOT",
        "case_id": "dataframe_pivot_basic",
        "mode": "strict",
        "operation": "dataframe_pivot",
        "oracle_source": "live_legacy_pandas",
        "pivot_index": "row",
        "pivot_columns": "col",
        "pivot_values": ["val"],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "columns": {
                "row": [
                    { "kind": "utf8", "value": "r1" },
                    { "kind": "utf8", "value": "r1" },
                    { "kind": "utf8", "value": "r2" },
                    { "kind": "utf8", "value": "r2" }
                ],
                "col": [
                    { "kind": "utf8", "value": "c1" },
                    { "kind": "utf8", "value": "c2" },
                    { "kind": "utf8", "value": "c1" },
                    { "kind": "utf8", "value": "c2" }
                ],
                "val": [
                    { "kind": "int64", "value": 11 },
                    { "kind": "int64", "value": 12 },
                    { "kind": "int64", "value": 21 },
                    { "kind": "int64", "value": 22 }
                ]
            },
            "column_order": ["row", "col", "val"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df pivot basic test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame.pivot("row", "col", "val").expect("pivot");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_explode_pipe_keep_index() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-EXPLODE-PIPE",
        "case_id": "dataframe_explode_pipe_keep_index",
        "mode": "strict",
        "operation": "dataframe_explode",
        "oracle_source": "live_legacy_pandas",
        "explode_column": "tags",
        "string_sep": "|",
        "ignore_index": false,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "columns": {
                "id": [
                    { "kind": "utf8", "value": "row0" },
                    { "kind": "utf8", "value": "row1" }
                ],
                "tags": [
                    { "kind": "utf8", "value": "a|b|c" },
                    { "kind": "utf8", "value": "x|y" }
                ]
            },
            "column_order": ["id", "tags"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df explode keep test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .explode_with_ignore_index("tags", "|", false)
        .expect("explode");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_explode_comma_ignore_index() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-DF-EXPLODE-COMMA",
        "case_id": "dataframe_explode_comma_ignore_index",
        "mode": "strict",
        "operation": "dataframe_explode",
        "oracle_source": "live_legacy_pandas",
        "explode_column": "items",
        "string_sep": ",",
        "ignore_index": true,
        "frame": {
            "index": [
                { "kind": "utf8", "value": "alpha" },
                { "kind": "utf8", "value": "beta" }
            ],
            "columns": {
                "label": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ],
                "items": [
                    { "kind": "utf8", "value": "x,y" },
                    { "kind": "utf8", "value": "z" }
                ]
            },
            "column_order": ["label", "items"]
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping df explode ignore test: {message}");
        return;
    }
    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected_frame) = expected else {
        return;
    };

    let frame = super::build_dataframe(fixture.frame.as_ref().expect("frame")).expect("dataframe");
    let result = frame
        .explode_with_ignore_index("items", ",", true)
        .expect("explode");
    super::compare_dataframe_expected(&result, &expected_frame).expect("pandas parity");
}
