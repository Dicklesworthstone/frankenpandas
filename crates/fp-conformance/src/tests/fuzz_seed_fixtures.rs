//! Fuzz-seed fixture tests extracted from fp-conformance/src/lib.rs's
//! `mod tests` per br-frankenpandas-lxhr monolith-split slice.
//!
//! Each test loads a pre-recorded seed from
//! `crates/fp-conformance/fixtures/adversarial/fuzz_corpus/<target>/`
//! and asserts that the corresponding crate-level `fuzz_*_bytes` entrypoint
//! either accepts it (the happy-path seeds) or reports the expected
//! typed error (the `*_reports_*` negative seeds).

use super::*;

#[test]
fn fuzz_fixture_parse_bytes_accepts_valid_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_fixture_parse/series_add_valid_seed.json"
    );
    fuzz_fixture_parse_bytes(seed).expect("valid fuzz seed should parse");
}

#[test]
fn fuzz_fixture_parse_bytes_accepts_expected_error_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_fixture_parse/dataframe_constructor_missing_matrix_rows_seed.json"
    );
    fuzz_fixture_parse_bytes(seed).expect("expected-error fuzz seed should parse");
}

#[test]
fn fuzz_fixture_parse_bytes_reports_invalid_json() {
    let err = fuzz_fixture_parse_bytes(br#"{"packet_id": "oops""#)
        .expect_err("invalid json should error");
    assert!(
        matches!(err, super::HarnessError::Json(_)),
        "expected JSON parse error, got {err:?}"
    );
}

#[test]
fn fuzz_csv_parse_bytes_accepts_simple_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/csv_parse/simple_valid_seed.csv");
    fuzz_csv_parse_bytes(seed).expect("simple csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_quoted_newline_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/csv_parse/quoted_newline_valid_seed.csv"
    );
    fuzz_csv_parse_bytes(seed).expect("quoted newline csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_reports_duplicate_headers() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/csv_parse/duplicate_headers_invalid_seed.csv"
    );
    let err = fuzz_csv_parse_bytes(seed).expect_err("duplicate csv headers should error");
    assert!(
        matches!(err, fp_io::IoError::DuplicateColumnName(_)),
        "expected duplicate header error, got {err:?}"
    );
}

#[test]
fn fuzz_csv_parse_bytes_accepts_empty_cells_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/csv_parse/empty_cells_seed.csv");
    fuzz_csv_parse_bytes(seed).expect("empty cells csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_numeric_columns_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/csv_parse/numeric_columns_seed.csv");
    fuzz_csv_parse_bytes(seed).expect("numeric columns csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_escaped_quotes_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/csv_parse/escaped_quotes_seed.csv");
    fuzz_csv_parse_bytes(seed).expect("escaped quotes csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_single_column_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/csv_parse/single_column_seed.csv");
    fuzz_csv_parse_bytes(seed).expect("single column csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_header_only_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/csv_parse/header_only_seed.csv");
    fuzz_csv_parse_bytes(seed).expect("header-only csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_trailing_whitespace_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/csv_parse/trailing_whitespace_seed.csv"
    );
    fuzz_csv_parse_bytes(seed).expect("trailing whitespace csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_many_columns_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/csv_parse/many_columns_seed.csv");
    fuzz_csv_parse_bytes(seed).expect("many columns csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_quoted_comma_in_value_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/csv_parse/quoted_comma_in_value_seed.csv"
    );
    fuzz_csv_parse_bytes(seed).expect("quoted comma csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_trailing_empty_field_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/csv_parse/trailing_empty_field_seed.csv"
    );
    fuzz_csv_parse_bytes(seed).expect("trailing empty field csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_single_quote_in_value_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/csv_parse/single_quote_in_value_seed.csv"
    );
    fuzz_csv_parse_bytes(seed).expect("single-quote csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_all_empty_cells_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/csv_parse/all_empty_cells_seed.csv"
    );
    fuzz_csv_parse_bytes(seed).expect("all-empty-cells csv fuzz seed should parse");
}

#[test]
fn fuzz_csv_parse_bytes_accepts_single_int_column_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/csv_parse/single_int_column_seed.csv"
    );
    fuzz_csv_parse_bytes(seed).expect("single-int-column csv fuzz seed should parse");
}

#[test]
fn fuzz_excel_io_bytes_accepts_valid_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/excel_io/simple_valid_seed.xlsx");
    fuzz_excel_io_bytes(seed).expect("excel fuzz seed should parse");
}

#[test]
fn fuzz_excel_io_bytes_reports_invalid_workbook() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/excel_io/invalid_text_seed.bin");
    let err = fuzz_excel_io_bytes(seed).expect_err("invalid workbook bytes should error");
    assert!(
        matches!(err, fp_io::IoError::Excel(_)),
        "expected Excel parse error, got {err:?}"
    );
}

#[test]
fn fuzz_parquet_io_bytes_accepts_synthesized_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/parquet_io/synthesized_valid_seed.bin"
    );
    fuzz_parquet_io_bytes(seed).expect("synthesized parquet seed should parse");
}

#[test]
fn fuzz_parquet_io_bytes_accepts_runtime_raw_parquet_bytes() {
    let frame = fp_frame::DataFrame::from_dict(
        &["ints", "bools"],
        vec![
            (
                "ints",
                vec![
                    fp_types::Scalar::Int64(5),
                    fp_types::Scalar::Null(fp_types::NullKind::Null),
                    fp_types::Scalar::Int64(-1),
                ],
            ),
            (
                "bools",
                vec![
                    fp_types::Scalar::Bool(true),
                    fp_types::Scalar::Null(fp_types::NullKind::Null),
                    fp_types::Scalar::Bool(false),
                ],
            ),
        ],
    )
    .expect("frame");
    let mut seed = vec![0];
    seed.extend(fp_io::write_parquet_bytes(&frame).expect("write parquet bytes"));

    fuzz_parquet_io_bytes(&seed).expect("raw parquet payload should parse");
}

#[test]
fn fuzz_parquet_io_bytes_reports_invalid_raw_bytes() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/parquet_io/invalid_text_seed.bin");
    let err = fuzz_parquet_io_bytes(seed).expect_err("invalid parquet bytes should error");
    assert!(
        matches!(err, fp_io::IoError::Parquet(_)),
        "expected Parquet parse error, got {err:?}"
    );
}

#[test]
fn fuzz_feather_io_bytes_accepts_synthesized_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/feather_io/synthesized_valid_seed.bin"
    );
    fuzz_feather_io_bytes(seed).expect("synthesized feather seed should parse");
}

#[test]
fn fuzz_feather_io_bytes_accepts_runtime_raw_feather_bytes() {
    let frame = fp_frame::DataFrame::from_dict(
        &["ints", "floats"],
        vec![
            (
                "ints",
                vec![
                    fp_types::Scalar::Int64(7),
                    fp_types::Scalar::Null(fp_types::NullKind::Null),
                    fp_types::Scalar::Int64(-3),
                ],
            ),
            (
                "floats",
                vec![
                    fp_types::Scalar::Float64(1.5),
                    fp_types::Scalar::Null(fp_types::NullKind::NaN),
                    fp_types::Scalar::Float64(-0.0),
                ],
            ),
        ],
    )
    .expect("frame");
    let mut seed = vec![0];
    seed.extend(fp_io::write_feather_bytes(&frame).expect("write feather bytes"));

    fuzz_feather_io_bytes(&seed).expect("raw feather payload should parse");
}

#[test]
fn fuzz_feather_io_bytes_reports_invalid_raw_bytes() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/feather_io/invalid_text_seed.bin");
    let err = fuzz_feather_io_bytes(seed).expect_err("invalid feather bytes should error");
    assert!(
        matches!(err, fp_io::IoError::Arrow(_)),
        "expected Arrow parse error, got {err:?}"
    );
}

#[test]
fn fuzz_ipc_stream_io_bytes_accepts_synthesized_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/ipc_stream_io/synthesized_valid_seed.bin"
    );
    fuzz_ipc_stream_io_bytes(seed).expect("synthesized IPC stream seed should parse");
}

#[test]
fn fuzz_ipc_stream_io_bytes_accepts_runtime_raw_ipc_stream_bytes() {
    let frame = fp_frame::DataFrame::from_dict(
        &["ints", "strings"],
        vec![
            (
                "ints",
                vec![
                    fp_types::Scalar::Int64(4),
                    fp_types::Scalar::Null(fp_types::NullKind::Null),
                    fp_types::Scalar::Int64(-2),
                ],
            ),
            (
                "strings",
                vec![
                    fp_types::Scalar::Utf8("alpha".to_owned()),
                    fp_types::Scalar::Null(fp_types::NullKind::Null),
                    fp_types::Scalar::Utf8("beta".to_owned()),
                ],
            ),
        ],
    )
    .expect("frame");
    let mut seed = vec![0];
    seed.extend(fp_io::write_ipc_stream_bytes(&frame).expect("write IPC stream bytes"));

    fuzz_ipc_stream_io_bytes(&seed).expect("raw IPC stream payload should parse");
}

