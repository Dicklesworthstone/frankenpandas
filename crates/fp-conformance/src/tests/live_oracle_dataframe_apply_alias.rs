use super::*;

fn assert_live_oracle_dataframe_apply_alias_series_parity(fixture: super::PacketFixture) {
    let mut cfg = HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = false;

    let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
        eprintln!(
            "live pandas unavailable; skipping dataframe apply alias oracle test {}: {message}",
            fixture.case_id
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

    let actual =
        super::execute_dataframe_apply_alias_fixture_operation(&fixture).expect("actual series");
    super::compare_series_expected(&actual, &expected).expect("pandas parity");
}

#[test]
fn live_oracle_dataframe_apply_sem_axis0_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-434",
        "case_id": "dataframe_apply_sem_axis0_live",
        "mode": "strict",
        "operation": "dataframe_apply_sem_axis0",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "column_order": ["x", "y"],
            "columns": {
                "x": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 }
                ],
                "y": [
                    { "kind": "float64", "value": 10.0 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_apply_alias_series_parity(fixture);
}

#[test]
fn live_oracle_dataframe_apply_nunique_axis0_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-435",
        "case_id": "dataframe_apply_nunique_axis0_live",
        "mode": "strict",
        "operation": "dataframe_apply_nunique_axis0",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["a", "b"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "null", "value": "na_n" }
                ],
                "b": [
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_apply_alias_series_parity(fixture);
}

#[test]
fn live_oracle_dataframe_apply_prod_axis1_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-436",
        "case_id": "dataframe_apply_prod_axis1_live",
        "mode": "strict",
        "operation": "dataframe_apply_prod_axis1",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a", "b", "c"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "null", "value": "na_n" }
                ],
                "b": [
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 }
                ],
                "c": [
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 6.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_apply_alias_series_parity(fixture);
}

#[test]
fn live_oracle_dataframe_apply_product_axis1_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-437",
        "case_id": "dataframe_apply_product_axis1_live",
        "mode": "strict",
        "operation": "dataframe_apply_product_axis1",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a", "b", "c"],
            "columns": {
                "a": [
                    { "kind": "float64", "value": 2.0 },
                    { "kind": "null", "value": "na_n" }
                ],
                "b": [
                    { "kind": "float64", "value": 3.0 },
                    { "kind": "float64", "value": 4.0 }
                ],
                "c": [
                    { "kind": "float64", "value": 5.0 },
                    { "kind": "float64", "value": 6.0 }
                ]
            }
        }
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_apply_alias_series_parity(fixture);
}
