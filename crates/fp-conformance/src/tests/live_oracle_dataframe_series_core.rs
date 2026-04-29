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
