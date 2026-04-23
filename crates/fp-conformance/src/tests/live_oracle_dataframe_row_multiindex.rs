fn assert_live_oracle_dataframe_frame_parity(
    fixture: super::PacketFixture,
    actual: fp_frame::DataFrame,
    context: &str,
) {
    let Some(expected) = live_oracle_expected_frame_or_skip(&fixture, context) else {
        return;
    };
    let comparison = super::compare_dataframe_expected(&actual, &expected);
    assert!(
        comparison.is_ok(),
        "{context}: pandas parity failed: {}",
        comparison.err().unwrap_or_default()
    );
}

fn assert_live_oracle_dataframe_fixture_parity(fixture: super::PacketFixture, context: &str) {
    let actual = super::execute_dataframe_fixture_operation(&fixture).expect("actual frame");
    assert_live_oracle_dataframe_frame_parity(fixture, actual, context);
}

fn row_multiindex_three_level_frame_json() -> serde_json::Value {
    serde_json::json!({
        "index": [
            { "kind": "utf8", "value": "north|apple|2023" },
            { "kind": "utf8", "value": "north|apple|2024" },
            { "kind": "utf8", "value": "north|pear|2023" },
            { "kind": "utf8", "value": "south|apple|2023" }
        ],
        "row_multiindex": {
            "tuples": [
                [
                    { "kind": "utf8", "value": "north" },
                    { "kind": "utf8", "value": "apple" },
                    { "kind": "int64", "value": 2023 }
                ],
                [
                    { "kind": "utf8", "value": "north" },
                    { "kind": "utf8", "value": "apple" },
                    { "kind": "int64", "value": 2024 }
                ],
                [
                    { "kind": "utf8", "value": "north" },
                    { "kind": "utf8", "value": "pear" },
                    { "kind": "int64", "value": 2023 }
                ],
                [
                    { "kind": "utf8", "value": "south" },
                    { "kind": "utf8", "value": "apple" },
                    { "kind": "int64", "value": 2023 }
                ]
            ],
            "names": ["region", "product", "year"]
        },
        "column_order": ["sales", "cost"],
        "columns": {
            "sales": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 15 },
                { "kind": "int64", "value": 30 }
            ],
            "cost": [
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 7 },
                { "kind": "int64", "value": 6 },
                { "kind": "int64", "value": 12 }
            ]
        }
    })
}

fn groupby_multikey_source_frame_json() -> serde_json::Value {
    serde_json::json!({
        "index": [
            { "kind": "int64", "value": 0 },
            { "kind": "int64", "value": 1 },
            { "kind": "int64", "value": 2 },
            { "kind": "int64", "value": 3 },
            { "kind": "int64", "value": 4 }
        ],
        "column_order": ["region", "product", "sales", "qty"],
        "columns": {
            "region": [
                { "kind": "utf8", "value": "north" },
                { "kind": "utf8", "value": "north" },
                { "kind": "utf8", "value": "north" },
                { "kind": "utf8", "value": "south" },
                { "kind": "utf8", "value": "south" }
            ],
            "product": [
                { "kind": "utf8", "value": "apple" },
                { "kind": "utf8", "value": "apple" },
                { "kind": "utf8", "value": "pear" },
                { "kind": "utf8", "value": "apple" },
                { "kind": "utf8", "value": "apple" }
            ],
            "sales": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 15 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 5 },
                { "kind": "int64", "value": 7 }
            ],
            "qty": [
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 6 }
            ]
        }
    })
}

fn live_oracle_expected_frame_or_skip(
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
        "expected live oracle frame payload for {context}, got {expected:?}"
    );
    let super::ResolvedExpected::Frame(expected) = expected else {
        return None;
    };
    Some(expected)
}

#[test]
fn live_oracle_dataframe_identity_row_multiindex_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-453",
        "case_id": "dataframe_identity_row_multiindex_live",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": row_multiindex_three_level_frame_json()
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_fixture_parity(
        fixture,
        "dataframe identity row multiindex oracle test",
    );
}

#[test]
fn live_oracle_dataframe_groupby_sum_multikey_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-454",
        "case_id": "dataframe_groupby_sum_multikey_live",
        "mode": "strict",
        "operation": "dataframe_groupby_sum",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["region", "product"],
        "frame": groupby_multikey_source_frame_json()
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_fixture_parity(
        fixture,
        "dataframe groupby multikey sum oracle test",
    );
}

