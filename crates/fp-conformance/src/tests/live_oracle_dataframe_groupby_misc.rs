#[test]
fn live_oracle_dataframe_groupby_idxmin_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-068",
        "case_id": "dataframe_groupby_idxmin_live",
        "mode": "strict",
        "operation": "dataframe_groupby_idxmin",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" }
            ],
            "column_order": ["grp", "val", "all_na"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 4.0 }
                ],
                "all_na": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby idxmin oracle test: {message}"
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
fn live_oracle_dataframe_groupby_idxmax_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-069",
        "case_id": "dataframe_groupby_idxmax_live",
        "mode": "strict",
        "operation": "dataframe_groupby_idxmax",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" }
            ],
            "column_order": ["grp", "val", "all_na"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 4.0 }
                ],
                "all_na": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby idxmax oracle test: {message}"
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
fn live_oracle_dataframe_groupby_cumcount_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-070",
        "case_id": "dataframe_groupby_cumcount_live",
        "mode": "strict",
        "operation": "dataframe_groupby_cumcount",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" }
                ],
                "val": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 8.0 },
                    { "kind": "float64", "value": 3.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby cumcount oracle test: {message}"
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

    let actual = super::execute_dataframe_groupby_series_fixture_operation(&fixture, false)
        .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_groupby_ngroup_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-071",
        "case_id": "dataframe_groupby_ngroup_live",
        "mode": "strict",
        "operation": "dataframe_groupby_ngroup",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "c" },
                    { "kind": "utf8", "value": "a" }
                ],
                "val": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 8.0 },
                    { "kind": "float64", "value": 3.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby ngroup oracle test: {message}"
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

    let actual = super::execute_dataframe_groupby_series_fixture_operation(&fixture, true)
        .expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_groupby_any_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-072",
        "case_id": "dataframe_groupby_any_live",
        "mode": "strict",
        "operation": "dataframe_groupby_any",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" },
                { "kind": "utf8", "value": "r5" }
            ],
            "column_order": ["grp", "flag", "count", "all_na"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "c" },
                    { "kind": "utf8", "value": "c" }
                ],
                "flag": [
                    { "kind": "bool", "value": true },
                    { "kind": "bool", "value": false },
                    { "kind": "bool", "value": false },
                    { "kind": "bool", "value": false },
                    { "kind": "null", "value": "null" },
                    { "kind": "null", "value": "null" }
                ],
                "count": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "int64", "value": 3 }
                ],
                "all_na": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dataframe groupby any oracle test: {message}");
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
fn live_oracle_dataframe_groupby_all_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-073",
        "case_id": "dataframe_groupby_all_live",
        "mode": "strict",
        "operation": "dataframe_groupby_all",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" },
                { "kind": "utf8", "value": "r5" }
            ],
            "column_order": ["grp", "flag", "count", "all_na"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "c" },
                    { "kind": "utf8", "value": "c" }
                ],
                "flag": [
                    { "kind": "bool", "value": true },
                    { "kind": "bool", "value": false },
                    { "kind": "bool", "value": false },
                    { "kind": "bool", "value": false },
                    { "kind": "null", "value": "null" },
                    { "kind": "null", "value": "null" }
                ],
                "count": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "int64", "value": 3 }
                ],
                "all_na": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dataframe groupby all oracle test: {message}");
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
fn live_oracle_dataframe_groupby_get_group_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-084",
        "case_id": "dataframe_groupby_get_group_live",
        "mode": "strict",
        "operation": "dataframe_groupby_get_group",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "group_name": "a",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" }
                ],
                "val": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby get_group oracle test: {message}"
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
fn live_oracle_dataframe_groupby_ffill_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-079",
        "case_id": "dataframe_groupby_ffill_live",
        "mode": "strict",
        "operation": "dataframe_groupby_ffill",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" }
            ],
            "column_order": ["grp", "val", "other"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" }
                ],
                "val": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "null", "value": "na_n" }
                ],
                "other": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby ffill oracle test: {message}"
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
fn live_oracle_dataframe_groupby_bfill_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-080",
        "case_id": "dataframe_groupby_bfill_live",
        "mode": "strict",
        "operation": "dataframe_groupby_bfill",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" }
            ],
            "column_order": ["grp", "val", "other"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" }
                ],
                "val": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" }
                ],
                "other": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 9.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby bfill oracle test: {message}"
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
fn live_oracle_dataframe_groupby_sem_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-081",
        "case_id": "dataframe_groupby_sem_live",
        "mode": "strict",
        "operation": "dataframe_groupby_sem",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" }
            ],
            "column_order": ["grp", "v"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" }
                ],
                "v": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 6.0 },
                    { "kind": "float64", "value": 8.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping dataframe groupby sem oracle test: {message}");
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
fn live_oracle_dataframe_groupby_skew_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-082",
        "case_id": "dataframe_groupby_skew_live",
        "mode": "strict",
        "operation": "dataframe_groupby_skew",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" }
            ],
            "column_order": ["grp", "v"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" }
                ],
                "v": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 5.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby skew oracle test: {message}"
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
fn live_oracle_dataframe_groupby_kurtosis_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-083",
        "case_id": "dataframe_groupby_kurtosis_live",
        "mode": "strict",
        "operation": "dataframe_groupby_kurtosis",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" }
            ],
            "column_order": ["grp", "v"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" }
                ],
                "v": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 5.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby kurtosis oracle test: {message}"
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
fn live_oracle_dataframe_groupby_ohlc_single_value_column_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-084",
        "case_id": "dataframe_groupby_ohlc_single_live",
        "mode": "strict",
        "operation": "dataframe_groupby_ohlc",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" }
            ],
            "column_order": ["grp", "price"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" }
                ],
                "price": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 15.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 12.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby ohlc single-column oracle test: {message}"
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
fn live_oracle_dataframe_groupby_ohlc_multi_value_columns_matches_pandas() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-085",
        "case_id": "dataframe_groupby_ohlc_multi_live",
        "mode": "strict",
        "operation": "dataframe_groupby_ohlc",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "frame": {
            "index": [
                { "kind": "utf8", "value": "r0" },
                { "kind": "utf8", "value": "r1" },
                { "kind": "utf8", "value": "r2" },
                { "kind": "utf8", "value": "r3" },
                { "kind": "utf8", "value": "r4" }
            ],
            "column_order": ["grp", "price", "qty"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" }
                ],
                "price": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 15.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 12.0 },
                    { "kind": "float64", "value": 20.0 }
                ],
                "qty": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }
    }))
    .expect("fixture");

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby ohlc multi-column oracle test: {message}"
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
