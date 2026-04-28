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
    let err =
        fuzz_ipc_stream_io_bytes(seed).expect_err("invalid IPC stream bytes should error");
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
fn fuzz_format_cross_round_trip_bytes_accepts_all_arrow_format_pairs() {
    let payload = [
        3, 2, 0, 1, 2, 3, 10, 20, 11, 21, 12, 22, 13, 23, 14, 24, 15, 25, 16, 26, 17, 27, 18,
        28, 19, 29, 20, 30, 21, 31, 22, 32, 23, 33, 24, 34,
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
        7, 1, 10, 20, 30, 40, 50, 2, 11, 21, 31, 41, 51, 3, 12, 22, 32, 42, 52, 4, 13, 23, 33,
        43, 53, 5, 14, 24, 34, 44, 54, 6, 15, 25, 35, 45, 55, 7, 16, 26, 36, 46, 56,
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
        0, 3, 6, 0, 1, 0, 5, 0, 2, 1, 6, 1, 7, 2, 0, 2, 8, 3, 9, 1, 1, 4, 10, 2, 11, 0, 12, 3,
        13, 1, 14, 2,
    ];
    fuzz_pivot_table_bytes(&seed)
        .expect("raw projection mode should satisfy pivot_table invariants");
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
fn fuzz_semantic_eq_bytes_accepts_empty_input() {
    fuzz_semantic_eq_bytes(&[]).expect("empty input should be a no-op");
}

#[test]
fn fuzz_semantic_eq_bytes_locks_nan_missing_bridge() {
    fuzz_semantic_eq_bytes(&[3, 0, 3, 0]).expect("nan/missing bridge seed should hold");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_empty_input() {
    fuzz_dataframe_eval_bytes(&[]).expect("empty input should be a no-op");
}

#[test]
fn fuzz_dataframe_eval_bytes_accepts_simple_numeric_expression() {
    let mut seed = vec![0, b'a', b'+', b'b'];
    seed.extend([
        4, 2, 0, 1, 10, 20, 11, 21, 12, 22, 13, 23, 14, 24, 15, 25, 16, 26, 17, 27, 18, 28, 19,
        29, 20, 30, 21, 31,
    ]);
    fuzz_dataframe_eval_bytes(&seed).expect("simple eval seed should satisfy invariants");
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
fn fuzz_scalar_cast_bytes_accepts_identity_int64_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/scalar_cast/identity_int64_seed.bin"
    );
    fuzz_scalar_cast_bytes(seed).expect("identity cast seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_missing_float_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/scalar_cast/missing_float_seed.bin"
    );
    fuzz_scalar_cast_bytes(seed).expect("missing float seed should satisfy invariants");
}

#[test]
fn fuzz_scalar_cast_bytes_accepts_lossy_float_error_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/scalar_cast/lossy_float_to_int_seed.bin"
    );
    fuzz_scalar_cast_bytes(seed)
        .expect("lossy float cast seed should still satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_unique_overlap_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/series_add/unique_overlap_seed.bin"
    );
    fuzz_series_add_bytes(seed).expect("unique-overlap seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_duplicate_cross_product_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/series_add/duplicate_cross_product_seed.bin"
    );
    fuzz_series_add_bytes(seed)
        .expect("duplicate cross-product seed should satisfy invariants");
}

#[test]
fn fuzz_series_add_bytes_accepts_missing_alignment_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/series_add/missing_alignment_seed.bin"
    );
    fuzz_series_add_bytes(seed).expect("missing alignment seed should satisfy invariants");
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
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/column_arith/div_identity_seed.bin"
    );
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
fn fuzz_join_series_bytes_accepts_inner_overlap_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/join_series/inner_overlap_seed.bin"
    );
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
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/join_series/cross_product_seed.bin"
    );
    fuzz_join_series_bytes(seed).expect("cross-product seed should satisfy invariants");
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
    fuzz_groupby_sum_bytes(seed)
        .expect("dropna=false null-group seed should satisfy invariants");
}

#[test]
fn fuzz_groupby_sum_bytes_accepts_alignment_seed() {
    let seed =
        include_bytes!("../../fixtures/adversarial/fuzz_corpus/groupby_sum/alignment_seed.bin");
    fuzz_groupby_sum_bytes(seed).expect("alignment seed should satisfy invariants");
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
    fuzz_index_align_bytes(seed)
        .expect("duplicate seed should satisfy multiplicity invariants");
}

#[test]
fn fuzz_index_align_bytes_accepts_utf8_right_only_seed() {
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/index_align/utf8_right_only_seed.bin"
    );
    fuzz_index_align_bytes(seed).expect("utf8 right-only seed should satisfy invariants");
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
    let seed = include_bytes!(
        "../../fixtures/adversarial/fuzz_corpus/fuzz_json_io/split_valid_seed.json"
    );
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
fn fuzz_json_io_bytes_reports_invalid_json() {
    let err = fuzz_json_io_bytes(br#"{"records": ["unterminated""#)
        .expect_err("invalid json should error");
    assert!(
        matches!(err, fp_io::IoError::Json(_)),
        "expected JSON parse error, got {err:?}"
    );
}