#[test]
fn live_oracle_dataframe_groupby_agg_multi_multikey_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-455",
        "case_id": "dataframe_groupby_agg_multi_multikey_live",
        "mode": "strict",
        "operation": "dataframe_groupby_agg_multi",
        "oracle_source": "live_legacy_pandas",
        "groupby_columns": ["region", "product"],
        "groupby_agg_multi": {
            "sales": ["sum", "mean", "max"],
            "qty": ["count", "min"]
        },
        "frame": groupby_multikey_source_frame_json()
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_fixture_parity(
        fixture,
        "dataframe groupby multikey agg_multi oracle test",
    );
}

#[test]
fn live_oracle_dataframe_loc_prefix_row_multiindex_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-456",
        "case_id": "dataframe_loc_prefix_row_multiindex_live",
        "mode": "strict",
        "operation": "dataframe_loc",
        "oracle_source": "live_legacy_pandas",
        "loc_labels": [
            { "kind": "utf8", "value": "north" }
        ],
        "frame": row_multiindex_three_level_frame_json()
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_fixture_parity(
        fixture,
        "dataframe loc prefix row multiindex oracle test",
    );
}

#[test]
fn live_oracle_dataframe_xs_level_row_multiindex_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-457",
        "case_id": "dataframe_xs_level_row_multiindex_live",
        "mode": "strict",
        "operation": "dataframe_xs",
        "oracle_source": "live_legacy_pandas",
        "xs_key": { "kind": "utf8", "value": "north" },
        "xs_level": 0,
        "frame": row_multiindex_three_level_frame_json()
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_fixture_parity(
        fixture,
        "dataframe xs level row multiindex oracle test",
    );
}

#[test]
fn live_oracle_dataframe_reset_index_row_multiindex_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-458",
        "case_id": "dataframe_reset_index_row_multiindex_live",
        "mode": "strict",
        "operation": "dataframe_reset_index",
        "oracle_source": "live_legacy_pandas",
        "reset_index_drop": false,
        "frame": row_multiindex_three_level_frame_json()
    }))
    .expect("fixture");

    assert_live_oracle_dataframe_fixture_parity(
        fixture,
        "dataframe reset_index row multiindex oracle test",
    );
}

#[test]
fn live_oracle_dataframe_csv_row_multiindex_roundtrip_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-459",
        "case_id": "dataframe_csv_row_multiindex_roundtrip_live",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": row_multiindex_three_level_frame_json()
    }))
    .expect("fixture");

    let source = super::execute_dataframe_fixture_operation(&fixture).expect("source frame");
    let csv = fp_io::write_csv_string_with_options(
        &source,
        &fp_io::CsvWriteOptions {
            include_index: true,
            ..fp_io::CsvWriteOptions::default()
        },
    )
    .expect("write csv");
    let actual = fp_io::read_csv_with_index_cols(
        &csv,
        &super::CsvReadOptions::default(),
        &["region", "product", "year"],
    )
    .expect("read csv");

    assert_live_oracle_dataframe_frame_parity(
        fixture,
        actual,
        "dataframe csv row multiindex roundtrip oracle test",
    );
}

#[test]
fn live_oracle_dataframe_json_split_row_multiindex_roundtrip_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-460",
        "case_id": "dataframe_json_split_row_multiindex_roundtrip_live",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": row_multiindex_three_level_frame_json()
    }))
    .expect("fixture");

    let source = super::execute_dataframe_fixture_operation(&fixture).expect("source frame");
    let encoded = super::write_json_string(&source, super::JsonOrient::Split).expect("write json");
    let actual = super::read_json_str(&encoded, super::JsonOrient::Split).expect("read json");

    assert_live_oracle_dataframe_frame_parity(
        fixture,
        actual,
        "dataframe json split row multiindex roundtrip oracle test",
    );
}

#[test]
fn live_oracle_dataframe_parquet_row_multiindex_roundtrip_matches_pandas() {
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-461",
        "case_id": "dataframe_parquet_row_multiindex_roundtrip_live",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": row_multiindex_three_level_frame_json()
    }))
    .expect("fixture");

    let source = super::execute_dataframe_fixture_operation(&fixture).expect("source frame");
    let encoded = super::write_parquet_bytes(&source).expect("write parquet");
    let actual = super::read_parquet_bytes(&encoded).expect("read parquet");

    assert_live_oracle_dataframe_frame_parity(
        fixture,
        actual,
        "dataframe parquet row multiindex roundtrip oracle test",
    );
}
