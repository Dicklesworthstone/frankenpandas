use super::*;

fn assert_live_oracle_dataframe_groupby_resample_frame_parity(fixture: super::PacketFixture) {
    let mut cfg = HarnessConfig::default_paths();
    // br-frankenpandas-l7r1p: opt into the SYSTEM pandas 2.2.3. This was the
    // never-converted default, not a recorded divergence -- with it false the
    // absent legacy oracle root made every test in this file skip.
    cfg.allow_system_pandas_fallback = true;

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe groupby resample oracle test {}: {message}",
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

    let actual = super::execute_dataframe_groupby_resample_fixture_operation(&fixture)
        .expect("actual frame");
    super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_groupby_resample_min_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-441",
        "case_id": "dataframe_groupby_resample_min_live",
        "mode": "strict",
        "operation": "dataframe_groupby_resample_min",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "resample_freq": "M",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "2024-01-01" },
                { "kind": "utf8", "value": "2024-01-15" },
                { "kind": "utf8", "value": "2024-02-01" },
                { "kind": "utf8", "value": "2024-02-20" },
                { "kind": "utf8", "value": "2024-01-05" },
                { "kind": "utf8", "value": "2024-02-05" },
                { "kind": "utf8", "value": "2024-02-25" }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 9.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_resample_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_resample_max_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-442",
        "case_id": "dataframe_groupby_resample_max_live",
        "mode": "strict",
        "operation": "dataframe_groupby_resample_max",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "resample_freq": "M",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "2024-01-01" },
                { "kind": "utf8", "value": "2024-01-15" },
                { "kind": "utf8", "value": "2024-02-01" },
                { "kind": "utf8", "value": "2024-02-20" },
                { "kind": "utf8", "value": "2024-01-05" },
                { "kind": "utf8", "value": "2024-02-05" },
                { "kind": "utf8", "value": "2024-02-25" }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 9.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_resample_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_resample_count_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-443",
        "case_id": "dataframe_groupby_resample_count_live",
        "mode": "strict",
        "operation": "dataframe_groupby_resample_count",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "resample_freq": "M",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "2024-01-01" },
                { "kind": "utf8", "value": "2024-01-15" },
                { "kind": "utf8", "value": "2024-02-10" },
                { "kind": "utf8", "value": "2024-03-05" },
                { "kind": "utf8", "value": "2024-01-20" },
                { "kind": "utf8", "value": "2024-02-02" },
                { "kind": "utf8", "value": "2024-02-25" }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 8.0 },
                    { "kind": "float64", "value": 7.0 },
                    { "kind": "float64", "value": 9.0 },
                    { "kind": "float64", "value": 4.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_resample_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_resample_first_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-451",
        "case_id": "dataframe_groupby_resample_first_live",
        "mode": "strict",
        "operation": "dataframe_groupby_resample_first",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "resample_freq": "M",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "2024-01-01" },
                { "kind": "utf8", "value": "2024-01-20" },
                { "kind": "utf8", "value": "2024-02-01" },
                { "kind": "utf8", "value": "2024-02-25" },
                { "kind": "utf8", "value": "2024-01-05" },
                { "kind": "utf8", "value": "2024-01-25" },
                { "kind": "utf8", "value": "2024-02-05" },
                { "kind": "utf8", "value": "2024-02-20" }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 8.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 7.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_resample_frame_parity(fixture);
}

#[test]
fn live_oracle_dataframe_groupby_resample_last_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-452",
        "case_id": "dataframe_groupby_resample_last_live",
        "mode": "strict",
        "operation": "dataframe_groupby_resample_last",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["grp"],
        "resample_freq": "M",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "2024-01-01" },
                { "kind": "utf8", "value": "2024-01-20" },
                { "kind": "utf8", "value": "2024-02-01" },
                { "kind": "utf8", "value": "2024-02-25" },
                { "kind": "utf8", "value": "2024-01-05" },
                { "kind": "utf8", "value": "2024-01-25" },
                { "kind": "utf8", "value": "2024-02-05" },
                { "kind": "utf8", "value": "2024-02-20" }
            ],
            "column_order": ["grp", "val"],
            "columns": {
                "grp": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" }
                ],
                "val": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "float64", "value": 20.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 8.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 7.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_groupby_resample_frame_parity(fixture);
}