#[test]
fn fuzz_ipc_stream_io_bytes_reports_invalid_raw_bytes() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/ipc_stream_io/invalid_text_seed.bin"
    );
    let err = fuzz_ipc_stream_io_bytes(seed).expect_err("invalid IPC stream bytes should error");
    assert!(
        matches!(err, fp_io::IoError::Arrow(_)),
        "expected Arrow parse error, got {err:?}"
    );
}

#[test]
fn fuzz_read_sql_bytes_accepts_indexed_query_dispatch_seed() {
    let mut seed = vec![0xff, 0x15];
    seed.extend(b"SELECT a, b FROM t1 ORDER BY a");
    fuzz_read_sql_bytes(&seed).expect("indexed query SQL fuzz seed should parse");
}

#[test]
fn fuzz_read_sql_bytes_accepts_empty_index_col_dispatch_seed() {
    let mut seed = vec![0xff, 0x35];
    seed.extend(b"SELECT a, b FROM t1 ORDER BY a");
    fuzz_read_sql_bytes(&seed).expect("empty index_col SQL fuzz seed should not panic");
}

#[test]
fn fuzz_read_sql_bytes_accepts_indexed_table_dispatch_seed() {
    let seed = [0xff, 0x26, b'x'];
    fuzz_read_sql_bytes(&seed).expect("indexed table SQL fuzz seed should parse");
}

#[test]
fn fuzz_read_sql_bytes_replays_committed_corpus_seeds() {
    let corpus: &[(&str, &[u8])] = &[
        (
            "aggregate_ordered_query.sql",
            include_bytes!("../../../../fuzz/corpus/fuzz_sql_read/aggregate_ordered_query.sql"),
        ),
        (
            "commented_indexed_t2_query.sql",
            include_bytes!("../../../../fuzz/corpus/fuzz_sql_read/commented_indexed_t2_query.sql"),
        ),
        (
            "derived_column_projection.sql",
            include_bytes!("../../../../fuzz/corpus/fuzz_sql_read/derived_column_projection.sql"),
        ),
        (
            "join_case_mapping.sql",
            include_bytes!("../../../../fuzz/corpus/fuzz_sql_read/join_case_mapping.sql"),
        ),
        (
            "mode1_filtered_projection.sql",
            include_bytes!("../../../../fuzz/corpus/fuzz_sql_read/mode1_filtered_projection.sql"),
        ),
        (
            "mode1_t2_desc_projection.sql",
            include_bytes!("../../../../fuzz/corpus/fuzz_sql_read/mode1_t2_desc_projection.sql"),
        ),
        (
            "mode2_leading_newline_projection.sql",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_sql_read/mode2_leading_newline_projection.sql"
            ),
        ),
        (
            "mode5_indexed_join_aliases.sql",
            include_bytes!("../../../../fuzz/corpus/fuzz_sql_read/mode5_indexed_join_aliases.sql"),
        ),
        (
            "table_mode6_t2",
            include_bytes!("../../../../fuzz/corpus/fuzz_sql_read/table_mode6_t2"),
        ),
        (
            "table_mode7_t2_with_options",
            include_bytes!("../../../../fuzz/corpus/fuzz_sql_read/table_mode7_t2_with_options"),
        ),
    ];

    for (name, seed) in corpus {
        let result = fuzz_read_sql_bytes(seed);
        assert!(
            result.is_ok(),
            "SQL fuzz corpus seed {name} should parse: {result:?}"
        );
    }
}

#[test]
fn fuzz_format_cross_round_trip_bytes_accepts_all_arrow_format_pairs() {
    let payload = [
        3, 2, 0, 1, 2, 3, 10, 20, 11, 21, 12, 22, 13, 23, 14, 24, 15, 25, 16, 26, 17, 27, 18, 28,
        19, 29, 20, 30, 21, 31, 22, 32, 23, 33, 24, 34,
    ];
    let pairs = [(0_u8, 0_u8), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1)];
    for (primary, secondary) in pairs {
        let mut seed = vec![primary, secondary];
        seed.extend(payload);
        fuzz_format_cross_round_trip_bytes(&seed).expect("cross-format seed should converge");
    }
}

