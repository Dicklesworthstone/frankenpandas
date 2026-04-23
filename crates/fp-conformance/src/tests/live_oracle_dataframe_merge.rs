fn expected_live_oracle_dataframe_or_skip(
    fixture: &super::PacketFixture,
    context: &str,
) -> Option<super::FixtureExpectedDataFrame> {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let expected_result = super::capture_live_oracle_expected(&cfg, fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!("live pandas unavailable; skipping {context}: {message}");
        return None;
    }

    let expected = expected_result.expect("live oracle expected");
    assert!(
        matches!(&expected, super::ResolvedExpected::Frame(_)),
        "expected live oracle frame payload, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected) = expected else {
        return None;
    };
    Some(expected)
}

fn assert_live_oracle_dataframe_merge_ordered_parity(fixture: super::PacketFixture, context: &str) {
    let Some(expected) = expected_live_oracle_dataframe_or_skip(&fixture, context) else {
        return;
    };

    let actual =
        super::execute_dataframe_merge_ordered_fixture_operation(&fixture).expect("actual frame");
    super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
}

fn assert_live_oracle_dataframe_merge_asof_parity(fixture: super::PacketFixture, context: &str) {
    let Some(expected) = expected_live_oracle_dataframe_or_skip(&fixture, context) else {
        return;
    };

    let actual =
        super::execute_dataframe_merge_asof_fixture_operation(&fixture).expect("actual frame");
    super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_merge_ordered_ffill_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-068",
        "case_id": "dataframe_merge_ordered_ffill_live",
        "mode": "strict",
        "operation": "dataframe_merge_ordered",
        "oracle_source": "live_legacy_pandas",
        "merge_on": "date",
        "merge_fill_method": "ffill",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "columns": {
                "date": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 3 }
                ],
                "left_val": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "c" }
                ]
            }
        },
        "frame_right": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "columns": {
                "date": [
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "right_val": [
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "d" }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_merge_ordered_parity(fixture, "merge_ordered oracle test");
}

#[test]
fn live_oracle_dataframe_merge_ordered_without_fill_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-114",
        "case_id": "dataframe_merge_ordered_no_fill_live",
        "mode": "strict",
        "operation": "dataframe_merge_ordered",
        "oracle_source": "live_legacy_pandas",
        "merge_on": "date",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["date", "left_val", "left_note"],
            "columns": {
                "date": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 4 }
                ],
                "left_val": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 40.0 }
                ],
                "left_note": [
                    { "kind": "utf8", "value": "alpha" },
                    { "kind": "utf8", "value": "beta" },
                    { "kind": "utf8", "value": "delta" }
                ]
            }
        },
        "frame_right": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["date", "right_val", "right_note"],
            "columns": {
                "date": [
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 5 }
                ],
                "right_val": [
                    { "kind": "float64", "value": 200.0 },
                    { "kind": "float64", "value": 300.0 },
                    { "kind": "float64", "value": 500.0 }
                ],
                "right_note": [
                    { "kind": "utf8", "value": "two" },
                    { "kind": "utf8", "value": "three" },
                    { "kind": "utf8", "value": "five" }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_merge_ordered_parity(fixture, "merge_ordered no-fill oracle test");
}

