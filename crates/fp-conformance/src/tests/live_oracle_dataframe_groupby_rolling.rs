use super::*;

fn assert_live_oracle_dataframe_groupby_rolling_frame_parity(fixture: super::PacketFixture) {
    let mut cfg = HarnessConfig::default_paths();
    // br-frankenpandas-l7r1p: opt into the SYSTEM pandas 2.2.3. This was the
    // never-converted default, not a recorded divergence -- with it false the
    // absent legacy oracle root made every test in this file skip.
    cfg.allow_system_pandas_fallback = true;

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby rolling oracle test {}: {message}",
            fixture.case_id
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

    let actual =
        super::execute_dataframe_groupby_rolling_fixture_operation(&fixture).expect("actual frame");
    super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_groupby_rolling_mean_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-444",
        "case_id": "dataframe_groupby_rolling_mean_live",
        "mode": "strict",
        "operation": "dataframe_groupby_rolling_mean",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "window_size": 2,
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
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 30.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "float64", "value": 50.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_rolling_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_rolling_sum_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-445",
        "case_id": "dataframe_groupby_rolling_sum_live",
        "mode": "strict",
        "operation": "dataframe_groupby_rolling_sum",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "window_size": 2,
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
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 8.0 },
                    { "kind": "float64", "value": 11.0 },
                    { "kind": "float64", "value": 16.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 32.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_rolling_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_rolling_min_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-446",
        "case_id": "dataframe_groupby_rolling_min_live",
        "mode": "strict",
        "operation": "dataframe_groupby_rolling_min",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "window_size": 2,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 6 }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" }
                ],
                "val": [
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 6.0 },
                    { "kind": "float64", "value": 4.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_rolling_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_rolling_max_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-447",
        "case_id": "dataframe_groupby_rolling_max_live",
        "mode": "strict",
        "operation": "dataframe_groupby_rolling_max",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "window_size": 2,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 6 }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" }
                ],
                "val": [
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 6.0 },
                    { "kind": "float64", "value": 4.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_rolling_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_rolling_count_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-448",
        "case_id": "dataframe_groupby_rolling_count_live",
        "mode": "strict",
        "operation": "dataframe_groupby_rolling_count",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "window_size": 2,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 6 }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" }
                ],
                "val": [
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 6.0 },
                    { "kind": "float64", "value": 4.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_rolling_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_rolling_std_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-449",
        "case_id": "dataframe_groupby_rolling_std_live",
        "mode": "strict",
        "operation": "dataframe_groupby_rolling_std",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "window_size": 3,
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
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 30.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 9.0 },
                    { "kind": "float64", "value": 50.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_rolling_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_rolling_var_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-450",
        "case_id": "dataframe_groupby_rolling_var_live",
        "mode": "strict",
        "operation": "dataframe_groupby_rolling_var",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "window_size": 3,
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
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 4.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 8.0 },
                    { "kind": "float64", "value": 11.0 },
                    { "kind": "float64", "value": 16.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 32.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_rolling_frame_parity(fixture);
}