#[test]
fn fuzz_format_cross_round_trip_bytes_accepts_empty_input() {
    fuzz_format_cross_round_trip_bytes(&[]).expect("empty input should be a no-op");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_empty_input() {
    fuzz_pivot_table_bytes(&[]).expect("empty input should be a no-op");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_all_supported_aggfuncs() {
    let payload = [
        7, 1, 10, 20, 30, 40, 50, 2, 11, 21, 31, 41, 51, 3, 12, 22, 32, 42, 52, 4, 13, 23, 33, 43,
        53, 5, 14, 24, 34, 44, 54, 6, 15, 25, 35, 45, 55, 7, 16, 26, 36, 46, 56,
    ];
    for agg_tag in 0..FUZZ_PIVOT_AGGFUNCS.len() {
        let mut seed = vec![1, agg_tag as u8];
        seed.extend(payload);
        fuzz_pivot_table_bytes(&seed)
            .expect("pivot_table fuzz seed should satisfy invariants for all aggfuncs");
    }
}

#[test]
fn fuzz_pivot_table_bytes_accepts_raw_projection_mode() {
    let seed = [
        0, 3, 6, 0, 1, 0, 5, 0, 2, 1, 6, 1, 7, 2, 0, 2, 8, 3, 9, 1, 1, 4, 10, 2, 11, 0, 12, 3, 13,
        1, 14, 2,
    ];
    fuzz_pivot_table_bytes(&seed)
        .expect("raw projection mode should satisfy pivot_table invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_sum_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_sum.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_sum seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_mean_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_mean.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_mean seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_count_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_count.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_count seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_min_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_min.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_min seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_max_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_max.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_max seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_prod_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_prod.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_prod seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_median_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_median.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_median seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_var_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_var.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_var seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_std_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_std.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_std seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_aggfunc_first_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/aggfunc_first.bin");
    fuzz_pivot_table_bytes(seed).expect("aggfunc_first seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_single_row_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/single_row.bin");
    fuzz_pivot_table_bytes(seed).expect("single_row seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_all_nulls_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/all_nulls.bin");
    fuzz_pivot_table_bytes(seed).expect("all_nulls seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_same_cell_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/same_cell.bin");
    fuzz_pivot_table_bytes(seed).expect("same_cell seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_synth_mode_sum_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/synth_mode_sum.bin");
    fuzz_pivot_table_bytes(seed).expect("synth_mode_sum seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_negative_vals_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/negative_vals.bin");
    fuzz_pivot_table_bytes(seed).expect("negative_vals seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_float_vals_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/float_vals.bin");
    fuzz_pivot_table_bytes(seed).expect("float_vals seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_mixed_nulls_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/mixed_nulls.bin");
    fuzz_pivot_table_bytes(seed).expect("mixed_nulls seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_zeros_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/zeros.bin");
    fuzz_pivot_table_bytes(seed).expect("zeros seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_max_rows_raw_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/max_rows_raw.bin");
    fuzz_pivot_table_bytes(seed).expect("max_rows_raw seed should satisfy invariants");
}

#[test]
fn fuzz_pivot_table_bytes_accepts_synth_max_rows_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/pivot_table/synth_max_rows.bin");
    fuzz_pivot_table_bytes(seed).expect("synth_max_rows seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_empty_input() {
    fuzz_rolling_window_bytes(&[]).expect("empty input should be a no-op");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_window_zero_seed() {
    let seed = [0, 0, 6, 1, 100, 2, 110, 0, 0, 3, 120];
    fuzz_rolling_window_bytes(&seed)
        .expect("window=0 rolling seed should reject cleanly without panicking");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_min_periods_gt_window_seed() {
    let seed = [1, 4, 5, 1, 100, 2, 110, 3, 120, 4, 130, 1, 140];
    fuzz_rolling_window_bytes(&seed)
        .expect("min_periods > window seed should stay bounded and non-panicking");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_window_0_min_none() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/rolling_window/window_0_min_none.bin"
    );
    fuzz_rolling_window_bytes(seed).expect("window_0_min_none seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_window_1_min_1() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/window_1_min_1.bin");
    fuzz_rolling_window_bytes(seed).expect("window_1_min_1 seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_window_len_min_none() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/rolling_window/window_len_min_none.bin"
    );
    fuzz_rolling_window_bytes(seed).expect("window_len_min_none seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_window_len_plus1() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/rolling_window/window_len_plus1.bin"
    );
    fuzz_rolling_window_bytes(seed).expect("window_len_plus1 seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_window_max() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/window_max.bin");
    fuzz_rolling_window_bytes(seed).expect("window_max seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_all_nulls() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/all_nulls.bin");
    fuzz_rolling_window_bytes(seed).expect("all_nulls seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_no_nulls() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/no_nulls.bin");
    fuzz_rolling_window_bytes(seed).expect("no_nulls seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_alternating_nulls() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/rolling_window/alternating_nulls.bin"
    );
    fuzz_rolling_window_bytes(seed).expect("alternating_nulls seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_single_value() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/single_value.bin");
    fuzz_rolling_window_bytes(seed).expect("single_value seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_increasing() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/increasing.bin");
    fuzz_rolling_window_bytes(seed).expect("increasing seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_decreasing() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/decreasing.bin");
    fuzz_rolling_window_bytes(seed).expect("decreasing seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_large_series() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/large_series.bin");
    fuzz_rolling_window_bytes(seed).expect("large_series seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_min_periods_0() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/min_periods_0.bin");
    fuzz_rolling_window_bytes(seed).expect("min_periods_0 seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_min_periods_window() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/rolling_window/min_periods_window.bin"
    );
    fuzz_rolling_window_bytes(seed).expect("min_periods_window seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_min_periods_exceeded() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/rolling_window/min_periods_exceeded.bin"
    );
    fuzz_rolling_window_bytes(seed).expect("min_periods_exceeded seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_negative_values() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/negative_values.bin");
    fuzz_rolling_window_bytes(seed).expect("negative_values seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_zeros() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/zeros.bin");
    fuzz_rolling_window_bytes(seed).expect("zeros seed should satisfy invariants");
}

#[test]
fn fuzz_rolling_window_bytes_accepts_mixed_signs() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/rolling_window/mixed_signs.bin");
    fuzz_rolling_window_bytes(seed).expect("mixed_signs seed should satisfy invariants");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_empty_input() {
    fuzz_semantic_eq_bytes(&[]).expect("empty input should be a no-op");
}

#[test]
fn fuzz_semantic_eq_bytes_locks_nan_missing_bridge() {
    fuzz_semantic_eq_bytes(&[3, 0, 3, 0]).expect("nan/missing bridge seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_int_same_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/int_same.bin");
    fuzz_semantic_eq_bytes(seed).expect("int_same seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_int_diff_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/int_diff.bin");
    fuzz_semantic_eq_bytes(seed).expect("int_diff seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_float_same_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/float_same.bin");
    fuzz_semantic_eq_bytes(seed).expect("float_same seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_float_diff_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/float_diff.bin");
    fuzz_semantic_eq_bytes(seed).expect("float_diff seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_float_nan_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/float_nan.bin");
    fuzz_semantic_eq_bytes(seed).expect("float_nan seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_float_nan_vs_null_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/semantic_eq/float_nan_vs_null.bin"
    );
    fuzz_semantic_eq_bytes(seed).expect("nan vs null seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_null_null_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/null_null.bin");
    fuzz_semantic_eq_bytes(seed).expect("null_null seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_null_nan_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/null_nan.bin");
    fuzz_semantic_eq_bytes(seed).expect("null_nan seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_nan_nat_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/nan_nat.bin");
    fuzz_semantic_eq_bytes(seed).expect("nan_nat seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_int_vs_float_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/semantic_eq/int_vs_float.bin"
    );
    fuzz_semantic_eq_bytes(seed).expect("int_vs_float seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_int_vs_bool_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/semantic_eq/int_vs_bool.bin"
    );
    fuzz_semantic_eq_bytes(seed).expect("int_vs_bool seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_bool_same_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/bool_same.bin");
    fuzz_semantic_eq_bytes(seed).expect("bool_same seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_replays_committed_corpus_seeds() {
    let seeds: &[(&str, &[u8])] = &[
        (
            "int_same.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/int_same.bin"),
        ),
        (
            "float_same.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/float_same.bin"),
        ),
        (
            "bool_same.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/bool_same.bin"),
        ),
        (
            "utf8_same.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/utf8_same.bin"),
        ),
        (
            "int_diff.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/int_diff.bin"),
        ),
        (
            "float_diff.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/float_diff.bin"),
        ),
        (
            "bool_diff.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/bool_diff.bin"),
        ),
        (
            "null_null.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/null_null.bin"),
        ),
        (
            "null_nan.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/null_nan.bin"),
        ),
        (
            "null_nat.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/null_nat.bin"),
        ),
        (
            "nan_nat.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/nan_nat.bin"),
        ),
        (
            "float_nan.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/float_nan.bin"),
        ),
        (
            "float_nan_vs_null.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/float_nan_vs_null.bin"),
        ),
        (
            "int_vs_float.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/int_vs_float.bin"),
        ),
        (
            "int_vs_bool.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/int_vs_bool.bin"),
        ),
        (
            "float_vs_bool.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/float_vs_bool.bin"),
        ),
        (
            "int_zero.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/int_zero.bin"),
        ),
        (
            "float_zero.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/float_zero.bin"),
        ),
        (
            "seed-empty.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_semantic_eq/seed-empty.bin"),
        ),
    ];

    for (name, seed) in seeds {
        let result = fuzz_semantic_eq_bytes(seed);
        assert!(
            result.is_ok(),
            "semantic_eq corpus seed {name} should pass: {result:?}"
        );
    }
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_empty_input() {
    fuzz_dataframe_eval_bytes(&[]).expect("empty input should be a no-op");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_simple_numeric_expression() {
    let mut seed = vec![0, b'a', b'+', b'b'];
    seed.extend([
        4, 2, 0, 1, 10, 20, 11, 21, 12, 22, 13, 23, 14, 24, 15, 25, 16, 26, 17, 27, 18, 28, 19, 29,
        20, 30, 21, 31,
    ]);
    fuzz_dataframe_eval_bytes(&seed).expect("simple eval seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_arith_add_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/dataframe_eval/arith_add.bin");
    fuzz_dataframe_eval_bytes(seed).expect("arith_add eval seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_arith_chain_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/dataframe_eval/arith_chain.bin");
    fuzz_dataframe_eval_bytes(seed).expect("arith_chain eval seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_arith_div_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/dataframe_eval/arith_div.bin");
    fuzz_dataframe_eval_bytes(seed).expect("arith_div eval seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_arith_mul_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/dataframe_eval/arith_mul.bin");
    fuzz_dataframe_eval_bytes(seed).expect("arith_mul eval seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_arith_paren_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/dataframe_eval/arith_paren.bin");
    fuzz_dataframe_eval_bytes(seed).expect("arith_paren eval seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_arith_sub_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/dataframe_eval/arith_sub.bin");
    fuzz_dataframe_eval_bytes(seed).expect("arith_sub eval seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_cmp_eq_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/dataframe_eval/cmp_eq.bin");
    fuzz_dataframe_eval_bytes(seed).expect("cmp_eq eval seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_cmp_gt_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/dataframe_eval/cmp_gt.bin");
    fuzz_dataframe_eval_bytes(seed).expect("cmp_gt eval seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_eval_bytes_replays_committed_corpus_seeds() {
    let seeds: &[(&str, &[u8])] = &[
        (
            "arith_add.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/arith_add.bin"),
        ),
        (
            "arith_sub.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/arith_sub.bin"),
        ),
        (
            "arith_mul.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/arith_mul.bin"),
        ),
        (
            "arith_div.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/arith_div.bin"),
        ),
        (
            "arith_chain.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/arith_chain.bin"),
        ),
        (
            "arith_paren.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/arith_paren.bin"),
        ),
        (
            "cmp_gt.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/cmp_gt.bin"),
        ),
        (
            "cmp_lt.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/cmp_lt.bin"),
        ),
        (
            "cmp_gte.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/cmp_gte.bin"),
        ),
        (
            "cmp_lte.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/cmp_lte.bin"),
        ),
        (
            "cmp_eq.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/cmp_eq.bin"),
        ),
        (
            "cmp_neq.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/cmp_neq.bin"),
        ),
        (
            "unary_neg.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/unary_neg.bin"),
        ),
        (
            "unary_pos.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/unary_pos.bin"),
        ),
        (
            "hardened_mul_add.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/hardened_mul_add.bin"),
        ),
        (
            "hardened_add_mul.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/hardened_add_mul.bin"),
        ),
        (
            "four_col_chain.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/four_col_chain.bin"),
        ),
        (
            "four_col_paren.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/four_col_paren.bin"),
        ),
        (
            "float_add.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/float_add.bin"),
        ),
        (
            "float_mul.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/float_mul.bin"),
        ),
        (
            "col_plus_literal.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/col_plus_literal.bin"),
        ),
        (
            "col_times_literal.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/col_times_literal.bin"),
        ),
        (
            "self_subtract.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/self_subtract.bin"),
        ),
        (
            "self_divide.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/self_divide.bin"),
        ),
        (
            "pow_square.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/pow_square.bin"),
        ),
        (
            "floor_div.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/floor_div.bin"),
        ),
        (
            "modulo.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/modulo.bin"),
        ),
        (
            "nested_paren.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/nested_paren.bin"),
        ),
        (
            "null_values.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/null_values.bin"),
        ),
        (
            "eight_rows.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/eight_rows.bin"),
        ),
        (
            "seed-empty.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_eval/seed-empty.bin"),
        ),
    ];

    for (name, seed) in seeds {
        let result = fuzz_dataframe_eval_bytes(seed);
        assert!(
            result.is_ok(),
            "dataframe_eval corpus seed {name} should pass: {result:?}"
        );
    }
}