#[test]
fn live_oracle_dataframe_merge_ordered_multi_key_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-114",
        "case_id": "dataframe_merge_ordered_multi_key_live",
        "mode": "strict",
        "operation": "dataframe_merge_ordered",
        "oracle_source": "live_legacy_pandas",
        "merge_on_keys": ["group", "date"],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "column_order": ["group", "date", "left_val"],
            "columns": {
                "group": [
                    { "kind": "utf8", "value": "A" },
                    { "kind": "utf8", "value": "A" },
                    { "kind": "utf8", "value": "B" },
                    { "kind": "utf8", "value": "B" }
                ],
                "date": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 4 }
                ],
                "left_val": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 30.0 },
                    { "kind": "float64", "value": 100.0 },
                    { "kind": "float64", "value": 400.0 }
                ]
            }
        },
        "frame_right": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "column_order": ["group", "date", "right_val"],
            "columns": {
                "group": [
                    { "kind": "utf8", "value": "A" },
                    { "kind": "utf8", "value": "A" },
                    { "kind": "utf8", "value": "B" },
                    { "kind": "utf8", "value": "B" }
                ],
                "date": [
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ],
                "right_val": [
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 30.0 },
                    { "kind": "float64", "value": 1000.0 },
                    { "kind": "float64", "value": 2000.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_merge_ordered_parity(
        fixture,
        "merge_ordered multi-key oracle test",
    );
}

#[test]
fn live_oracle_dataframe_merge_asof_allow_exact_matches_false_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-056",
        "case_id": "dataframe_merge_asof_allow_exact_matches_false_live",
        "mode": "strict",
        "operation": "dataframe_merge_asof",
        "oracle_source": "live_legacy_pandas",
        "merge_on": "time",
        "direction": "backward",
        "allow_exact_matches": false,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["time", "val"],
            "columns": {
                "time": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "val": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 30.0 }
                ]
            }
        },
        "frame_right": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["time", "quote"],
            "columns": {
                "time": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "quote": [
                    { "kind": "float64", "value": 100.0 },
                    { "kind": "float64", "value": 200.0 },
                    { "kind": "float64", "value": 300.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_merge_asof_parity(fixture, "merge_asof exact-match oracle test");
}

#[test]
fn live_oracle_dataframe_merge_asof_forward_tolerance_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-056",
        "case_id": "dataframe_merge_asof_forward_tolerance_live",
        "mode": "strict",
        "operation": "dataframe_merge_asof",
        "oracle_source": "live_legacy_pandas",
        "merge_on": "time",
        "direction": "forward",
        "tolerance": 2.5,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["time", "val"],
            "columns": {
                "time": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 5 },
                    { "kind": "int64", "value": 10 }
                ],
                "val": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 50.0 },
                    { "kind": "float64", "value": 100.0 }
                ]
            }
        },
        "frame_right": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["time", "quote"],
            "columns": {
                "time": [
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 6 },
                    { "kind": "int64", "value": 15 }
                ],
                "quote": [
                    { "kind": "float64", "value": 200.0 },
                    { "kind": "float64", "value": 600.0 },
                    { "kind": "float64", "value": 1500.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_merge_asof_parity(
        fixture,
        "merge_asof forward+tolerance oracle test",
    );
}

#[test]
fn live_oracle_dataframe_merge_asof_by_group_and_no_exact_matches_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-056",
        "case_id": "dataframe_merge_asof_by_group_no_exact_live",
        "mode": "strict",
        "operation": "dataframe_merge_asof",
        "oracle_source": "live_legacy_pandas",
        "merge_on": "time",
        "direction": "backward",
        "allow_exact_matches": false,
        "by": ["group"],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "column_order": ["group", "time", "val"],
            "columns": {
                "group": [
                    { "kind": "utf8", "value": "A" },
                    { "kind": "utf8", "value": "B" },
                    { "kind": "utf8", "value": "A" },
                    { "kind": "utf8", "value": "B" }
                ],
                "time": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 5 }
                ],
                "val": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "float64", "value": 30.0 },
                    { "kind": "float64", "value": 50.0 }
                ]
            }
        },
        "frame_right": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "column_order": ["group", "time", "quote"],
            "columns": {
                "group": [
                    { "kind": "utf8", "value": "A" },
                    { "kind": "utf8", "value": "B" },
                    { "kind": "utf8", "value": "A" },
                    { "kind": "utf8", "value": "B" }
                ],
                "time": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 4 },
                    { "kind": "int64", "value": 4 }
                ],
                "quote": [
                    { "kind": "float64", "value": 100.0 },
                    { "kind": "float64", "value": 200.0 },
                    { "kind": "float64", "value": 400.0 },
                    { "kind": "float64", "value": 450.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_merge_asof_parity(fixture, "grouped merge_asof oracle test");
}
