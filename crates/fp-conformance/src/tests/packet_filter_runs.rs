//! Packet-filter smoke tests extracted from fp-conformance/src/lib.rs's
//! `mod tests` per br-frankenpandas-lxhr monolith-split slice.
//!
//! Each test calls `run_packet_by_id` for one specific `FP-P2x-XXX`
//! packet id, asserts the returned report is green, and (for most
//! cases) that at least one fixture ran.
//!
//! The tests are self-contained: they only touch crate-level pub
//! items (HarnessConfig / run_packet_by_id / OracleMode) so the
//! standard `use super::*;` pattern suffices. No inner-mod helpers
//! are referenced.

use super::*;


#[test]
fn packet_filter_runs_only_requested_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2C-002", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-002"));
    assert!(
        report.fixture_count >= 3,
        "expected dedicated FP-P2C-002 fixtures"
    );
    assert!(report.is_green());
}

#[test]
fn packet_filter_runs_series_add_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2C-003", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-003"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2C-003 series_add fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_index_monotonic_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-058", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-058"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-058 index monotonic fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_join_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2C-004", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-004"));
    assert!(report.fixture_count >= 3, "expected join packet fixtures");
    assert!(report.is_green());
}

#[test]
fn packet_filter_runs_groupby_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2C-005", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-005"));
    assert!(
        report.fixture_count >= 4,
        "expected groupby packet fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_groupby_aggregate_matrix_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2C-011", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-011"));
    assert!(
        report.fixture_count >= 23,
        "expected FP-P2C-011 aggregate matrix fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_concat_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-014", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-014"));
    assert!(
        report.fixture_count >= 8,
        "expected FP-P2D-014 dataframe merge/concat fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_nanops_matrix_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-015", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-015"));
    assert!(
        report.fixture_count >= 14,
        "expected FP-P2D-015 nanops matrix fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_csv_edge_case_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-016", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-016"));
    assert!(
        report.fixture_count >= 14,
        "expected FP-P2D-016 csv edge-case fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_constructor_dtype_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-017", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-017"));
    assert!(
        report.fixture_count >= 15,
        "expected FP-P2D-017 constructor+dtype fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_constructor_matrix_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-018", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-018"));
    assert!(
        report.fixture_count >= 14,
        "expected FP-P2D-018 dataframe constructor fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_constructor_kwargs_matrix_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-019", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-019"));
    assert!(
        report.fixture_count >= 14,
        "expected FP-P2D-019 constructor kwargs fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_constructor_scalar_and_dict_series_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-020", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-020"));
    assert!(
        report.fixture_count >= 14,
        "expected FP-P2D-020 constructor scalar+dict-of-series fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_constructor_list_like_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-021", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-021"));
    assert!(
        report.fixture_count >= 14,
        "expected FP-P2D-021 constructor list-like fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_constructor_shape_taxonomy_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-022", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-022"));
    assert!(
        report.fixture_count >= 14,
        "expected FP-P2D-022 list-like shape taxonomy fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_constructor_dtype_kwargs_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-023", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-023"));
    assert!(
        report.fixture_count >= 14,
        "expected FP-P2D-023 constructor dtype/copy fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_constructor_dtype_spec_normalization_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-024", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-024"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-024 constructor dtype-spec normalization fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_loc_iloc_multi_axis_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-025", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-025"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-025 dataframe loc/iloc multi-axis fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_head_tail_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-026", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-026"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-026 dataframe head/tail fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_head_tail_negative_n_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-027", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-027"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-027 dataframe head/tail negative-n fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_concat_axis1_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-028", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-028"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-028 dataframe concat axis=1 fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_concat_axis1_inner_join_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-029", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-029"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-029 dataframe concat axis=1 inner-join fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_concat_axis0_inner_join_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-030", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-030"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-030 dataframe concat axis=0 inner-join fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_concat_axis0_outer_join_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-031", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-031"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-031 dataframe concat axis=0 outer-join fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_concat_axis0_outer_column_order_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-032", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-032"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-032 dataframe concat axis=0 outer column-order fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_composite_key_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-033", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-033"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-033 dataframe composite-key merge fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_indicator_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-034", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-034"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-034 dataframe merge indicator fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_validate_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-035", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-035"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-035 dataframe merge validate fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_suffix_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-036", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-036"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-036 dataframe merge suffix fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_sort_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-037", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-037"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-037 dataframe merge sort fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_sort_index_alias_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-038", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-038"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-038 dataframe merge sort index/alias fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_cross_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-039", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-039"));
    assert!(
        report.fixture_count >= 5,
        "expected FP-P2D-039 dataframe merge cross-join fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_sort_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-040", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-040"));
    assert!(
        report.fixture_count >= 10,
        "expected FP-P2D-040 dataframe sort index/value fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_any_all_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-041", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-041"));
    assert!(
        report.fixture_count >= 8,
        "expected FP-P2D-041 series any/all fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_value_counts_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-042", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-042"));
    assert!(
        report.fixture_count >= 6,
        "expected FP-P2D-042 series value_counts fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_sort_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-043", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-043"));
    assert!(
        report.fixture_count >= 6,
        "expected FP-P2D-043 series sort index/value fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_tail_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-044", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-044"));
    assert!(
        report.fixture_count >= 6,
        "expected FP-P2D-044 series tail fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_isna_notna_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-045", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-045"));
    assert!(
        report.fixture_count >= 6,
        "expected FP-P2D-045 series isna/notna fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_fillna_dropna_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-046", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-046"));
    assert!(
        report.fixture_count >= 8,
        "expected FP-P2D-046 series fillna/dropna fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_count_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-047", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-047"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-047 series count fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_isnull_notnull_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-048", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-048"));
    assert!(
        report.fixture_count >= 6,
        "expected FP-P2D-048 series isnull/notnull fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_isna_notna_isnull_notnull_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-049", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-049"));
    assert!(
        report.fixture_count >= 6,
        "expected FP-P2D-049 dataframe missingness fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_fillna_dropna_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-050", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-050"));
    assert!(
        report.fixture_count >= 8,
        "expected FP-P2D-050 dataframe fillna/dropna fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_count_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-051", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-051"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-051 dataframe count fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_dropna_columns_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-052", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-052"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-052 dataframe dropna(axis=1) fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_set_reset_index_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-053", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-053"));
    assert!(
        report.fixture_count >= 7,
        "expected FP-P2D-053 dataframe set/reset index fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_duplicates_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-054", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-054"));
    assert!(
        report.fixture_count >= 9,
        "expected FP-P2D-054 dataframe duplicated/drop_duplicates fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_arithmetic_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-055", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-055"));
    assert!(
        report.fixture_count >= 6,
        "expected FP-P2D-055 series sub/mul/div fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_asof_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-056", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-056"));
    assert!(
        report.fixture_count >= 2,
        "expected FP-P2D-056 dataframe merge_asof fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_rank_mode_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-057", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-057"));
    assert!(
        report.fixture_count >= 7,
        "expected FP-P2D-057 dataframe rank/mode fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_diff_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-060", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-060"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-060 series_diff fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_shift_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-061", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-061"));
    assert!(
        report.fixture_count >= 2,
        "expected FP-P2D-061 series_shift fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_pct_change_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-062", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-062"));
    assert!(
        report.fixture_count >= 2,
        "expected FP-P2D-062 series_pct_change fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_melt_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-063", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-063"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-063 dataframe_melt fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_combine_first_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-090", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-090"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-090 combine_first fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_asof_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-419", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-419"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-419 series asof fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_explode_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-420", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-420"));
    assert!(
        report.fixture_count >= 2,
        "expected FP-P2D-420 dataframe explode fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_abs_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-064", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-064"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-064 series_abs fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_round_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-065", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-065"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-065 series_round fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_cumsum_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-066", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-066"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-066 series_cumsum fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_cumprod_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-067", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-067"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-067 series_cumprod fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_cummax_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-068", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-068"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-068 series_cummax fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_cummin_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-069", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-069"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-069 series_cummin fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_nlargest_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-070", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-070"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-070 series_nlargest fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_nsmallest_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-071", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-071"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-071 series_nsmallest fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_between_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-072", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-072"));
    assert!(
        report.fixture_count >= 5,
        "expected FP-P2D-072 series_between fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_cumsum_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-073", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-073"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-073 dataframe_cumsum fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_cumprod_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-074", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-074"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-074 dataframe_cumprod fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_cummax_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-075", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-075"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-075 dataframe_cummax fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_cummin_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-076", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-076"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-076 dataframe_cummin fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_repeat_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-077", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-077"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-077 series_repeat fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_xs_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-078", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-078"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-078 series_xs fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_take_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-079", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-079"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-079 series_take fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_bool_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-080", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-080"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-080 series_bool fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_cut_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-081", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-081"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-081 series_cut fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_qcut_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-082", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-082"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-082 series_qcut fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_to_numeric_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-083", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-083"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-083 series_to_numeric fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_partition_df_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-084", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-084"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-084 series_partition_df fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_rpartition_df_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-085", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-085"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-085 series_rpartition_df fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_extract_df_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-086", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-086"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-086 series_extract_df fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_extractall_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-087", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-087"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-087 series_extractall fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_at_time_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-088", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-088"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-088 series_at_time fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_between_time_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-089", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-089"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-089 series_between_time fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_to_datetime_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-091", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-091"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-091 series_to_datetime fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_to_timedelta_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-092", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-092"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-092 series_to_timedelta fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_to_arrow_round_trip_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-427", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-427"));
    assert_eq!(report.fixture_count, 1);
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_rolling_min_min_periods_zero_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-428", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-428"));
    assert_eq!(report.fixture_count, 1);
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_csv_read_frame_parse_dates_mixed_timezone_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-429", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-429"));
    assert_eq!(report.fixture_count, 1);
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_csv_read_frame_parse_dates_combined_columns_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-432", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-432"));
    assert_eq!(report.fixture_count, 1);
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_to_json_records_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-433", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-433"));
    assert_eq!(report.fixture_count, 1);
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_timedelta_total_seconds_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-093", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-093"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-093 series_timedelta_total_seconds fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_asof_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-094", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-094"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-094 dataframe_asof fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_at_time_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-095", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-095"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-095 dataframe_at_time fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_between_time_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-096", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-096"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-096 dataframe_between_time fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_bool_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-097", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-097"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-097 dataframe_bool fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_xs_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-098", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-098"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-098 dataframe_xs fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_take_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-099", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-099"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-099 dataframe_take fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_idxmin_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-100", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-100"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-100 dataframe_groupby_idxmin fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_idxmax_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-101", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-101"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-101 dataframe_groupby_idxmax fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_any_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-102", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-102"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-102 dataframe_groupby_any fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_all_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-103", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-103"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-103 dataframe_groupby_all fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_get_group_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-104", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-104"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-104 dataframe_groupby_get_group fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_ffill_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-105", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-105"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-105 dataframe_groupby_ffill fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_bfill_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-106", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-106"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-106 dataframe_groupby_bfill fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_sem_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-107", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-107"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-107 dataframe_groupby_sem fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_skew_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-108", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-108"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-108 dataframe_groupby_skew fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_kurtosis_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-109", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-109"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-109 dataframe_groupby_kurtosis fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_ohlc_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-110", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-110"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-110 dataframe_groupby_ohlc fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_cumcount_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-111", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-111"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-111 dataframe_groupby_cumcount fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_ngroup_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-112", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-112"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-112 dataframe_groupby_ngroup fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_sum_observed_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-422", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-422"));
    assert_eq!(report.fixture_count, 1);
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_groupby_agg_multi_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-430", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-430"));
    assert_eq!(report.fixture_count, 1);
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_split_expand_n_padding_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-431", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-431"));
    assert_eq!(report.fixture_count, 1);
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_shift_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-113", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-113"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-113 dataframe_shift fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_shift_axis1_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-144", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-144"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-144 dataframe shift axis1 fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_merge_ordered_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-114", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-114"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-114 dataframe_merge_ordered fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_mode_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-115", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-115"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-115 series_mode fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_rank_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-116", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-116"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-116 series_rank fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_describe_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-117", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-117"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-117 series_describe fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_duplicated_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-118", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-118"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-118 series_duplicated fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_drop_duplicates_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-119", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-119"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-119 series_drop_duplicates fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_where_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-120", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-120"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-120 series_where fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_mask_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-121", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-121"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-121 series_mask fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_replace_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-122", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-122"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-122 series_replace fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_update_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-123", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-123"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-123 series_update fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_map_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-124", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-124"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-124 series_map fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_series_to_frame_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-125", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-125"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-125 series_to_frame fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_eval_query_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-126", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-126"));
    assert!(
        report.fixture_count >= 5,
        "expected FP-P2D-126 dataframe eval/query fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_window_resample_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-127", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-127"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-127 window/resample fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_reshape_dummy_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-128", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-128"));
    assert!(
        report.fixture_count >= 12,
        "expected FP-P2D-128 reshape/pivot/dummy fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_io_round_trip_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-129", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-129"));
    assert!(
        report.fixture_count >= 9,
        "expected FP-P2D-129 io round-trip fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_numeric_transform_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-130", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-130"));
    assert!(
        report.fixture_count >= 6,
        "expected FP-P2D-130 dataframe numeric transform fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_transpose_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-131", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-131"));
    assert!(
        report.fixture_count >= 5,
        "expected FP-P2D-131 dataframe transpose fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_topn_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-132", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-132"));
    assert!(
        report.fixture_count >= 6,
        "expected FP-P2D-132 dataframe top-N fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_insert_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-133", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-133"));
    assert!(
        report.fixture_count >= 5,
        "expected FP-P2D-133 dataframe insert fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_assign_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-134", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-134"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-134 dataframe assign fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_rename_columns_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-135", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-135"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-135 dataframe rename_columns fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_reindex_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-136", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-136"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-136 dataframe reindex fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_reindex_columns_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-137", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-137"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-137 dataframe reindex_columns fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_drop_columns_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-138", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-138"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-138 dataframe drop_columns fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_replace_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-139", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-139"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-139 dataframe replace fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_where_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-140", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-140"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-140 dataframe where fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_mask_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-141", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-141"));
    assert!(
        report.fixture_count >= 4,
        "expected FP-P2D-141 dataframe mask fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_where_df_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-142", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-142"));
    assert!(
        report.fixture_count >= 5,
        "expected FP-P2D-142 dataframe where_df fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_mask_df_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-143", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-143"));
    assert!(
        report.fixture_count >= 5,
        "expected FP-P2D-143 dataframe mask_df fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_describe_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-145", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-145"));
    assert!(
        report.fixture_count >= 2,
        "expected FP-P2D-145 dataframe_describe fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_idx_extrema_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-148", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-148"));
    assert!(
        report.fixture_count >= 2,
        "expected FP-P2D-148 dataframe idxmin/idxmax fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_corr_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-146", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-146"));
    assert!(
        report.fixture_count >= 3,
        "expected FP-P2D-146 dataframe corr fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_cov_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-147", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-147"));
    assert!(
        report.fixture_count >= 2,
        "expected FP-P2D-147 dataframe cov fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_sem_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-149", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-149"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-149 dataframe sem fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_skew_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-167", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-167"));
    assert!(
        report.fixture_count >= 2,
        "expected FP-P2D-167 dataframe skew fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_kurtosis_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-151", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-151"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-151 dataframe kurtosis fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_kurtosis_extreme_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-150", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-150"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-150 dataframe kurtosis fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_prod_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-168", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-168"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-168 dataframe prod fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_sum_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-152", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-152"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-152 dataframe sum fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_mean_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-153", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-153"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-153 dataframe mean fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_std_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-154", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-154"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-154 dataframe std fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_var_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-155", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-155"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-155 dataframe var fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_min_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-156", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-156"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-156 dataframe min fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_max_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-157", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-157"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-157 dataframe max fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_median_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-158", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-158"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-158 dataframe median fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_any_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-159", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-159"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-159 dataframe any fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_all_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-160", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-160"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-160 dataframe all fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_nunique_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-161", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-161"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-161 dataframe nunique fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_quantile_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-162", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-162"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-162 dataframe quantile fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_value_counts_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-163", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-163"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-163 dataframe value_counts fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_memory_usage_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-164", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-164"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-164 dataframe memory_usage fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_head_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-165", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-165"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-165 dataframe head fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}

#[test]
fn packet_filter_runs_dataframe_tail_packet() {
    let cfg = HarnessConfig::default_paths();
    let report =
        run_packet_by_id(&cfg, "FP-P2D-166", OracleMode::FixtureExpected).expect("report");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-166"));
    assert!(
        report.fixture_count >= 1,
        "expected FP-P2D-166 dataframe tail fixtures"
    );
    assert!(report.is_green(), "expected report green: {report:?}");
}