#[test]
fn fuzz_query_str_bytes_accepts_arith_cmp_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/arith_cmp.bin");
    fuzz_query_str_bytes(seed).expect("arith_cmp query seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_arith_mul_cmp_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/query_str/arith_mul_cmp.bin"
    );
    fuzz_query_str_bytes(seed).expect("arith_mul_cmp query seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_arith_sub_cmp_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/query_str/arith_sub_cmp.bin"
    );
    fuzz_query_str_bytes(seed).expect("arith_sub_cmp query seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_bool_and_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/bool_and.bin");
    fuzz_query_str_bytes(seed).expect("bool_and query seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_bool_not_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/bool_not.bin");
    fuzz_query_str_bytes(seed).expect("bool_not query seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_bool_or_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/bool_or.bin");
    fuzz_query_str_bytes(seed).expect("bool_or query seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_bool_paren_and_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/query_str/bool_paren_and.bin"
    );
    fuzz_query_str_bytes(seed).expect("bool_paren_and query seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_bool_paren_or_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/query_str/bool_paren_or.bin"
    );
    fuzz_query_str_bytes(seed).expect("bool_paren_or query seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_replays_committed_corpus_seeds() {
    let seeds: &[(&str, &[u8])] = &[
        (
            "cmp_gt_zero.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_gt_zero.bin"),
        ),
        (
            "cmp_lt_zero.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_lt_zero.bin"),
        ),
        (
            "cmp_gte_zero.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_gte_zero.bin"),
        ),
        (
            "cmp_lte_zero.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_lte_zero.bin"),
        ),
        (
            "cmp_eq_zero.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_eq_zero.bin"),
        ),
        (
            "cmp_neq_zero.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_neq_zero.bin"),
        ),
        (
            "cmp_col_gt.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_col_gt.bin"),
        ),
        (
            "cmp_col_lt.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_col_lt.bin"),
        ),
        (
            "cmp_col_eq.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_col_eq.bin"),
        ),
        (
            "cmp_col_neq.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/cmp_col_neq.bin"),
        ),
        (
            "bool_and.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/bool_and.bin"),
        ),
        (
            "bool_or.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/bool_or.bin"),
        ),
        (
            "bool_not.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/bool_not.bin"),
        ),
        (
            "bool_paren_and.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/bool_paren_and.bin"),
        ),
        (
            "bool_paren_or.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/bool_paren_or.bin"),
        ),
        (
            "arith_cmp.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/arith_cmp.bin"),
        ),
        (
            "arith_sub_cmp.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/arith_sub_cmp.bin"),
        ),
        (
            "arith_mul_cmp.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/arith_mul_cmp.bin"),
        ),
        (
            "hardened_cmp.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/hardened_cmp.bin"),
        ),
        (
            "hardened_chain.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/hardened_chain.bin"),
        ),
        (
            "four_col_and.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/four_col_and.bin"),
        ),
        (
            "four_col_arith.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/four_col_arith.bin"),
        ),
        (
            "float_cmp.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/float_cmp.bin"),
        ),
        (
            "null_frame.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/null_frame.bin"),
        ),
        (
            "eight_rows.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/eight_rows.bin"),
        ),
        (
            "filter_some.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/filter_some.bin"),
        ),
        (
            "filter_none.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/filter_none.bin"),
        ),
        (
            "filter_all.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str/filter_all.bin"),
        ),
    ];

    for (name, seed) in seeds {
        let result = fuzz_query_str_bytes(seed);
        assert!(
            result.is_ok(),
            "query_str corpus seed {name} should pass: {result:?}"
        );
    }
}

#[test]
fn fuzz_query_str_with_locals_bytes_accepts_structured_corpus_seeds() {
    let seeds: &[(&str, &[u8])] = &[
        (
            "arithmetic_local_threshold",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/arithmetic_local_threshold"
            ),
        ),
        (
            "column_collision_and_limit",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/column_collision_and_limit"
            ),
        ),
        (
            "column_collision_eq",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/column_collision_eq"
            ),
        ),
        (
            "dunder_arithmetic",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str_with_locals/dunder_arithmetic"),
        ),
        (
            "empty",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str_with_locals/empty"),
        ),
        (
            "empty_bytes",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str_with_locals/empty_bytes"),
        ),
        (
            "hardened_or_threshold",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/hardened_or_threshold"
            ),
        ),
        (
            "local_secret_int_cmp",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/local_secret_int_cmp"
            ),
        ),
        (
            "missing_local_error_path",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/missing_local_error_path"
            ),
        ),
        (
            "nested_attr",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str_with_locals/nested_attr"),
        ),
        (
            "null_local_eq",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str_with_locals/null_local_eq"),
        ),
        (
            "parenthesized_mixed_arithmetic",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/parenthesized_mixed_arithmetic"
            ),
        ),
        (
            "policy_cmp",
            include_bytes!("../../../../fuzz/corpus/fuzz_query_str_with_locals/policy_cmp"),
        ),
        (
            "scalar_threshold_strict",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/scalar_threshold_strict"
            ),
        ),
        (
            "utf8_local_type_mismatch",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/utf8_local_type_mismatch"
            ),
        ),
        (
            "whole_expr_starts_with_local",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_query_str_with_locals/whole_expr_starts_with_local"
            ),
        ),
    ];

    for &(name, seed) in seeds {
        let result = fuzz_query_str_with_locals_bytes(seed);
        assert!(
            result.is_ok(),
            "query-with-locals seed {name} should satisfy invariants: {result:?}"
        );
    }
}

#[test]
fn fuzz_dataframe_op_chain_bytes_accepts_structured_corpus_seeds() {
    let seeds: &[(&str, &[u8])] = &[
        (
            "drop_then_select",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/drop_then_select"),
        ),
        (
            "drop_to_empty_after_reset",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_dataframe_op_chain/drop_to_empty_after_reset"
            ),
        ),
        (
            "eight_step_all_ops_mixed",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_dataframe_op_chain/eight_step_all_ops_mixed"
            ),
        ),
        (
            "interleaved_select_reset",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_dataframe_op_chain/interleaved_select_reset"
            ),
        ),
        (
            "max_rows_four_col_desc",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_dataframe_op_chain/max_rows_four_col_desc"
            ),
        ),
        (
            "empty",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/empty"),
        ),
        (
            "missing_heavy_values",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/missing_heavy_values"),
        ),
        (
            "mixed_chain",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/mixed_chain"),
        ),
        (
            "nan_heavy_float",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/nan_heavy_float"),
        ),
        (
            "select_drop_boundary",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/select_drop_boundary"),
        ),
        (
            "single_row_reset_sort",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/single_row_reset_sort"),
        ),
        (
            "sort_desc_float",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/sort_desc_float"),
        ),
        (
            "short_min_valid",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/short_min_valid"),
        ),
        (
            "select_only",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/select_only"),
        ),
        (
            "wide_desc_sort_churn",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/wide_desc_sort_churn"),
        ),
        (
            "wide_int_sort_churn",
            include_bytes!("../../../../fuzz/corpus/fuzz_dataframe_op_chain/wide_int_sort_churn"),
        ),
    ];

    for &(name, seed) in seeds {
        let result = fuzz_dataframe_op_chain_bytes(seed);
        assert!(
            result.is_ok(),
            "op-chain seed {name} should satisfy invariants: {result:?}"
        );
    }
}

#[test]
fn fuzz_common_dtype_bytes_accepts_identical_dtype_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/common_dtype/identical_int64_seed.bin"
    );
    fuzz_common_dtype_bytes(seed).expect("identical dtype seed should satisfy invariants");
}

#[test]
fn fuzz_common_dtype_bytes_accepts_numeric_promotion_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/common_dtype/numeric_promotion_seed.bin"
    );
    fuzz_common_dtype_bytes(seed).expect("numeric promotion seed should satisfy invariants");
}

#[test]
fn fuzz_common_dtype_bytes_accepts_incompatible_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/common_dtype/incompatible_utf8_bool_seed.bin"
    );
    fuzz_common_dtype_bytes(seed).expect("incompatible seed should still preserve symmetry");
}

#[test]
fn fuzz_common_dtype_bytes_accepts_null_null_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/common_dtype/null_null_seed.bin");
    fuzz_common_dtype_bytes(seed).expect("null+null seed should preserve symmetry");
}

#[test]
fn fuzz_common_dtype_bytes_accepts_bool_bool_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/common_dtype/bool_bool_seed.bin");
    fuzz_common_dtype_bytes(seed).expect("bool+bool seed should preserve symmetry");
}

#[test]
fn fuzz_common_dtype_bytes_accepts_bool_float_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/common_dtype/bool_float_seed.bin");
    fuzz_common_dtype_bytes(seed).expect("bool+float seed should preserve symmetry");
}

#[test]
fn fuzz_common_dtype_bytes_accepts_float_utf8_incompatible_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/common_dtype/float_utf8_incompatible_seed.bin"
    );
    fuzz_common_dtype_bytes(seed).expect("float+utf8 incompat seed should preserve symmetry");
}

#[test]
fn fuzz_common_dtype_bytes_accepts_null_int_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/common_dtype/null_int_seed.bin");
    fuzz_common_dtype_bytes(seed).expect("null+int seed should preserve symmetry");
}

#[test]
fn fuzz_common_dtype_bytes_replays_committed_corpus_seeds() {
    let corpus: &[(&str, &[u8])] = &[
        (
            "null_null.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/null_null.bin"),
        ),
        (
            "bool_bool.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/bool_bool.bin"),
        ),
        (
            "int64_int64.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/int64_int64.bin"),
        ),
        (
            "float64_float64.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/float64_float64.bin"),
        ),
        (
            "utf8_utf8.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/utf8_utf8.bin"),
        ),
        (
            "null_bool.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/null_bool.bin"),
        ),
        (
            "null_int64.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/null_int64.bin"),
        ),
        (
            "null_float64.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/null_float64.bin"),
        ),
        (
            "null_utf8.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/null_utf8.bin"),
        ),
        (
            "bool_int64.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/bool_int64.bin"),
        ),
        (
            "bool_float64.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/bool_float64.bin"),
        ),
        (
            "int64_float64.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/int64_float64.bin"),
        ),
        (
            "bool_utf8.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/bool_utf8.bin"),
        ),
        (
            "int64_utf8.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/int64_utf8.bin"),
        ),
        (
            "float64_utf8.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/float64_utf8.bin"),
        ),
        (
            "float64_bool.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/float64_bool.bin"),
        ),
        (
            "float64_int64.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/float64_int64.bin"),
        ),
        (
            "utf8_null.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_common_dtype/utf8_null.bin"),
        ),
    ];

    for (name, seed) in corpus {
        let result = fuzz_common_dtype_bytes(seed);
        assert!(
            result.is_ok(),
            "common_dtype corpus seed {name} should pass: {result:?}"
        );
    }
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_identity_int64_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/scalar_cast/identity_int64_seed.bin"
    );
    fuzz_scalar_cast_bytes(seed).expect("identity cast seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_missing_float_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/scalar_cast/missing_float_seed.bin");
    fuzz_scalar_cast_bytes(seed).expect("missing float seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_lossy_float_error_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/scalar_cast/lossy_float_to_int_seed.bin"
    );
    fuzz_scalar_cast_bytes(seed).expect("lossy float cast seed should still satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_bool_true_to_int_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/scalar_cast/bool_true_to_int.bin"
    );
    fuzz_scalar_cast_bytes(seed).expect("bool->int seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_int_to_utf8_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/scalar_cast/int_to_utf8.bin");
    fuzz_scalar_cast_bytes(seed).expect("int->utf8 seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_nan_to_int_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/scalar_cast/nan_to_int.bin");
    fuzz_scalar_cast_bytes(seed).expect("nan->int seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_null_to_bool_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/scalar_cast/null_to_bool.bin");
    fuzz_scalar_cast_bytes(seed).expect("null->bool seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_float_inf_to_int_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/scalar_cast/float_inf_to_int.bin"
    );
    fuzz_scalar_cast_bytes(seed).expect("inf->int seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_bool_to_str_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/scalar_cast/bool_to_str.bin");
    fuzz_scalar_cast_bytes(seed).expect("bool->str seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_utf8_to_int_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/scalar_cast/utf8_to_int.bin");
    fuzz_scalar_cast_bytes(seed).expect("utf8->int seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_nat_to_int_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/scalar_cast/nat_to_int.bin");
    fuzz_scalar_cast_bytes(seed).expect("nat->int seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_bool_false_to_int_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/scalar_cast/bool_false_to_int.bin"
    );
    fuzz_scalar_cast_bytes(seed).expect("bool false->int seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_neg_inf_to_str_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/scalar_cast/neg_inf_to_str.bin");
    fuzz_scalar_cast_bytes(seed).expect("neg_inf->str seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_int_max_to_str_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/scalar_cast/int_max_to_str.bin");
    fuzz_scalar_cast_bytes(seed).expect("int_max->str seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_empty_utf8_to_int_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/scalar_cast/empty_utf8_to_int.bin"
    );
    fuzz_scalar_cast_bytes(seed).expect("empty utf8->int seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_replays_committed_corpus_seeds() {
    let corpus: &[(&str, &[u8])] = &[
        (
            "null_to_null.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/null_to_null.bin"),
        ),
        (
            "null_to_bool.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/null_to_bool.bin"),
        ),
        (
            "null_to_int.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/null_to_int.bin"),
        ),
        (
            "null_to_float.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/null_to_float.bin"),
        ),
        (
            "null_to_utf8.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/null_to_utf8.bin"),
        ),
        (
            "nan_to_float.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/nan_to_float.bin"),
        ),
        (
            "bool_true_to_int.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/bool_true_to_int.bin"),
        ),
        (
            "bool_false_to_float.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/bool_false_to_float.bin"),
        ),
        (
            "int_max_to_float.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/int_max_to_float.bin"),
        ),
        (
            "int_min_to_float.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/int_min_to_float.bin"),
        ),
        (
            "int_to_utf8.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/int_to_utf8.bin"),
        ),
        (
            "float_inf_to_int.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/float_inf_to_int.bin"),
        ),
        (
            "float_nan_to_int.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/float_nan_to_int.bin"),
        ),
        (
            "float_zero_to_bool.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/float_zero_to_bool.bin"),
        ),
        (
            "float_one_to_bool.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/float_one_to_bool.bin"),
        ),
        (
            "float_neg_to_utf8.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/float_neg_to_utf8.bin"),
        ),
        (
            "utf8_42_to_int.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/utf8_42_to_int.bin"),
        ),
        (
            "utf8_empty_to_bool.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/utf8_empty_to_bool.bin"),
        ),
        (
            "bool_to_null.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/bool_to_null.bin"),
        ),
        (
            "bool_false_to_utf8.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/bool_false_to_utf8.bin"),
        ),
        (
            "float_neginf_to_float.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/float_neginf_to_float.bin"),
        ),
        (
            "utf8_neg1_to_int.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/utf8_neg1_to_int.bin"),
        ),
        (
            "int_zero_to_float.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_scalar_cast/int_zero_to_float.bin"),
        ),
    ];

    for (name, seed) in corpus {
        let result = fuzz_scalar_cast_bytes(seed);
        assert!(
            result.is_ok(),
            "scalar_cast corpus seed {name} should pass: {result:?}"
        );
    }
}

#[test]
fn fuzz_series_add_bytes_accepts_unique_overlap_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/unique_overlap_seed.bin");
    fuzz_series_add_bytes(seed).expect("unique-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_duplicate_cross_product_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/series_add/duplicate_cross_product_seed.bin"
    );
    fuzz_series_add_bytes(seed).expect("duplicate cross-product seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_float_plus_inf_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/float_plus_inf.bin");
    fuzz_series_add_bytes(seed).expect("float+inf seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_float_plus_neginf_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/float_plus_neginf.bin");
    fuzz_series_add_bytes(seed).expect("float+-inf seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_float_plus_nanfloat_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/float_plus_nanfloat.bin");
    fuzz_series_add_bytes(seed).expect("float+NaN seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_zero_plus_negzero_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/zero_plus_negzero.bin");
    fuzz_series_add_bytes(seed).expect("zero+negzero seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_null_plus_int_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/null_plus_int.bin");
    fuzz_series_add_bytes(seed).expect("null+int seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_nan_plus_int_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/nan_plus_int.bin");
    fuzz_series_add_bytes(seed).expect("nan+int seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_int_plus_float_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/int_plus_float.bin");
    fuzz_series_add_bytes(seed).expect("int+float seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_neg_int_values_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/neg_int_values.bin");
    fuzz_series_add_bytes(seed).expect("neg_int seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_seed_empty_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/seed-empty.bin");
    fuzz_series_add_bytes(seed).expect("empty seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_missing_alignment_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/series_add/missing_alignment_seed.bin"
    );
    fuzz_series_add_bytes(seed).expect("missing alignment seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_replays_committed_corpus_seeds() {
    let seeds: &[(&str, &[u8])] = &[
        (
            "single_same_idx_int",
            include_bytes!(
                "../../fixtures/adversarial/fuzz_corpus/series_add/single_same_idx_int.bin"
            ),
        ),
        (
            "single_diff_idx_int",
            include_bytes!(
                "../../fixtures/adversarial/fuzz_corpus/series_add/single_diff_idx_int.bin"
            ),
        ),
        (
            "null_plus_int",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/null_plus_int.bin"),
        ),
        (
            "nan_plus_int",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/nan_plus_int.bin"),
        ),
        (
            "int_plus_float",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/int_plus_float.bin"),
        ),
        (
            "float_plus_inf",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/float_plus_inf.bin"),
        ),
        (
            "float_plus_neginf",
            include_bytes!(
                "../../fixtures/adversarial/fuzz_corpus/series_add/float_plus_neginf.bin"
            ),
        ),
        (
            "float_plus_nanfloat",
            include_bytes!(
                "../../fixtures/adversarial/fuzz_corpus/series_add/float_plus_nanfloat.bin"
            ),
        ),
        (
            "multi_overlap_nulls",
            include_bytes!(
                "../../fixtures/adversarial/fuzz_corpus/series_add/multi_overlap_nulls.bin"
            ),
        ),
        (
            "all_nulls_left",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/all_nulls_left.bin"),
        ),
        (
            "all_nans_left",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/all_nans_left.bin"),
        ),
        (
            "mixed_types_both",
            include_bytes!(
                "../../fixtures/adversarial/fuzz_corpus/series_add/mixed_types_both.bin"
            ),
        ),
        (
            "neg_int_values",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/neg_int_values.bin"),
        ),
        (
            "zero_plus_negzero",
            include_bytes!(
                "../../fixtures/adversarial/fuzz_corpus/series_add/zero_plus_negzero.bin"
            ),
        ),
        (
            "large_plus_small",
            include_bytes!(
                "../../fixtures/adversarial/fuzz_corpus/series_add/large_plus_small.bin"
            ),
        ),
        (
            "string_idx_labels",
            include_bytes!(
                "../../fixtures/adversarial/fuzz_corpus/series_add/string_idx_labels.bin"
            ),
        ),
        (
            "dup_labels_same",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/dup_labels_same.bin"),
        ),
        (
            "seed-empty",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/series_add/seed-empty.bin"),
        ),
    ];

    for (name, seed) in seeds {
        let result = fuzz_series_add_bytes(seed);
        assert!(
            result.is_ok(),
            "series_add corpus seed {name} should pass: {result:?}"
        );
    }
}

#[test]
fn fuzz_column_arith_bytes_accepts_add_missing_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/add_missing_seed.bin");
    fuzz_column_arith_bytes(seed).expect("add-missing seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_sub_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/sub_int_seed.bin");
    fuzz_column_arith_bytes(seed).expect("sub seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_mixed_mul_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/mixed_mul_seed.bin");
    fuzz_column_arith_bytes(seed).expect("mixed-mul seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_div_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/div_identity_seed.bin");
    fuzz_column_arith_bytes(seed).expect("div seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_mod_zero_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/mod_zero_divisor_seed.bin"
    );
    fuzz_column_arith_bytes(seed).expect("mod-zero seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_pow_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/pow_seed.bin");
    fuzz_column_arith_bytes(seed).expect("pow seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_floor_div_zero_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/floor_div_zero_divisor_seed.bin"
    );
    fuzz_column_arith_bytes(seed).expect("floor-div-zero seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_add_zero_to_zero_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/add_zero_to_zero.bin"
    );
    fuzz_column_arith_bytes(seed).expect("add-zero-to-zero seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_div_pos_inf_by_neg_inf_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/div_pos_inf_by_neg_inf.bin"
    );
    fuzz_column_arith_bytes(seed).expect("div +inf/-inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_pow_zero_to_neg_two_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/pow_zero_to_neg_two.bin"
    );
    fuzz_column_arith_bytes(seed).expect("pow zero^-2 seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_accepts_mul_int_4_by_float_25_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/mul_int_4_by_float_25.bin"
    );
    fuzz_column_arith_bytes(seed).expect("mul int*float seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_mod_pos_div_pos_inf() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/mod_pos_div_pos_inf.bin"
    );
    fuzz_column_arith_bytes(seed).expect("mod pos/+inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_mod_neg_div_pos_inf() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/mod_neg_div_pos_inf.bin"
    );
    fuzz_column_arith_bytes(seed).expect("mod neg/+inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_mod_pos_div_neg_inf() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/mod_pos_div_neg_inf.bin"
    );
    fuzz_column_arith_bytes(seed).expect("mod pos/-inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_mod_neg_div_neg_inf() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/mod_neg_div_neg_inf.bin"
    );
    fuzz_column_arith_bytes(seed).expect("mod neg/-inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_mod_zero_div_inf() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/mod_zero_div_inf.bin");
    fuzz_column_arith_bytes(seed).expect("mod 0/inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_mod_nan_div_inf() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/mod_nan_div_inf.bin");
    fuzz_column_arith_bytes(seed).expect("mod nan/inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_floordiv_pos_div_inf() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/floordiv_pos_div_inf.bin"
    );
    fuzz_column_arith_bytes(seed).expect("floordiv pos/inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_floordiv_neg_div_inf() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/floordiv_neg_div_inf.bin"
    );
    fuzz_column_arith_bytes(seed).expect("floordiv neg/inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_mul_inf_by_zero() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/mul_inf_by_zero.bin");
    fuzz_column_arith_bytes(seed).expect("mul inf*0 seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_div_by_neg_zero() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/div_by_neg_zero.bin");
    fuzz_column_arith_bytes(seed).expect("div by -0 seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_add_inf_to_neg_inf() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/add_inf_to_neg_inf.bin"
    );
    fuzz_column_arith_bytes(seed).expect("add inf+(-inf) seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_mul_inf_zero() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/mul_inf_zero.bin");
    fuzz_column_arith_bytes(seed).expect("mul inf*0 seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_pow_neg_zero_zero() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/pow_neg_zero_zero.bin");
    fuzz_column_arith_bytes(seed).expect("pow (-0.0)**0.0 seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_mod_negative_negative() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/mod_negative_negative.bin"
    );
    fuzz_column_arith_bytes(seed).expect("mod neg%neg seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_sub_nan_int() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/sub_nan_int.bin");
    fuzz_column_arith_bytes(seed).expect("sub NaN-int seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_floordiv_neg_inf_pos_inf() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/floordiv_neg_inf_pos_inf.bin"
    );
    fuzz_column_arith_bytes(seed).expect("floordiv -inf//+inf seed should satisfy invariants");
}

#[test]
fn fuzz_column_arith_bytes_add_null_finite() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/column_arith/add_null_finite.bin");
    fuzz_column_arith_bytes(seed).expect("add Null+finite seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_inner_overlap_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/inner_overlap_seed.bin");
    fuzz_join_series_bytes(seed).expect("inner-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_left_unmatched_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/join_series/left_unmatched_seed.bin"
    );
    fuzz_join_series_bytes(seed).expect("left-unmatched seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_right_unmatched_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/join_series/right_unmatched_seed.bin"
    );
    fuzz_join_series_bytes(seed).expect("right-unmatched seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_outer_union_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/outer_union_seed.bin");
    fuzz_join_series_bytes(seed).expect("outer-union seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_cross_product_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/cross_product_seed.bin");
    fuzz_join_series_bytes(seed).expect("cross-product seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_inner_empty_left() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/inner_empty_left.bin");
    fuzz_join_series_bytes(seed).expect("inner-empty-left seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_inner_empty_right() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/inner_empty_right.bin");
    fuzz_join_series_bytes(seed).expect("inner-empty-right seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_inner_dup_labels() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/inner_dup_labels.bin");
    fuzz_join_series_bytes(seed).expect("inner-dup-labels seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_outer_all_nulls() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/outer_all_nulls.bin");
    fuzz_join_series_bytes(seed).expect("outer-all-nulls seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_left_null_values() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/left_null_values.bin");
    fuzz_join_series_bytes(seed).expect("left-null-values seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_outer_large_overlap() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/join_series/outer_large_overlap.bin"
    );
    fuzz_join_series_bytes(seed).expect("outer-large-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_inner_no_overlap() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/inner_no_overlap.bin");
    fuzz_join_series_bytes(seed).expect("inner-no-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_left_asymmetric() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/left_asymmetric.bin");
    fuzz_join_series_bytes(seed).expect("left-asymmetric seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_cross_small() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/cross_small.bin");
    fuzz_join_series_bytes(seed).expect("cross-small seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_right_asymmetric() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/right_asymmetric.bin");
    fuzz_join_series_bytes(seed).expect("right-asymmetric seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_outer_mixed_dtypes() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/outer_mixed_dtypes.bin");
    fuzz_join_series_bytes(seed).expect("outer-mixed-dtypes seed should satisfy invariants");
}

#[test]
fn fuzz_join_series_bytes_accepts_inner_single_each() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/join_series/inner_single_each.bin");
    fuzz_join_series_bytes(seed).expect("inner-single-each seed should satisfy invariants");
}

#[test]
fn fuzz_groupby_sum_bytes_accepts_dropna_true_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/groupby_sum/dropna_true_seed.bin");
    fuzz_groupby_sum_bytes(seed).expect("dropna=true seed should satisfy invariants");
}

#[test]
fn fuzz_groupby_sum_bytes_accepts_dropna_false_null_group_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/groupby_sum/dropna_false_null_group_seed.bin"
    );
    fuzz_groupby_sum_bytes(seed).expect("dropna=false null-group seed should satisfy invariants");
}

#[test]
fn fuzz_groupby_sum_bytes_accepts_alignment_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/groupby_sum/alignment_seed.bin");
    fuzz_groupby_sum_bytes(seed).expect("alignment seed should satisfy invariants");
}

#[test]
fn fuzz_groupby_sum_bytes_accepts_structured_corpus_seeds() {
    let seeds: &[(&str, &[u8])] = &[
        (
            "alignment_seed.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/alignment_seed.bin"),
        ),
        (
            "all_null_keys.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/all_null_keys.bin"),
        ),
        (
            "all_null_values.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/all_null_values.bin"),
        ),
        (
            "alternating_keys.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/alternating_keys.bin"),
        ),
        (
            "boundary_key_modulo.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/boundary_key_modulo.bin"),
        ),
        (
            "dropna_false_null_group_seed.bin",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_sum/dropna_false_null_group_seed.bin"
            ),
        ),
        (
            "dropna_true_seed.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/dropna_true_seed.bin"),
        ),
        (
            "dropna_true_with_nulls.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/dropna_true_with_nulls.bin"),
        ),
        (
            "empty_keys.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/empty_keys.bin"),
        ),
        (
            "empty_values.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/empty_values.bin"),
        ),
        (
            "float_edge_values.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/float_edge_values.bin"),
        ),
        (
            "many_unique_keys.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/many_unique_keys.bin"),
        ),
        (
            "mixed_int_float.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/mixed_int_float.bin"),
        ),
        (
            "multi_key_single_value.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/multi_key_single_value.bin"),
        ),
        (
            "negative_values.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/negative_values.bin"),
        ),
        (
            "same_key_repeated.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/same_key_repeated.bin"),
        ),
        (
            "seed-empty.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/seed-empty.bin"),
        ),
        (
            "single_key_multi_value.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/single_key_multi_value.bin"),
        ),
        (
            "zero_sum_group.bin",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_sum/zero_sum_group.bin"),
        ),
    ];

    for &(name, seed) in seeds {
        let result = fuzz_groupby_sum_bytes(seed);
        assert!(
            result.is_ok(),
            "{name} should satisfy groupby_sum invariants: {result:?}"
        );
    }
}

#[test]
fn fuzz_groupby_agg_bytes_accepts_empty_input() {
    fuzz_groupby_agg_bytes(&[]).expect("empty input should be a no-op");
}

#[test]
fn fuzz_groupby_agg_bytes_accepts_supported_agg_list_seed() {
    let seed = [0, 0, 1, 2, b'|', 1, 0, 1, 1, 1, 4, 1, 2, 7, 1, 3, 9];
    fuzz_groupby_agg_bytes(&seed).expect("agg_list seed should satisfy invariants");
}

#[test]
fn fuzz_groupby_agg_bytes_accepts_compat_rejected_seed() {
    let seed = [2, 13, b'|', 1, 0, 1, 1, 1, 4, 1, 2, 7];
    fuzz_groupby_agg_bytes(&seed)
        .expect("unsupported agg seed should stay in CompatibilityRejected");
}

#[test]
fn fuzz_groupby_agg_bytes_accepts_named_aggregation_seed() {
    let seed = [3, 0, 1, b'|', 1, 0, 1, 1, 1, 3, 1, 2, 5, 1, 3, 7];
    fuzz_groupby_agg_bytes(&seed).expect("agg_named seed should satisfy invariants");
}

#[test]
fn fuzz_groupby_agg_bytes_accepts_structured_corpus_seeds() {
    let seeds: &[(&str, &[u8])] = &[
        (
            "agg_dict_list_median_prod",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_dict_list_median_prod"
            ),
        ),
        (
            "agg_dict_list_midpoint_split",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_dict_list_midpoint_split"
            ),
        ),
        (
            "agg_dict_list_sum_count_max",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_dict_list_sum_count_max"
            ),
        ),
        (
            "agg_list_bogus_rejected",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_list_bogus_rejected"
            ),
        ),
        (
            "agg_list_sum_count_max",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_list_sum_count_max"
            ),
        ),
        (
            "agg_named_duplicate_func_dedup",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_named_duplicate_func_dedup"
            ),
        ),
        (
            "agg_named_last_nunique",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_named_last_nunique"
            ),
        ),
        (
            "agg_named_sum_count_max",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_named_sum_count_max"
            ),
        ),
        (
            "agg_single_first_null_keys",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_single_first_null_keys"
            ),
        ),
        (
            "agg_single_mean_duplicates",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/agg_single_mean_duplicates"
            ),
        ),
        (
            "default_sum_empty_func",
            include_bytes!(
                "../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/default_sum_empty_func"
            ),
        ),
        (
            "short_row_boundary",
            include_bytes!("../../../../fuzz/corpus/fuzz_groupby_agg_dispatch/short_row_boundary"),
        ),
    ];

    for &(name, seed) in seeds {
        let result = fuzz_groupby_agg_bytes(seed);
        assert!(
            result.is_ok(),
            "groupby agg dispatch seed {name} should satisfy invariants: {result:?}"
        );
    }
}

#[test]
fn fuzz_index_align_bytes_accepts_unique_overlap_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/index_align/unique_overlap_seed.bin"
    );
    fuzz_index_align_bytes(seed).expect("unique-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_duplicate_cross_product_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/index_align/duplicate_cross_product_seed.bin"
    );
    fuzz_index_align_bytes(seed).expect("duplicate seed should satisfy multiplicity invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_utf8_right_only_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/index_align/utf8_right_only_seed.bin"
    );
    fuzz_index_align_bytes(seed).expect("utf8 right-only seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_empty_left() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/empty_left.bin");
    fuzz_index_align_bytes(seed).expect("empty-left seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_empty_right() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/empty_right.bin");
    fuzz_index_align_bytes(seed).expect("empty-right seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_both_empty() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/both_empty.bin");
    fuzz_index_align_bytes(seed).expect("both-empty seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_identical() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/identical.bin");
    fuzz_index_align_bytes(seed).expect("identical seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_no_overlap_int() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/no_overlap_int.bin");
    fuzz_index_align_bytes(seed).expect("no-overlap-int seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_dup_left() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/dup_left.bin");
    fuzz_index_align_bytes(seed).expect("dup-left seed should satisfy multiplicity invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_dup_right() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/dup_right.bin");
    fuzz_index_align_bytes(seed).expect("dup-right seed should satisfy multiplicity invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_mixed_overlap() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/mixed_overlap.bin");
    fuzz_index_align_bytes(seed).expect("mixed-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_max_length() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/max_length.bin");
    fuzz_index_align_bytes(seed).expect("max-length seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_single_same() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/single_same.bin");
    fuzz_index_align_bytes(seed).expect("single-same seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_single_diff() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/single_diff.bin");
    fuzz_index_align_bytes(seed).expect("single-diff seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_interleaved() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/interleaved.bin");
    fuzz_index_align_bytes(seed).expect("interleaved seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_utf8_overlap() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/utf8_overlap.bin");
    fuzz_index_align_bytes(seed).expect("utf8-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_utf8_no_overlap() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/utf8_no_overlap.bin");
    fuzz_index_align_bytes(seed).expect("utf8-no-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_seed_empty() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/index_align/seed-empty.bin");
    fuzz_index_align_bytes(seed).expect("seed-empty should satisfy invariants");
}

#[test]
fn fuzz_json_io_bytes_accepts_records_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/records_valid_seed.json"
    );
    fuzz_json_io_bytes(seed).expect("records fuzz seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_split_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/split_valid_seed.json");
    fuzz_json_io_bytes(seed).expect("split fuzz seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_jsonl_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/jsonl_valid_seed.jsonl"
    );
    fuzz_json_io_bytes(seed).expect("jsonl fuzz seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_columns_orient_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/columns_orient_seed.json"
    );
    fuzz_json_io_bytes(seed).expect("columns orient json seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_index_orient_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/index_orient_seed.json"
    );
    fuzz_json_io_bytes(seed).expect("index orient json seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_empty_records_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/empty_records_seed.json"
    );
    fuzz_json_io_bytes(seed).expect("empty records json seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_single_record_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/single_record_seed.json"
    );
    fuzz_json_io_bytes(seed).expect("single record json seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_jsonl_with_nulls_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/jsonl_with_nulls_seed.jsonl"
    );
    fuzz_json_io_bytes(seed).expect("jsonl with nulls seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_values_orient_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/values_orient_seed.json"
    );
    fuzz_json_io_bytes(seed).expect("values orient seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_jsonl_mixed_types_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/jsonl_mixed_types_seed.jsonl"
    );
    fuzz_json_io_bytes(seed).expect("jsonl mixed types seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_records_unicode_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/records_unicode_seed.json"
    );
    fuzz_json_io_bytes(seed).expect("records unicode seed should parse");
}

#[test]
fn fuzz_json_io_bytes_accepts_records_with_negatives_seed_fixture() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/records_with_negatives_seed.json"
    );
    fuzz_json_io_bytes(seed).expect("records with negatives seed should parse");
}

#[test]
fn fuzz_json_io_bytes_replays_committed_corpus_seeds() {
    let corpus: &[(&str, &[u8])] = &[
        (
            "columns_bool_float_seed.json",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/columns_bool_float_seed.json"),
        ),
        (
            "index_nested_labels_seed.json",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/index_nested_labels_seed.json"),
        ),
        (
            "jsonl_sparse_unicode_seed.jsonl",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/jsonl_sparse_unicode_seed.jsonl"),
        ),
        (
            "records_bool_null_seed.json",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/records_bool_null_seed.json"),
        ),
        (
            "records_sparse_keys_seed.json",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/records_sparse_keys_seed.json"),
        ),
        (
            "split_empty_columns_seed.json",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/split_empty_columns_seed.json"),
        ),
        (
            "split_mixed_scalar_seed.json",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/split_mixed_scalar_seed.json"),
        ),
        (
            "values_rectangular_seed.json",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/values_rectangular_seed.json"),
        ),
        (
            "values_single_column_seed.json",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/values_single_column_seed.json"),
        ),
        (
            "whitespace_records_seed.json",
            include_bytes!("../../../../fuzz/corpus/fuzz_json_io/whitespace_records_seed.json"),
        ),
    ];

    for (name, seed) in corpus {
        let result = fuzz_json_io_bytes(seed);
        assert!(
            result.is_ok(),
            "committed json io corpus seed {name} failed: {result:?}"
        );
    }
}

#[test]
fn fuzz_json_io_bytes_reports_invalid_json() {
    let err = fuzz_json_io_bytes(br#"{"records": ["unterminated""#)
        .expect_err("invalid json should error");
    assert!(
        matches!(err, fp_io::IoError::Json(_)),
        "expected JSON parse error, got {err:?}"
    );
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_inner_single_each() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/inner_single_each.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("inner-single-each seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_outer_no_overlap() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/outer_no_overlap.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("outer-no-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_inner_all_overlap() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/inner_all_overlap.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("inner-all-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_left_asymmetric() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/left_asymmetric.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("left-asymmetric seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_right_asymmetric() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/right_asymmetric.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("right-asymmetric seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_outer_max_rows() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/dataframe_merge/outer_max_rows.bin");
    fuzz_dataframe_merge_bytes(seed).expect("outer-max-rows seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_inner_empty_result() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/inner_empty_result.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("inner-empty-result seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_left_single_unmatched() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/left_single_unmatched.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("left-single-unmatched seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_right_single_unmatched() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/right_single_unmatched.bin"
    );
    fuzz_dataframe_merge_bytes(seed)
        .expect("right-single-unmatched seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_outer_partial_overlap() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/outer_partial_overlap.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("outer-partial-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_inner_duplicate_keys() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/inner_duplicate_keys.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("inner-duplicate-keys seed should satisfy invariants");
}

#[test]
fn fuzz_dataframe_merge_bytes_accepts_left_all_matched() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/dataframe_merge/left_all_matched.bin"
    );
    fuzz_dataframe_merge_bytes(seed).expect("left-all-matched seed should satisfy invariants");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_arith_add_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/arith_add.txt");
    fuzz_parse_expr_bytes(seed).expect("arith_add seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_arith_chain_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/arith_chain.txt");
    fuzz_parse_expr_bytes(seed).expect("arith_chain seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_arith_complex_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/arith_complex.txt");
    fuzz_parse_expr_bytes(seed).expect("arith_complex seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_bool_and_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/bool_and.txt");
    fuzz_parse_expr_bytes(seed).expect("bool_and seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_bool_combined_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/bool_combined.txt");
    fuzz_parse_expr_bytes(seed).expect("bool_combined seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_bool_not_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/bool_not.txt");
    fuzz_parse_expr_bytes(seed).expect("bool_not seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_cmp_eq_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/cmp_eq.txt");
    fuzz_parse_expr_bytes(seed).expect("cmp_eq seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_cmp_gt_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/cmp_gt.txt");
    fuzz_parse_expr_bytes(seed).expect("cmp_gt seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_deeply_nested_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/deeply_nested.txt");
    fuzz_parse_expr_bytes(seed).expect("deeply_nested seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_lit_float_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/lit_float.txt");
    fuzz_parse_expr_bytes(seed).expect("lit_float seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_replays_committed_corpus_seeds() {
    let seeds: &[(&str, &[u8])] = &[
        (
            "arith_add",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/arith_add.txt"),
        ),
        (
            "arith_multi",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/arith_multi.txt"),
        ),
        (
            "arith_complex",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/arith_complex.txt"),
        ),
        (
            "arith_chain",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/arith_chain.txt"),
        ),
        (
            "cmp_gt",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/cmp_gt.txt"),
        ),
        (
            "cmp_lte",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/cmp_lte.txt"),
        ),
        (
            "cmp_eq",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/cmp_eq.txt"),
        ),
        (
            "cmp_neq",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/cmp_neq.txt"),
        ),
        (
            "bool_and",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/bool_and.txt"),
        ),
        (
            "bool_or",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/bool_or.txt"),
        ),
        (
            "bool_not",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/bool_not.txt"),
        ),
        (
            "bool_combined",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/bool_combined.txt"),
        ),
        (
            "nested_paren",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/nested_paren.txt"),
        ),
        (
            "deeply_nested",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/deeply_nested.txt"),
        ),
        (
            "col_underscore",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/col_underscore.txt"),
        ),
        (
            "lit_int",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/lit_int.txt"),
        ),
        (
            "lit_float",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/lit_float.txt"),
        ),
        (
            "lit_neg",
            include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/lit_neg.txt"),
        ),
    ];

    for (name, seed) in seeds {
        fuzz_parse_expr_bytes(seed).unwrap_or_else(|err| panic!("seed {name} failed: {err}"));
    }
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_bool_diff_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/bool_diff.bin");
    fuzz_semantic_eq_bytes(seed).expect("bool_diff seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_float_vs_bool_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/semantic_eq/float_vs_bool.bin"
    );
    fuzz_semantic_eq_bytes(seed).expect("float_vs_bool seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_float_zero_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/float_zero.bin");
    fuzz_semantic_eq_bytes(seed).expect("float_zero seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_int_zero_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/int_zero.bin");
    fuzz_semantic_eq_bytes(seed).expect("int_zero seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_null_nat_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/null_nat.bin");
    fuzz_semantic_eq_bytes(seed).expect("null_nat seed should hold");
}

#[test]
fn fuzz_semantic_eq_bytes_accepts_utf8_same_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/semantic_eq/utf8_same.bin");
    fuzz_semantic_eq_bytes(seed).expect("utf8_same seed should hold");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_arith_multi_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/arith_multi.txt");
    fuzz_parse_expr_bytes(seed).expect("arith_multi seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_bool_or_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/bool_or.txt");
    fuzz_parse_expr_bytes(seed).expect("bool_or seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_cmp_lte_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/cmp_lte.txt");
    fuzz_parse_expr_bytes(seed).expect("cmp_lte seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_cmp_neq_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/cmp_neq.txt");
    fuzz_parse_expr_bytes(seed).expect("cmp_neq seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_col_underscore_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/col_underscore.txt");
    fuzz_parse_expr_bytes(seed).expect("col_underscore seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_lit_int_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/lit_int.txt");
    fuzz_parse_expr_bytes(seed).expect("lit_int seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_lit_neg_seed() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/lit_neg.txt");
    fuzz_parse_expr_bytes(seed).expect("lit_neg seed should parse");
}

#[test]
fn fuzz_parse_expr_bytes_accepts_nested_paren_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/parse_expr/nested_paren.txt");
    fuzz_parse_expr_bytes(seed).expect("nested_paren seed should parse");
}

#[test]
fn fuzz_query_str_bytes_accepts_cmp_col_eq_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/cmp_col_eq.bin");
    fuzz_query_str_bytes(seed).expect("cmp_col_eq seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_cmp_col_gt_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/cmp_col_gt.bin");
    fuzz_query_str_bytes(seed).expect("cmp_col_gt seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_cmp_col_lt_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/cmp_col_lt.bin");
    fuzz_query_str_bytes(seed).expect("cmp_col_lt seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_cmp_col_neq_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/cmp_col_neq.bin");
    fuzz_query_str_bytes(seed).expect("cmp_col_neq seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_cmp_gt_zero_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/cmp_gt_zero.bin");
    fuzz_query_str_bytes(seed).expect("cmp_gt_zero seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_cmp_lt_zero_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/cmp_lt_zero.bin");
    fuzz_query_str_bytes(seed).expect("cmp_lt_zero seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_filter_all_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/filter_all.bin");
    fuzz_query_str_bytes(seed).expect("filter_all seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_filter_none_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/filter_none.bin");
    fuzz_query_str_bytes(seed).expect("filter_none seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_filter_some_seed_fixture() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/filter_some.bin");
    fuzz_query_str_bytes(seed).expect("filter_some seed should satisfy invariants");
}

#[test]
fn fuzz_query_str_bytes_accepts_float_cmp_seed_fixture() {
    let seed = include_bytes!("../../fixtures/adversarial/fuzz_corpus/query_str/float_cmp.bin");
    fuzz_query_str_bytes(seed).expect("float_cmp seed should satisfy invariants");
}
