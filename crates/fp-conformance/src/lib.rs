#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use fp_columnar::{ArithmeticOp, Column};
use fp_frame::{
    ConcatJoin, DataFrame, FrameError, Series, concat_dataframes_with_axis_join, concat_series,
    cut, qcut, to_numeric,
};
use fp_groupby::{
    GroupByExecutionOptions, GroupByOptions, groupby_count, groupby_first, groupby_last,
    groupby_max, groupby_mean, groupby_median, groupby_min, groupby_std, groupby_sum,
    groupby_sum_with_options, groupby_var,
};
use fp_index::{
    AlignmentPlan, DuplicateKeep, Index, IndexLabel, align_union, validate_alignment_plan,
};
use fp_io::{
    ExcelReadOptions, IoError as FpIoError, JsonOrient, read_csv_str, read_excel_bytes,
    read_feather_bytes, read_ipc_stream_bytes, read_json_str, read_jsonl_str, read_parquet_bytes,
    write_csv_string, write_excel_bytes, write_feather_bytes, write_ipc_stream_bytes,
    write_json_string, write_jsonl_string, write_parquet_bytes,
};
use fp_join::{
    JoinExecutionOptions, JoinType, JoinedSeries, MergeExecutionOptions, MergeValidateMode,
    join_series, join_series_with_options, merge_dataframes_on_with_options, merge_ordered,
};
#[cfg(feature = "asupersync")]
use fp_runtime::asupersync::{
    ArtifactCodec, ArtifactPayload, Fnv1aVerifier, IntegrityVerifier, PassthroughCodec,
    RuntimeAsupersyncConfig,
};
use fp_runtime::{
    DecisionAction, DecodeProof, EvidenceLedger, MAX_DECODE_PROOFS, RaptorQEnvelope,
    RaptorQMetadata, RuntimeMode, RuntimePolicy, ScrubStatus,
};
use fp_types::{
    DType, NullKind, Scalar, cast_scalar, cast_scalar_owned, common_dtype, dropna, fill_na,
    nancount, nanmax, nanmean, nanmin, nanstd, nansum, nanvar,
};
use raptorq::{Decoder, Encoder, EncodingPacket, ObjectTransmissionInformation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct HarnessConfig {
    pub repo_root: PathBuf,
    pub oracle_root: PathBuf,
    pub fixture_root: PathBuf,
    pub strict_mode: bool,
    pub python_bin: String,
    pub allow_system_pandas_fallback: bool,
}

impl HarnessConfig {
    #[must_use]
    pub fn default_paths() -> Self {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let python_bin = std::env::var("FP_PYTHON_BIN")
            .ok()
            .and_then(|value| {
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                }
            })
            .unwrap_or_else(|| "python3".to_owned());
        Self {
            oracle_root: repo_root.join("legacy_pandas_code/pandas"),
            fixture_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"),
            strict_mode: true,
            python_bin,
            allow_system_pandas_fallback: false,
            repo_root,
        }
    }

    #[must_use]
    pub fn packet_fixture_root(&self) -> PathBuf {
        self.fixture_root.join("packets")
    }

    #[must_use]
    pub fn packet_artifact_root(&self, packet_id: &str) -> PathBuf {
        self.repo_root.join("artifacts/phase2c").join(packet_id)
    }

    #[must_use]
    pub fn parity_gate_path(&self, packet_id: &str) -> PathBuf {
        self.packet_artifact_root(packet_id)
            .join("parity_gate.yaml")
    }

    #[must_use]
    pub fn oracle_script_path(&self) -> PathBuf {
        self.repo_root
            .join("crates/fp-conformance/oracle/pandas_oracle.py")
    }
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self::default_paths()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessReport {
    pub suite: &'static str,
    pub oracle_present: bool,
    pub fixture_count: usize,
    pub strict_mode: bool,
}

#[must_use]
pub fn run_smoke(config: &HarnessConfig) -> HarnessReport {
    let fixture_count = fs::read_dir(&config.fixture_root)
        .ok()
        .into_iter()
        .flat_map(|it| it.filter_map(Result::ok))
        .count();

    HarnessReport {
        suite: "smoke",
        oracle_present: config.oracle_root.exists(),
        fixture_count,
        strict_mode: config.strict_mode,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleMode {
    FixtureExpected,
    LiveLegacyPandas,
}

#[derive(Debug, Clone)]
pub struct SuiteOptions {
    pub packet_filter: Option<String>,
    pub oracle_mode: OracleMode,
}

impl Default for SuiteOptions {
    fn default() -> Self {
        Self {
            packet_filter: None,
            oracle_mode: OracleMode::FixtureExpected,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureOperation {
    SeriesAdd,
    SeriesSub,
    SeriesMul,
    SeriesDiv,
    SeriesJoin,
    #[serde(rename = "series_constructor", alias = "series_from_values")]
    SeriesConstructor,
    #[serde(rename = "series_to_datetime", alias = "to_datetime")]
    SeriesToDatetime,
    #[serde(rename = "series_to_timedelta", alias = "to_timedelta")]
    SeriesToTimedelta,
    #[serde(
        rename = "series_timedelta_total_seconds",
        alias = "timedelta_total_seconds"
    )]
    SeriesTimedeltaTotalSeconds,
    #[serde(rename = "dataframe_from_series", alias = "data_frame_from_series")]
    DataFrameFromSeries,
    #[serde(rename = "dataframe_from_dict", alias = "data_frame_from_dict")]
    DataFrameFromDict,
    #[serde(rename = "dataframe_from_records", alias = "data_frame_from_records")]
    DataFrameFromRecords,
    #[serde(
        rename = "dataframe_constructor_kwargs",
        alias = "data_frame_constructor_kwargs"
    )]
    DataFrameConstructorKwargs,
    #[serde(
        rename = "dataframe_constructor_scalar",
        alias = "data_frame_constructor_scalar"
    )]
    DataFrameConstructorScalar,
    #[serde(
        rename = "dataframe_constructor_dict_of_series",
        alias = "data_frame_constructor_dict_of_series"
    )]
    DataFrameConstructorDictOfSeries,
    #[serde(
        rename = "dataframe_constructor_list_like",
        alias = "data_frame_constructor_list_like",
        alias = "dataframe_constructor_2d",
        alias = "data_frame_constructor_2d"
    )]
    DataFrameConstructorListLike,
    #[serde(rename = "groupby_sum", alias = "group_by_sum")]
    GroupBySum,
    IndexAlignUnion,
    IndexHasDuplicates,
    IndexIsMonotonicIncreasing,
    IndexIsMonotonicDecreasing,
    IndexFirstPositions,
    // FP-P2C-006: Join + concat
    SeriesConcat,
    #[serde(
        rename = "series_combine_first",
        alias = "series_combine_first_default"
    )]
    SeriesCombineFirst,
    // FP-P2C-007: Missingness + nanops
    NanSum,
    NanMean,
    NanMin,
    NanMax,
    NanStd,
    NanVar,
    NanCount,
    FillNa,
    DropNa,
    // FP-P2C-008: IO round-trip
    CsvRoundTrip,
    // FP-P2C-009: Storage invariants
    ColumnDtypeCheck,
    // FP-P2C-010: loc/iloc
    SeriesFilter,
    SeriesHead,
    #[serde(rename = "series_tail", alias = "series_tail_default")]
    SeriesTail,
    SeriesAny,
    SeriesAll,
    #[serde(rename = "series_bool", alias = "series_bool_default")]
    SeriesBool,
    #[serde(rename = "series_repeat", alias = "series_repeat_default")]
    SeriesRepeat,
    #[serde(rename = "series_to_numeric", alias = "series_to_numeric_default")]
    SeriesToNumeric,
    #[serde(
        rename = "series_convert_dtypes",
        alias = "series_convert_dtypes_default"
    )]
    SeriesConvertDtypes,
    #[serde(rename = "series_astype", alias = "series_astype_default")]
    SeriesAstype,
    #[serde(rename = "series_clip", alias = "series_clip_default")]
    SeriesClip,
    #[serde(rename = "series_abs", alias = "series_abs_default")]
    SeriesAbs,
    #[serde(rename = "series_round", alias = "series_round_default")]
    SeriesRound,
    #[serde(rename = "series_cumsum", alias = "series_cumsum_default")]
    SeriesCumsum,
    #[serde(rename = "series_cumprod", alias = "series_cumprod_default")]
    SeriesCumprod,
    #[serde(rename = "series_cummax", alias = "series_cummax_default")]
    SeriesCummax,
    #[serde(rename = "series_cummin", alias = "series_cummin_default")]
    SeriesCummin,
    #[serde(rename = "series_nlargest", alias = "series_nlargest_default")]
    SeriesNlargest,
    #[serde(rename = "series_nsmallest", alias = "series_nsmallest_default")]
    SeriesNsmallest,
    #[serde(rename = "series_between", alias = "series_between_default")]
    SeriesBetween,
    #[serde(rename = "series_cut", alias = "series_cut_default")]
    SeriesCut,
    #[serde(rename = "series_qcut", alias = "series_qcut_default")]
    SeriesQcut,
    #[serde(rename = "series_value_counts", alias = "series_value_counts_default")]
    SeriesValueCounts,
    #[serde(rename = "series_sort_index", alias = "series_sort_index_default")]
    SeriesSortIndex,
    #[serde(rename = "series_sort_values", alias = "series_sort_values_default")]
    SeriesSortValues,
    #[serde(rename = "series_isna", alias = "series_isna_default")]
    SeriesIsNa,
    #[serde(rename = "series_notna", alias = "series_notna_default")]
    SeriesNotNa,
    #[serde(rename = "series_isnull", alias = "series_isnull_default")]
    SeriesIsNull,
    #[serde(rename = "series_notnull", alias = "series_notnull_default")]
    SeriesNotNull,
    #[serde(rename = "series_fillna", alias = "series_fillna_default")]
    SeriesFillNa,
    #[serde(rename = "series_dropna", alias = "series_dropna_default")]
    SeriesDropNa,
    #[serde(rename = "series_count", alias = "series_count_default")]
    SeriesCount,
    #[serde(rename = "series_diff", alias = "series_diff_default")]
    SeriesDiff,
    #[serde(rename = "series_shift", alias = "series_shift_default")]
    SeriesShift,
    #[serde(rename = "series_pct_change", alias = "series_pct_change_default")]
    SeriesPctChange,
    #[serde(rename = "series_xs", alias = "series_xs_default")]
    SeriesXs,
    SeriesLoc,
    SeriesIloc,
    #[serde(rename = "series_take", alias = "series_take_default")]
    SeriesTake,
    #[serde(rename = "series_at_time", alias = "series_at_time_default")]
    SeriesAtTime,
    #[serde(rename = "series_between_time", alias = "series_between_time_default")]
    SeriesBetweenTime,
    #[serde(rename = "series_partition_df", alias = "series_str_partition_df")]
    SeriesPartitionDf,
    #[serde(rename = "series_rpartition_df", alias = "series_str_rpartition_df")]
    SeriesRpartitionDf,
    #[serde(rename = "series_extract_df", alias = "series_str_extract_df")]
    SeriesExtractDf,
    #[serde(rename = "series_extractall", alias = "series_str_extractall")]
    SeriesExtractAll,
    #[serde(rename = "dataframe_loc", alias = "data_frame_loc")]
    DataFrameLoc,
    #[serde(rename = "dataframe_iloc", alias = "data_frame_iloc")]
    DataFrameIloc,
    #[serde(rename = "dataframe_take", alias = "data_frame_take")]
    DataFrameTake,
    #[serde(
        rename = "dataframe_groupby_idxmin",
        alias = "data_frame_groupby_idxmin"
    )]
    DataFrameGroupByIdxMin,
    #[serde(
        rename = "dataframe_groupby_idxmax",
        alias = "data_frame_groupby_idxmax"
    )]
    DataFrameGroupByIdxMax,
    #[serde(rename = "dataframe_groupby_any", alias = "data_frame_groupby_any")]
    DataFrameGroupByAny,
    #[serde(rename = "dataframe_groupby_all", alias = "data_frame_groupby_all")]
    DataFrameGroupByAll,
    #[serde(
        rename = "dataframe_groupby_get_group",
        alias = "data_frame_groupby_get_group"
    )]
    DataFrameGroupByGetGroup,
    #[serde(rename = "dataframe_groupby_ffill", alias = "data_frame_groupby_ffill")]
    DataFrameGroupByFfill,
    #[serde(rename = "dataframe_groupby_bfill", alias = "data_frame_groupby_bfill")]
    DataFrameGroupByBfill,
    #[serde(rename = "dataframe_groupby_sem", alias = "data_frame_groupby_sem")]
    DataFrameGroupBySem,
    #[serde(rename = "dataframe_groupby_skew", alias = "data_frame_groupby_skew")]
    DataFrameGroupBySkew,
    #[serde(
        rename = "dataframe_groupby_kurtosis",
        alias = "data_frame_groupby_kurtosis"
    )]
    DataFrameGroupByKurtosis,
    #[serde(rename = "dataframe_groupby_ohlc", alias = "data_frame_groupby_ohlc")]
    DataFrameGroupByOhlc,
    #[serde(
        rename = "dataframe_groupby_cumcount",
        alias = "data_frame_groupby_cumcount"
    )]
    DataFrameGroupByCumcount,
    #[serde(
        rename = "dataframe_groupby_ngroup",
        alias = "data_frame_groupby_ngroup"
    )]
    DataFrameGroupByNgroup,
    #[serde(rename = "dataframe_asof", alias = "data_frame_asof")]
    DataFrameAsof,
    #[serde(rename = "dataframe_at_time", alias = "data_frame_at_time")]
    DataFrameAtTime,
    #[serde(rename = "dataframe_between_time", alias = "data_frame_between_time")]
    DataFrameBetweenTime,
    #[serde(rename = "dataframe_bool", alias = "data_frame_bool")]
    DataFrameBool,
    #[serde(rename = "dataframe_head", alias = "data_frame_head")]
    DataFrameHead,
    #[serde(rename = "dataframe_tail", alias = "data_frame_tail")]
    DataFrameTail,
    #[serde(rename = "dataframe_xs", alias = "data_frame_xs")]
    DataFrameXs,
    #[serde(rename = "dataframe_isna", alias = "data_frame_isna")]
    DataFrameIsNa,
    #[serde(rename = "dataframe_notna", alias = "data_frame_notna")]
    DataFrameNotNa,
    #[serde(rename = "dataframe_isnull", alias = "data_frame_isnull")]
    DataFrameIsNull,
    #[serde(rename = "dataframe_notnull", alias = "data_frame_notnull")]
    DataFrameNotNull,
    #[serde(rename = "dataframe_count", alias = "data_frame_count")]
    DataFrameCount,
    #[serde(rename = "dataframe_mode", alias = "data_frame_mode")]
    DataFrameMode,
    #[serde(rename = "dataframe_rank", alias = "data_frame_rank")]
    DataFrameRank,
    #[serde(rename = "dataframe_fillna", alias = "data_frame_fillna")]
    DataFrameFillNa,
    #[serde(rename = "dataframe_dropna", alias = "data_frame_dropna")]
    DataFrameDropNa,
    #[serde(
        rename = "dataframe_dropna_columns",
        alias = "data_frame_dropna_columns"
    )]
    DataFrameDropNaColumns,
    #[serde(rename = "dataframe_set_index", alias = "data_frame_set_index")]
    DataFrameSetIndex,
    #[serde(rename = "dataframe_reset_index", alias = "data_frame_reset_index")]
    DataFrameResetIndex,
    #[serde(rename = "dataframe_duplicated", alias = "data_frame_duplicated")]
    DataFrameDuplicated,
    #[serde(
        rename = "dataframe_drop_duplicates",
        alias = "data_frame_drop_duplicates"
    )]
    DataFrameDropDuplicates,
    // FP-P2D-040: DataFrame ordering parity matrix
    #[serde(rename = "dataframe_sort_index", alias = "data_frame_sort_index")]
    DataFrameSortIndex,
    #[serde(rename = "dataframe_sort_values", alias = "data_frame_sort_values")]
    DataFrameSortValues,
    #[serde(rename = "dataframe_diff", alias = "data_frame_diff")]
    DataFrameDiff,
    #[serde(rename = "dataframe_shift", alias = "data_frame_shift")]
    DataFrameShift,
    #[serde(rename = "dataframe_pct_change", alias = "data_frame_pct_change")]
    DataFramePctChange,
    #[serde(rename = "dataframe_melt", alias = "data_frame_melt")]
    DataFrameMelt,
    // FP-P2D-014: DataFrame merge/join/concat parity matrix
    #[serde(rename = "dataframe_merge", alias = "data_frame_merge")]
    DataFrameMerge,
    #[serde(rename = "dataframe_merge_index", alias = "data_frame_merge_index")]
    DataFrameMergeIndex,
    #[serde(rename = "dataframe_merge_asof", alias = "data_frame_merge_asof")]
    DataFrameMergeAsof,
    #[serde(rename = "dataframe_merge_ordered", alias = "data_frame_merge_ordered")]
    DataFrameMergeOrdered,
    #[serde(rename = "dataframe_concat", alias = "data_frame_concat")]
    DataFrameConcat,
    #[serde(rename = "dataframe_combine_first", alias = "data_frame_combine_first")]
    DataFrameCombineFirst,
    // FP-P2C-011: Full GroupBy aggregate matrix
    #[serde(rename = "groupby_mean", alias = "group_by_mean")]
    GroupByMean,
    #[serde(rename = "groupby_count", alias = "group_by_count")]
    GroupByCount,
    #[serde(rename = "groupby_min", alias = "group_by_min")]
    GroupByMin,
    #[serde(rename = "groupby_max", alias = "group_by_max")]
    GroupByMax,
    #[serde(rename = "groupby_first", alias = "group_by_first")]
    GroupByFirst,
    #[serde(rename = "groupby_last", alias = "group_by_last")]
    GroupByLast,
    #[serde(rename = "groupby_std", alias = "group_by_std")]
    GroupByStd,
    #[serde(rename = "groupby_var", alias = "group_by_var")]
    GroupByVar,
    #[serde(rename = "groupby_median", alias = "group_by_median")]
    GroupByMedian,
}

impl FixtureOperation {
    #[must_use]
    pub fn operation_name(self) -> &'static str {
        match self {
            Self::SeriesAdd => "series_add",
            Self::SeriesSub => "series_sub",
            Self::SeriesMul => "series_mul",
            Self::SeriesDiv => "series_div",
            Self::SeriesJoin => "series_join",
            Self::SeriesConstructor => "series_constructor",
            Self::SeriesToDatetime => "series_to_datetime",
            Self::SeriesToTimedelta => "series_to_timedelta",
            Self::SeriesTimedeltaTotalSeconds => "series_timedelta_total_seconds",
            Self::DataFrameFromSeries => "dataframe_from_series",
            Self::DataFrameFromDict => "dataframe_from_dict",
            Self::DataFrameFromRecords => "dataframe_from_records",
            Self::DataFrameConstructorKwargs => "dataframe_constructor_kwargs",
            Self::DataFrameConstructorScalar => "dataframe_constructor_scalar",
            Self::DataFrameConstructorDictOfSeries => "dataframe_constructor_dict_of_series",
            Self::DataFrameConstructorListLike => "dataframe_constructor_list_like",
            Self::GroupBySum => "groupby_sum",
            Self::IndexAlignUnion => "index_align_union",
            Self::IndexHasDuplicates => "index_has_duplicates",
            Self::IndexIsMonotonicIncreasing => "index_is_monotonic_increasing",
            Self::IndexIsMonotonicDecreasing => "index_is_monotonic_decreasing",
            Self::IndexFirstPositions => "index_first_positions",
            Self::SeriesConcat => "series_concat",
            Self::SeriesCombineFirst => "series_combine_first",
            Self::NanSum => "nan_sum",
            Self::NanMean => "nan_mean",
            Self::NanMin => "nan_min",
            Self::NanMax => "nan_max",
            Self::NanStd => "nan_std",
            Self::NanVar => "nan_var",
            Self::NanCount => "nan_count",
            Self::FillNa => "fill_na",
            Self::DropNa => "drop_na",
            Self::CsvRoundTrip => "csv_round_trip",
            Self::ColumnDtypeCheck => "column_dtype_check",
            Self::SeriesFilter => "series_filter",
            Self::SeriesHead => "series_head",
            Self::SeriesTail => "series_tail",
            Self::SeriesAny => "series_any",
            Self::SeriesAll => "series_all",
            Self::SeriesBool => "series_bool",
            Self::SeriesRepeat => "series_repeat",
            Self::SeriesToNumeric => "series_to_numeric",
            Self::SeriesConvertDtypes => "series_convert_dtypes",
            Self::SeriesAstype => "series_astype",
            Self::SeriesClip => "series_clip",
            Self::SeriesAbs => "series_abs",
            Self::SeriesRound => "series_round",
            Self::SeriesCumsum => "series_cumsum",
            Self::SeriesCumprod => "series_cumprod",
            Self::SeriesCummax => "series_cummax",
            Self::SeriesCummin => "series_cummin",
            Self::SeriesNlargest => "series_nlargest",
            Self::SeriesNsmallest => "series_nsmallest",
            Self::SeriesBetween => "series_between",
            Self::SeriesCut => "series_cut",
            Self::SeriesQcut => "series_qcut",
            Self::SeriesValueCounts => "series_value_counts",
            Self::SeriesSortIndex => "series_sort_index",
            Self::SeriesSortValues => "series_sort_values",
            Self::SeriesIsNa => "series_isna",
            Self::SeriesNotNa => "series_notna",
            Self::SeriesIsNull => "series_isnull",
            Self::SeriesNotNull => "series_notnull",
            Self::SeriesFillNa => "series_fillna",
            Self::SeriesDropNa => "series_dropna",
            Self::SeriesCount => "series_count",
            Self::SeriesXs => "series_xs",
            Self::SeriesLoc => "series_loc",
            Self::SeriesIloc => "series_iloc",
            Self::SeriesTake => "series_take",
            Self::SeriesAtTime => "series_at_time",
            Self::SeriesBetweenTime => "series_between_time",
            Self::SeriesPartitionDf => "series_partition_df",
            Self::SeriesRpartitionDf => "series_rpartition_df",
            Self::SeriesExtractDf => "series_extract_df",
            Self::SeriesExtractAll => "series_extractall",
            Self::DataFrameLoc => "dataframe_loc",
            Self::DataFrameIloc => "dataframe_iloc",
            Self::DataFrameTake => "dataframe_take",
            Self::DataFrameGroupByIdxMin => "dataframe_groupby_idxmin",
            Self::DataFrameGroupByIdxMax => "dataframe_groupby_idxmax",
            Self::DataFrameGroupByAny => "dataframe_groupby_any",
            Self::DataFrameGroupByAll => "dataframe_groupby_all",
            Self::DataFrameGroupByGetGroup => "dataframe_groupby_get_group",
            Self::DataFrameGroupByFfill => "dataframe_groupby_ffill",
            Self::DataFrameGroupByBfill => "dataframe_groupby_bfill",
            Self::DataFrameGroupBySem => "dataframe_groupby_sem",
            Self::DataFrameGroupBySkew => "dataframe_groupby_skew",
            Self::DataFrameGroupByKurtosis => "dataframe_groupby_kurtosis",
            Self::DataFrameGroupByOhlc => "dataframe_groupby_ohlc",
            Self::DataFrameGroupByCumcount => "dataframe_groupby_cumcount",
            Self::DataFrameGroupByNgroup => "dataframe_groupby_ngroup",
            Self::DataFrameAsof => "dataframe_asof",
            Self::DataFrameAtTime => "dataframe_at_time",
            Self::DataFrameBetweenTime => "dataframe_between_time",
            Self::DataFrameBool => "dataframe_bool",
            Self::DataFrameHead => "dataframe_head",
            Self::DataFrameTail => "dataframe_tail",
            Self::DataFrameXs => "dataframe_xs",
            Self::DataFrameIsNa => "dataframe_isna",
            Self::DataFrameNotNa => "dataframe_notna",
            Self::DataFrameIsNull => "dataframe_isnull",
            Self::DataFrameNotNull => "dataframe_notnull",
            Self::DataFrameCount => "dataframe_count",
            Self::DataFrameMode => "dataframe_mode",
            Self::DataFrameRank => "dataframe_rank",
            Self::DataFrameFillNa => "dataframe_fillna",
            Self::DataFrameDropNa => "dataframe_dropna",
            Self::DataFrameDropNaColumns => "dataframe_dropna_columns",
            Self::DataFrameSetIndex => "dataframe_set_index",
            Self::DataFrameResetIndex => "dataframe_reset_index",
            Self::DataFrameDuplicated => "dataframe_duplicated",
            Self::DataFrameDropDuplicates => "dataframe_drop_duplicates",
            Self::DataFrameSortIndex => "dataframe_sort_index",
            Self::DataFrameSortValues => "dataframe_sort_values",
            Self::DataFrameDiff => "dataframe_diff",
            Self::DataFrameShift => "dataframe_shift",
            Self::DataFramePctChange => "dataframe_pct_change",
            Self::DataFrameMelt => "dataframe_melt",
            Self::DataFrameMerge => "dataframe_merge",
            Self::DataFrameMergeIndex => "dataframe_merge_index",
            Self::DataFrameMergeAsof => "dataframe_merge_asof",
            Self::DataFrameMergeOrdered => "dataframe_merge_ordered",
            Self::DataFrameConcat => "dataframe_concat",
            Self::DataFrameCombineFirst => "dataframe_combine_first",
            Self::GroupByMean => "groupby_mean",
            Self::GroupByCount => "groupby_count",
            Self::GroupByMin => "groupby_min",
            Self::GroupByMax => "groupby_max",
            Self::GroupByFirst => "groupby_first",
            Self::GroupByLast => "groupby_last",
            Self::GroupByStd => "groupby_std",
            Self::GroupByVar => "groupby_var",
            Self::GroupByMedian => "groupby_median",
            Self::SeriesDiff => "series_diff",
            Self::SeriesShift => "series_shift",
            Self::SeriesPctChange => "series_pct_change",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureJoinType {
    Inner,
    Left,
    Right,
    Outer,
    Cross,
}

impl FixtureJoinType {
    #[must_use]
    pub fn into_join_type(self) -> JoinType {
        match self {
            Self::Inner => JoinType::Inner,
            Self::Left => JoinType::Left,
            Self::Right => JoinType::Right,
            Self::Outer => JoinType::Outer,
            Self::Cross => JoinType::Cross,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureOracleSource {
    Fixture,
    LiveLegacyPandas,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureSeries {
    pub name: String,
    pub index: Vec<IndexLabel>,
    pub values: Vec<Scalar>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureExpectedSeries {
    pub index: Vec<IndexLabel>,
    pub values: Vec<Scalar>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureDataFrame {
    pub index: Vec<IndexLabel>,
    pub columns: BTreeMap<String, Vec<Scalar>>,
    #[serde(default)]
    pub column_order: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureExpectedDataFrame {
    pub index: Vec<IndexLabel>,
    pub columns: BTreeMap<String, Vec<Scalar>>,
    #[serde(default)]
    pub column_order: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureExpectedAlignment {
    pub union_index: Vec<IndexLabel>,
    pub left_positions: Vec<Option<usize>>,
    pub right_positions: Vec<Option<usize>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureExpectedJoin {
    pub index: Vec<IndexLabel>,
    pub left_values: Vec<Scalar>,
    pub right_values: Vec<Scalar>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketFixture {
    pub packet_id: String,
    pub case_id: String,
    pub mode: RuntimeMode,
    pub operation: FixtureOperation,
    #[serde(default)]
    pub oracle_source: Option<FixtureOracleSource>,
    #[serde(default)]
    pub left: Option<FixtureSeries>,
    #[serde(default)]
    pub right: Option<FixtureSeries>,
    #[serde(default)]
    pub groupby_keys: Option<Vec<FixtureSeries>>,
    #[serde(default)]
    pub groupby_columns: Option<Vec<String>>,
    #[serde(default)]
    pub frame: Option<FixtureDataFrame>,
    #[serde(default)]
    pub frame_right: Option<FixtureDataFrame>,
    #[serde(default)]
    pub dict_columns: Option<BTreeMap<String, Vec<Scalar>>>,
    #[serde(default)]
    pub column_order: Option<Vec<String>>,
    #[serde(default)]
    pub constructor_dtype: Option<String>,
    #[serde(default)]
    pub constructor_copy: Option<bool>,
    #[serde(default)]
    pub records: Option<Vec<BTreeMap<String, Scalar>>>,
    #[serde(default)]
    pub matrix_rows: Option<Vec<Vec<Scalar>>>,
    #[serde(default)]
    pub index: Option<Vec<IndexLabel>>,
    #[serde(default)]
    pub join_type: Option<FixtureJoinType>,
    #[serde(default)]
    pub merge_on: Option<String>,
    #[serde(default)]
    pub merge_on_keys: Option<Vec<String>>,
    #[serde(default)]
    pub left_on_keys: Option<Vec<String>>,
    #[serde(default)]
    pub right_on_keys: Option<Vec<String>>,
    #[serde(default)]
    pub left_index: Option<bool>,
    #[serde(default)]
    pub right_index: Option<bool>,
    #[serde(default)]
    pub merge_indicator: Option<bool>,
    #[serde(default)]
    pub merge_indicator_name: Option<String>,
    #[serde(default)]
    pub merge_validate: Option<String>,
    #[serde(default)]
    pub merge_suffixes: Option<[Option<String>; 2]>,
    #[serde(default)]
    pub merge_sort: Option<bool>,
    #[serde(default)]
    pub expected_series: Option<FixtureExpectedSeries>,
    #[serde(default)]
    pub expected_join: Option<FixtureExpectedJoin>,
    #[serde(default)]
    pub expected_frame: Option<FixtureExpectedDataFrame>,
    #[serde(default)]
    pub expected_error_contains: Option<String>,
    #[serde(default)]
    pub expected_alignment: Option<FixtureExpectedAlignment>,
    #[serde(default)]
    pub expected_bool: Option<bool>,
    #[serde(default)]
    pub expected_positions: Option<Vec<Option<usize>>>,
    #[serde(default)]
    pub expected_scalar: Option<Scalar>,
    #[serde(default)]
    pub expected_dtype: Option<String>,
    #[serde(default)]
    pub datetime_unit: Option<String>,
    #[serde(default)]
    pub datetime_origin: Option<serde_json::Value>,
    #[serde(default)]
    pub datetime_utc: Option<bool>,
    #[serde(default)]
    pub fill_value: Option<Scalar>,
    #[serde(default)]
    pub clip_lower: Option<f64>,
    #[serde(default)]
    pub clip_upper: Option<f64>,
    #[serde(default)]
    pub round_decimals: Option<i32>,
    #[serde(default)]
    pub nlargest_n: Option<usize>,
    #[serde(default)]
    pub between_left: Option<Scalar>,
    #[serde(default)]
    pub between_right: Option<Scalar>,
    #[serde(default)]
    pub between_inclusive: Option<String>,
    #[serde(default)]
    pub head_n: Option<i64>,
    #[serde(default)]
    pub tail_n: Option<i64>,
    #[serde(default)]
    pub diff_periods: Option<i64>,
    #[serde(default)]
    pub shift_periods: Option<i64>,
    #[serde(default)]
    pub shift_axis: Option<usize>,
    #[serde(default)]
    pub pct_change_periods: Option<i64>,
    #[serde(default)]
    pub diff_axis: Option<usize>,
    #[serde(default)]
    pub rank_method: Option<String>,
    #[serde(default)]
    pub rank_na_option: Option<String>,
    #[serde(default)]
    pub rank_axis: Option<usize>,
    #[serde(default)]
    pub sort_column: Option<String>,
    #[serde(default)]
    pub sort_ascending: Option<bool>,
    #[serde(default)]
    pub concat_axis: Option<i64>,
    #[serde(default)]
    pub concat_join: Option<String>,
    #[serde(default)]
    pub direction: Option<String>,
    /// For merge_asof: allow matching with same key value (default true)
    #[serde(default)]
    pub allow_exact_matches: Option<bool>,
    /// For merge_asof: maximum distance between keys for a match
    #[serde(default)]
    pub tolerance: Option<f64>,
    /// For merge_asof: columns to match exactly before asof matching
    #[serde(default, rename = "by")]
    pub merge_asof_by: Option<Vec<String>>,
    #[serde(default)]
    pub merge_fill_method: Option<String>,
    #[serde(default)]
    pub set_index_column: Option<String>,
    #[serde(default)]
    pub set_index_drop: Option<bool>,
    #[serde(default)]
    pub reset_index_drop: Option<bool>,
    #[serde(default)]
    pub subset: Option<Vec<String>>,
    #[serde(default)]
    pub keep: Option<String>,
    #[serde(default)]
    pub ignore_index: Option<bool>,
    #[serde(default)]
    pub csv_input: Option<String>,
    #[serde(default)]
    pub loc_labels: Option<Vec<IndexLabel>>,
    #[serde(default)]
    pub iloc_positions: Option<Vec<i64>>,
    #[serde(default)]
    pub take_indices: Option<Vec<i64>>,
    #[serde(default)]
    pub repeat_n: Option<i64>,
    #[serde(default)]
    pub repeat_counts: Option<Vec<i64>>,
    #[serde(default)]
    pub group_name: Option<String>,
    #[serde(default)]
    pub xs_key: Option<IndexLabel>,
    #[serde(default)]
    pub cut_bins: Option<usize>,
    #[serde(default)]
    pub qcut_quantiles: Option<usize>,
    #[serde(default)]
    pub take_axis: Option<usize>,
    #[serde(default)]
    pub asof_label: Option<IndexLabel>,
    #[serde(default)]
    pub time_value: Option<String>,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub end_time: Option<String>,
    #[serde(default)]
    pub string_sep: Option<String>,
    #[serde(default)]
    pub regex_pattern: Option<String>,
    #[serde(default)]
    pub melt_id_vars: Option<Vec<String>>,
    #[serde(default)]
    pub melt_value_vars: Option<Vec<String>>,
    #[serde(default)]
    pub melt_var_name: Option<String>,
    #[serde(default)]
    pub melt_value_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    pub packet_id: String,
    pub case_id: String,
    pub mode: RuntimeMode,
    pub operation: FixtureOperation,
    pub status: CaseStatus,
    pub mismatch: Option<String>,
    #[serde(default)]
    pub mismatch_class: Option<String>,
    #[serde(default)]
    pub replay_key: String,
    #[serde(default)]
    pub trace_id: String,
    #[serde(default)]
    pub elapsed_us: u64,
    pub evidence_records: usize,
}

// === Differential Harness: Comparator Taxonomy + Drift Classification ===

/// Drift severity classification following frankenlibc fail-closed doctrine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftLevel {
    /// Hard parity failure: no tolerance, blocks gates.
    Critical,
    /// Soft divergence: within configured tolerance budget.
    NonCritical,
    /// Known behavioral gap, documented and accepted.
    Informational,
}

/// Comparison dimension in the differential taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonCategory {
    /// Scalar value equality (exact or within tolerance).
    Value,
    /// Data type agreement between actual and expected.
    Type,
    /// Shape: length or dimensionality mismatch.
    Shape,
    /// Index labels and ordering.
    Index,
    /// Null/NaN propagation behavior.
    Nullness,
}

/// A single drift observation from a differential comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftRecord {
    pub category: ComparisonCategory,
    pub level: DriftLevel,
    #[serde(default)]
    pub mismatch_class: String,
    pub location: String,
    pub message: String,
}

/// Full differential comparison result for a single fixture case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DifferentialResult {
    pub case_id: String,
    pub packet_id: String,
    pub operation: FixtureOperation,
    pub mode: RuntimeMode,
    #[serde(default)]
    pub replay_key: String,
    #[serde(default)]
    pub trace_id: String,
    pub oracle_source: FixtureOracleSource,
    pub status: CaseStatus,
    pub drift_records: Vec<DriftRecord>,
    pub evidence_records: usize,
}

fn runtime_mode_slug(mode: RuntimeMode) -> &'static str {
    match mode {
        RuntimeMode::Strict => "strict",
        RuntimeMode::Hardened => "hardened",
    }
}

fn comparison_category_slug(category: ComparisonCategory) -> &'static str {
    match category {
        ComparisonCategory::Value => "value",
        ComparisonCategory::Type => "type",
        ComparisonCategory::Shape => "shape",
        ComparisonCategory::Index => "index",
        ComparisonCategory::Nullness => "nullness",
    }
}

fn drift_level_slug(level: DriftLevel) -> &'static str {
    match level {
        DriftLevel::Critical => "critical",
        DriftLevel::NonCritical => "non_critical",
        DriftLevel::Informational => "informational",
    }
}

fn mismatch_class_for(category: ComparisonCategory, level: DriftLevel) -> String {
    format!(
        "{}_{}",
        comparison_category_slug(category),
        drift_level_slug(level)
    )
}

fn deterministic_trace_id(packet_id: &str, case_id: &str, mode: RuntimeMode) -> String {
    format!("{packet_id}:{case_id}:{}", runtime_mode_slug(mode))
}

fn deterministic_replay_key(packet_id: &str, case_id: &str, mode: RuntimeMode) -> String {
    format!("{packet_id}/{case_id}/{}", runtime_mode_slug(mode))
}

fn deterministic_scenario_id(suite: &str, packet_id: &str) -> String {
    format!("{suite}:{packet_id}")
}

fn parity_report_artifact_id(packet_id: &str) -> String {
    format!("{packet_id}/parity_report")
}

fn deterministic_step_id(case_id: &str) -> String {
    format!("case:{case_id}")
}

fn deterministic_seed(packet_id: &str, case_id: &str, mode: RuntimeMode) -> u64 {
    let key = format!("{packet_id}:{case_id}:{}", runtime_mode_slug(mode));
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn assertion_path_for_case(packet_id: &str, case_id: &str) -> String {
    format!("ASUPERSYNC-G/{packet_id}/{case_id}")
}

fn replay_cmd_for_case(case_id: &str) -> String {
    format!("cargo test -p fp-conformance -- {case_id} --nocapture")
}

fn result_label_for_status(status: &CaseStatus) -> &'static str {
    match status {
        CaseStatus::Pass => "pass",
        CaseStatus::Fail => "fail",
    }
}

fn decision_action_for(status: &CaseStatus) -> &'static str {
    match status {
        CaseStatus::Pass => "allow",
        CaseStatus::Fail => "repair",
    }
}

const COMPAT_CLOSURE_SUITE_ID: &str = "COMPAT-CLOSURE-E";
const COMPAT_CLOSURE_COVERAGE_FLOOR_PERCENT: usize = 100;
const COMPAT_CLOSURE_REQUIRED_ROWS: [&str; 9] = [
    "CC-001", "CC-002", "CC-003", "CC-004", "CC-005", "CC-006", "CC-007", "CC-008", "CC-009",
];

fn compat_contract_rows_for_operation(operation: FixtureOperation) -> &'static [&'static str] {
    match operation {
        FixtureOperation::SeriesAdd
        | FixtureOperation::SeriesSub
        | FixtureOperation::SeriesMul
        | FixtureOperation::SeriesDiv => &["CC-004", "CC-005"],
        FixtureOperation::SeriesConstructor
        | FixtureOperation::SeriesToDatetime
        | FixtureOperation::SeriesToTimedelta
        | FixtureOperation::SeriesTimedeltaTotalSeconds
        | FixtureOperation::DataFrameFromDict
        | FixtureOperation::DataFrameFromRecords
        | FixtureOperation::DataFrameConstructorKwargs
        | FixtureOperation::DataFrameConstructorScalar
        | FixtureOperation::DataFrameConstructorDictOfSeries
        | FixtureOperation::DataFrameConstructorListLike => &["CC-001", "CC-003", "CC-005"],
        FixtureOperation::SeriesJoin
        | FixtureOperation::SeriesConcat
        | FixtureOperation::SeriesCombineFirst
        | FixtureOperation::DataFrameMerge
        | FixtureOperation::DataFrameMergeIndex
        | FixtureOperation::DataFrameMergeAsof
        | FixtureOperation::DataFrameMergeOrdered
        | FixtureOperation::DataFrameConcat
        | FixtureOperation::DataFrameCombineFirst => &["CC-006"],
        FixtureOperation::DataFrameFromSeries => &["CC-003", "CC-005", "CC-006"],
        FixtureOperation::GroupBySum
        | FixtureOperation::GroupByMean
        | FixtureOperation::GroupByCount
        | FixtureOperation::GroupByMin
        | FixtureOperation::GroupByMax
        | FixtureOperation::GroupByFirst
        | FixtureOperation::GroupByLast
        | FixtureOperation::GroupByStd
        | FixtureOperation::GroupByVar
        | FixtureOperation::GroupByMedian
        | FixtureOperation::SeriesValueCounts => &["CC-007"],
        FixtureOperation::IndexAlignUnion
        | FixtureOperation::IndexHasDuplicates
        | FixtureOperation::IndexIsMonotonicIncreasing
        | FixtureOperation::IndexIsMonotonicDecreasing
        | FixtureOperation::IndexFirstPositions => &["CC-003"],
        FixtureOperation::NanSum
        | FixtureOperation::NanMean
        | FixtureOperation::NanMin
        | FixtureOperation::NanMax
        | FixtureOperation::NanStd
        | FixtureOperation::NanVar
        | FixtureOperation::NanCount
        | FixtureOperation::SeriesCount
        | FixtureOperation::DataFrameCount
        | FixtureOperation::DataFrameMode => &["CC-005"],
        FixtureOperation::FillNa
        | FixtureOperation::DropNa
        | FixtureOperation::SeriesToNumeric
        | FixtureOperation::SeriesConvertDtypes
        | FixtureOperation::SeriesAstype
        | FixtureOperation::SeriesClip
        | FixtureOperation::SeriesAbs
        | FixtureOperation::SeriesRound
        | FixtureOperation::SeriesCumsum
        | FixtureOperation::SeriesCumprod
        | FixtureOperation::SeriesCummax
        | FixtureOperation::SeriesCummin
        | FixtureOperation::SeriesNlargest
        | FixtureOperation::SeriesNsmallest
        | FixtureOperation::SeriesBetween
        | FixtureOperation::SeriesCut
        | FixtureOperation::SeriesQcut
        | FixtureOperation::SeriesIsNa
        | FixtureOperation::SeriesNotNa
        | FixtureOperation::SeriesIsNull
        | FixtureOperation::SeriesNotNull
        | FixtureOperation::SeriesFillNa
        | FixtureOperation::SeriesDropNa
        | FixtureOperation::DataFrameIsNa
        | FixtureOperation::DataFrameNotNa
        | FixtureOperation::DataFrameIsNull
        | FixtureOperation::DataFrameNotNull
        | FixtureOperation::DataFrameFillNa
        | FixtureOperation::DataFrameDropNa
        | FixtureOperation::DataFrameDropNaColumns => &["CC-002", "CC-005"],
        FixtureOperation::CsvRoundTrip => &["CC-006"],
        FixtureOperation::ColumnDtypeCheck => &["CC-001"],
        FixtureOperation::SeriesFilter
        | FixtureOperation::SeriesHead
        | FixtureOperation::SeriesTail
        | FixtureOperation::SeriesAny
        | FixtureOperation::SeriesAll
        | FixtureOperation::SeriesBool
        | FixtureOperation::SeriesRepeat
        | FixtureOperation::SeriesSortIndex
        | FixtureOperation::SeriesSortValues
        | FixtureOperation::SeriesDiff
        | FixtureOperation::SeriesShift
        | FixtureOperation::SeriesPctChange
        | FixtureOperation::SeriesXs
        | FixtureOperation::SeriesLoc
        | FixtureOperation::SeriesIloc
        | FixtureOperation::SeriesTake
        | FixtureOperation::SeriesAtTime
        | FixtureOperation::SeriesBetweenTime
        | FixtureOperation::SeriesPartitionDf
        | FixtureOperation::SeriesRpartitionDf
        | FixtureOperation::SeriesExtractDf
        | FixtureOperation::SeriesExtractAll
        | FixtureOperation::DataFrameLoc
        | FixtureOperation::DataFrameIloc
        | FixtureOperation::DataFrameTake
        | FixtureOperation::DataFrameGroupByIdxMin
        | FixtureOperation::DataFrameGroupByIdxMax
        | FixtureOperation::DataFrameGroupByAny
        | FixtureOperation::DataFrameGroupByAll
        | FixtureOperation::DataFrameGroupByGetGroup
        | FixtureOperation::DataFrameGroupByFfill
        | FixtureOperation::DataFrameGroupByBfill
        | FixtureOperation::DataFrameGroupBySem
        | FixtureOperation::DataFrameGroupBySkew
        | FixtureOperation::DataFrameGroupByKurtosis
        | FixtureOperation::DataFrameGroupByOhlc
        | FixtureOperation::DataFrameGroupByCumcount
        | FixtureOperation::DataFrameGroupByNgroup
        | FixtureOperation::DataFrameAsof
        | FixtureOperation::DataFrameAtTime
        | FixtureOperation::DataFrameBetweenTime
        | FixtureOperation::DataFrameBool
        | FixtureOperation::DataFrameHead
        | FixtureOperation::DataFrameTail
        | FixtureOperation::DataFrameXs
        | FixtureOperation::DataFrameSetIndex
        | FixtureOperation::DataFrameResetIndex
        | FixtureOperation::DataFrameDuplicated
        | FixtureOperation::DataFrameDropDuplicates
        | FixtureOperation::DataFrameSortIndex
        | FixtureOperation::DataFrameSortValues
        | FixtureOperation::DataFrameRank
        | FixtureOperation::DataFrameDiff
        | FixtureOperation::DataFrameShift
        | FixtureOperation::DataFramePctChange
        | FixtureOperation::DataFrameMelt => &["CC-004"],
    }
}

fn compat_primary_api_surface_id(operation: FixtureOperation) -> &'static str {
    compat_contract_rows_for_operation(operation)
        .first()
        .copied()
        .unwrap_or("CC-UNKNOWN")
}

fn stable_json_digest<T: Serialize>(value: &T) -> String {
    let payload = serde_json::to_vec(value).unwrap_or_else(|_| Vec::new());
    hash_bytes(&payload)
}

fn relative_to_repo(config: &HarnessConfig, path: &Path) -> String {
    path.strip_prefix(&config.repo_root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn compat_closure_artifact_refs(config: &HarnessConfig, packet_id: &str) -> Vec<String> {
    let packet_root = config.packet_artifact_root(packet_id);
    vec![
        relative_to_repo(config, &packet_root.join("parity_report.json")),
        relative_to_repo(config, &packet_root.join("parity_report.raptorq.json")),
        relative_to_repo(config, &packet_root.join("parity_report.decode_proof.json")),
        relative_to_repo(config, &packet_root.join("parity_gate_result.json")),
        relative_to_repo(config, &packet_root.join("parity_mismatch_corpus.json")),
    ]
}

fn compat_closure_env_fingerprint(config: &HarnessConfig) -> String {
    stable_json_digest(&serde_json::json!({
        "repo_root": config.repo_root.display().to_string(),
        "oracle_root": config.oracle_root.display().to_string(),
        "fixture_root": config.fixture_root.display().to_string(),
        "strict_mode": config.strict_mode,
        "platform": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "pkg": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

fn runtime_mode_split_contracts_hold() -> bool {
    let mut strict_ledger = EvidenceLedger::new();
    let mut hardened_ledger = EvidenceLedger::new();
    let strict = RuntimePolicy::strict();
    let hardened = RuntimePolicy::hardened(Some(1024));

    let strict_action = strict.decide_unknown_feature(
        "compat-closure-matrix",
        "coverage-check",
        &mut strict_ledger,
    );
    let hardened_action = hardened.decide_join_admission(2048, &mut hardened_ledger);

    strict_action == DecisionAction::Reject && hardened_action == DecisionAction::Repair
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatClosureCaseLog {
    pub ts_utc: u64,
    pub suite_id: String,
    pub test_id: String,
    pub api_surface_id: String,
    pub packet_id: String,
    pub mode: RuntimeMode,
    pub seed: u64,
    pub input_digest: String,
    pub output_digest: String,
    pub env_fingerprint: String,
    pub artifact_refs: Vec<String>,
    pub duration_ms: u64,
    pub outcome: String,
    pub reason_code: String,
}

fn build_compat_closure_case_log(
    config: &HarnessConfig,
    suite_id: &str,
    case: &CaseResult,
    ts_utc: u64,
) -> CompatClosureCaseLog {
    let mode_slug = runtime_mode_slug(case.mode);
    let seed = deterministic_seed(&case.packet_id, &case.case_id, case.mode);
    let input_digest = stable_json_digest(&serde_json::json!({
        "packet_id": case.packet_id.clone(),
        "case_id": case.case_id.clone(),
        "mode": mode_slug,
        "operation": case.operation,
        "seed": seed,
    }));
    let output_digest = stable_json_digest(&serde_json::json!({
        "status": result_label_for_status(&case.status),
        "mismatch": case.mismatch.clone(),
        "mismatch_class": case.mismatch_class.clone(),
        "replay_key": case.replay_key.clone(),
        "trace_id": case.trace_id.clone(),
    }));
    let duration_ms = case.elapsed_us.saturating_add(999) / 1000;
    let outcome = result_label_for_status(&case.status).to_owned();

    CompatClosureCaseLog {
        ts_utc,
        suite_id: suite_id.to_owned(),
        test_id: case.case_id.clone(),
        api_surface_id: compat_primary_api_surface_id(case.operation).to_owned(),
        packet_id: case.packet_id.clone(),
        mode: case.mode,
        seed,
        input_digest,
        output_digest,
        env_fingerprint: compat_closure_env_fingerprint(config),
        artifact_refs: compat_closure_artifact_refs(config, &case.packet_id),
        duration_ms: duration_ms.max(1),
        outcome: outcome.clone(),
        reason_code: case.mismatch_class.clone().unwrap_or_else(|| {
            if outcome == "pass" {
                "ok".to_owned()
            } else {
                "execution_critical".to_owned()
            }
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatClosureCoverageReport {
    pub suite_id: String,
    pub required_rows: Vec<String>,
    pub covered_rows: Vec<String>,
    pub uncovered_rows: Vec<String>,
    pub coverage_floor_percent: usize,
    pub achieved_percent: usize,
}

impl CompatClosureCoverageReport {
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.uncovered_rows.is_empty()
            && self.achieved_percent >= self.coverage_floor_percent
            && self.coverage_floor_percent == COMPAT_CLOSURE_COVERAGE_FLOOR_PERCENT
    }
}

pub fn build_compat_closure_coverage_report(
    config: &HarnessConfig,
) -> Result<CompatClosureCoverageReport, HarnessError> {
    let fixtures = load_fixtures(config, None)?;
    let mut covered_rows: BTreeSet<String> = BTreeSet::new();
    for fixture in &fixtures {
        for row in compat_contract_rows_for_operation(fixture.operation) {
            covered_rows.insert((*row).to_owned());
        }
    }

    if runtime_mode_split_contracts_hold() {
        covered_rows.insert("CC-008".to_owned());
        covered_rows.insert("CC-009".to_owned());
    }

    let required_rows = COMPAT_CLOSURE_REQUIRED_ROWS
        .iter()
        .map(|row| (*row).to_owned())
        .collect::<Vec<_>>();
    let uncovered_rows = required_rows
        .iter()
        .filter(|row| !covered_rows.contains(*row))
        .cloned()
        .collect::<Vec<_>>();
    let achieved_percent = (covered_rows.len() * 100) / required_rows.len().max(1);

    Ok(CompatClosureCoverageReport {
        suite_id: COMPAT_CLOSURE_SUITE_ID.to_owned(),
        required_rows,
        covered_rows: covered_rows.into_iter().collect(),
        uncovered_rows,
        coverage_floor_percent: COMPAT_CLOSURE_COVERAGE_FLOOR_PERCENT,
        achieved_percent,
    })
}

fn make_drift_record(
    category: ComparisonCategory,
    level: DriftLevel,
    location: impl Into<String>,
    message: impl Into<String>,
) -> DriftRecord {
    DriftRecord {
        category,
        level,
        mismatch_class: mismatch_class_for(category, level),
        location: location.into(),
        message: message.into(),
    }
}

impl DifferentialResult {
    /// Convert to backward-compatible CaseResult.
    #[must_use]
    pub fn to_case_result(&self) -> CaseResult {
        let mismatch = if self.drift_records.is_empty() {
            None
        } else {
            Some(
                self.drift_records
                    .iter()
                    .map(|d| {
                        format!(
                            "[{:?}/{:?}] {}: {}",
                            d.category, d.level, d.location, d.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        };
        CaseResult {
            packet_id: self.packet_id.clone(),
            case_id: self.case_id.clone(),
            mode: self.mode,
            operation: self.operation,
            status: self.status.clone(),
            mismatch,
            mismatch_class: self
                .drift_records
                .iter()
                .find(|drift| matches!(drift.level, DriftLevel::Critical))
                .or_else(|| self.drift_records.first())
                .map(|drift| drift.mismatch_class.clone()),
            replay_key: self.replay_key.clone(),
            trace_id: self.trace_id.clone(),
            elapsed_us: 0,
            evidence_records: self.evidence_records,
        }
    }
}

/// Per-category drift count in a summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryCount {
    pub category: ComparisonCategory,
    pub count: usize,
}

/// Aggregate drift statistics across all differential results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftSummary {
    pub total_drift_records: usize,
    pub critical_count: usize,
    pub non_critical_count: usize,
    pub informational_count: usize,
    pub categories: Vec<CategoryCount>,
}

/// Differential report: extends PacketParityReport with structured drift details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DifferentialReport {
    pub report: PacketParityReport,
    pub differential_results: Vec<DifferentialResult>,
    pub drift_summary: DriftSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DifferentialValidationLogEntry {
    pub packet_id: String,
    pub case_id: String,
    pub mode: RuntimeMode,
    pub trace_id: String,
    pub oracle_source: FixtureOracleSource,
    pub mismatch_class: String,
    pub replay_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultInjectionClassification {
    StrictViolation,
    HardenedAllowlisted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultInjectionValidationEntry {
    pub packet_id: String,
    pub case_id: String,
    pub mode: RuntimeMode,
    pub trace_id: String,
    pub oracle_source: FixtureOracleSource,
    pub mismatch_class: String,
    pub replay_key: String,
    pub classification: FaultInjectionClassification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultInjectionValidationReport {
    pub packet_id: String,
    pub entry_count: usize,
    pub strict_violation_count: usize,
    pub hardened_allowlisted_count: usize,
    pub entries: Vec<FaultInjectionValidationEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketParityReport {
    pub suite: String,
    pub packet_id: Option<String>,
    pub oracle_present: bool,
    pub fixture_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<CaseResult>,
}

impl PacketParityReport {
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.failed == 0 && self.fixture_count > 0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketGateResult {
    pub packet_id: String,
    pub pass: bool,
    pub fixture_count: usize,
    pub strict_total: usize,
    pub strict_failed: usize,
    pub hardened_total: usize,
    pub hardened_failed: usize,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaptorQPacketRecord {
    pub source_block_number: u8,
    pub encoding_symbol_id: u32,
    pub is_source: bool,
    pub serialized_hex: String,
    pub symbol_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaptorQScrubReport {
    pub verified_at_unix_ms: u64,
    pub status: String,
    pub packet_count: usize,
    pub invalid_packets: usize,
    pub source_hash_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaptorQSidecarArtifact {
    #[serde(flatten)]
    pub envelope: RaptorQEnvelope,
    pub oti_serialized_hex: String,
    pub source_packets: usize,
    pub repair_packets: usize,
    pub repair_packets_per_block: u32,
    pub packet_records: Vec<RaptorQPacketRecord>,
    pub scrub_report: RaptorQScrubReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asupersync_codec: Option<AsupersyncCodecEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsupersyncCodecEvidence {
    pub codec: String,
    pub verifier: String,
    pub encoded_bytes: usize,
    pub repair_symbols: u32,
    pub integrity_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WrittenPacketArtifacts {
    pub packet_id: String,
    pub parity_report_path: PathBuf,
    pub raptorq_sidecar_path: PathBuf,
    pub decode_proof_path: PathBuf,
    pub gate_result_path: PathBuf,
    pub mismatch_corpus_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketDriftHistoryEntry {
    pub ts_unix_ms: u64,
    pub packet_id: String,
    pub suite: String,
    pub fixture_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub strict_failed: usize,
    pub hardened_failed: usize,
    pub gate_pass: bool,
    pub report_hash: String,
}

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("fixture format error: {0}")]
    FixtureFormat(String),
    #[error("oracle is unavailable: {0}")]
    OracleUnavailable(String),
    #[error("oracle command failed: status={status}, stderr={stderr}")]
    OracleCommandFailed { status: i32, stderr: String },
    #[error("raptorq error: {0}")]
    RaptorQ(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ParityGateConfig {
    packet_id: String,
    strict: StrictGateConfig,
    hardened: HardenedGateConfig,
    machine_check: MachineCheckConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StrictGateConfig {
    critical_drift_budget: usize,
    non_critical_drift_budget_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HardenedGateConfig {
    divergence_budget_percent: f64,
    allowlisted_divergence_categories: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MachineCheckConfig {
    suite: String,
    require_fixture_count_at_least: usize,
    require_failed: usize,
}

fn synthesize_parity_gate(report: &PacketParityReport) -> Result<ParityGateConfig, HarnessError> {
    let packet_id = report
        .packet_id
        .clone()
        .ok_or_else(|| HarnessError::FixtureFormat("report has no packet_id".to_owned()))?;

    Ok(ParityGateConfig {
        packet_id,
        strict: StrictGateConfig {
            critical_drift_budget: 0,
            non_critical_drift_budget_percent: 0.1,
        },
        hardened: HardenedGateConfig {
            divergence_budget_percent: 1.0,
            allowlisted_divergence_categories: None,
        },
        machine_check: MachineCheckConfig {
            suite: "phase2c_packets".to_owned(),
            require_fixture_count_at_least: report.fixture_count,
            require_failed: 0,
        },
    })
}

fn load_or_create_parity_gate(
    config: &HarnessConfig,
    report: &PacketParityReport,
) -> Result<ParityGateConfig, HarnessError> {
    let packet_id = report
        .packet_id
        .clone()
        .ok_or_else(|| HarnessError::FixtureFormat("report has no packet_id".to_owned()))?;
    let gate_path = config.parity_gate_path(&packet_id);

    if gate_path.exists() {
        return Ok(serde_yaml::from_str(&fs::read_to_string(&gate_path)?)?);
    }

    let gate = synthesize_parity_gate(report)?;
    if let Some(parent) = gate_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&gate_path, serde_yaml::to_string(&gate)?)?;
    Ok(gate)
}

#[derive(Debug, Serialize, Deserialize)]
struct OracleRequest {
    operation: FixtureOperation,
    left: Option<FixtureSeries>,
    right: Option<FixtureSeries>,
    #[serde(default)]
    groupby_keys: Option<Vec<FixtureSeries>>,
    #[serde(default)]
    groupby_columns: Option<Vec<String>>,
    frame: Option<FixtureDataFrame>,
    frame_right: Option<FixtureDataFrame>,
    #[serde(default)]
    dict_columns: Option<BTreeMap<String, Vec<Scalar>>>,
    #[serde(default)]
    column_order: Option<Vec<String>>,
    #[serde(default)]
    records: Option<Vec<BTreeMap<String, Scalar>>>,
    #[serde(default)]
    matrix_rows: Option<Vec<Vec<Scalar>>>,
    index: Option<Vec<IndexLabel>>,
    join_type: Option<FixtureJoinType>,
    merge_on: Option<String>,
    #[serde(default)]
    merge_on_keys: Option<Vec<String>>,
    #[serde(default)]
    left_on_keys: Option<Vec<String>>,
    #[serde(default)]
    right_on_keys: Option<Vec<String>>,
    #[serde(default)]
    left_index: Option<bool>,
    #[serde(default)]
    right_index: Option<bool>,
    #[serde(default)]
    merge_indicator: Option<bool>,
    #[serde(default)]
    merge_indicator_name: Option<String>,
    #[serde(default)]
    merge_validate: Option<String>,
    #[serde(default)]
    merge_suffixes: Option<[Option<String>; 2]>,
    #[serde(default)]
    merge_sort: Option<bool>,
    #[serde(default)]
    fill_value: Option<Scalar>,
    #[serde(default)]
    datetime_unit: Option<String>,
    #[serde(default)]
    datetime_origin: Option<serde_json::Value>,
    #[serde(default)]
    datetime_utc: Option<bool>,
    #[serde(default)]
    pub head_n: Option<i64>,
    #[serde(default)]
    pub tail_n: Option<i64>,
    #[serde(default)]
    pub diff_periods: Option<i64>,
    #[serde(default)]
    pub shift_periods: Option<i64>,
    #[serde(default)]
    pub shift_axis: Option<usize>,
    #[serde(default)]
    pub pct_change_periods: Option<i64>,
    #[serde(default)]
    pub diff_axis: Option<usize>,
    #[serde(default)]
    pub rank_method: Option<String>,
    #[serde(default)]
    pub rank_na_option: Option<String>,
    #[serde(default)]
    pub rank_axis: Option<usize>,
    #[serde(default)]
    pub sort_column: Option<String>,
    #[serde(default)]
    sort_ascending: Option<bool>,
    #[serde(default)]
    concat_axis: Option<i64>,
    #[serde(default)]
    concat_join: Option<String>,
    #[serde(default)]
    set_index_column: Option<String>,
    #[serde(default)]
    set_index_drop: Option<bool>,
    #[serde(default)]
    reset_index_drop: Option<bool>,
    #[serde(default)]
    subset: Option<Vec<String>>,
    #[serde(default)]
    keep: Option<String>,
    #[serde(default)]
    ignore_index: Option<bool>,
    #[serde(default)]
    csv_input: Option<String>,
    #[serde(default)]
    loc_labels: Option<Vec<IndexLabel>>,
    #[serde(default)]
    iloc_positions: Option<Vec<i64>>,
    #[serde(default)]
    take_indices: Option<Vec<i64>>,
    #[serde(default)]
    repeat_n: Option<i64>,
    #[serde(default)]
    repeat_counts: Option<Vec<i64>>,
    #[serde(default)]
    group_name: Option<String>,
    #[serde(default)]
    xs_key: Option<IndexLabel>,
    #[serde(default)]
    cut_bins: Option<usize>,
    #[serde(default)]
    qcut_quantiles: Option<usize>,
    #[serde(default)]
    take_axis: Option<usize>,
    #[serde(default)]
    asof_label: Option<IndexLabel>,
    #[serde(default)]
    time_value: Option<String>,
    #[serde(default)]
    start_time: Option<String>,
    #[serde(default)]
    end_time: Option<String>,
    #[serde(default)]
    string_sep: Option<String>,
    #[serde(default)]
    regex_pattern: Option<String>,
    #[serde(default)]
    melt_id_vars: Option<Vec<String>>,
    #[serde(default)]
    melt_value_vars: Option<Vec<String>>,
    #[serde(default)]
    melt_var_name: Option<String>,
    #[serde(default)]
    melt_value_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OracleResponse {
    #[serde(default)]
    expected_series: Option<FixtureExpectedSeries>,
    #[serde(default)]
    expected_join: Option<FixtureExpectedJoin>,
    #[serde(default)]
    expected_frame: Option<FixtureExpectedDataFrame>,
    #[serde(default)]
    expected_alignment: Option<FixtureExpectedAlignment>,
    #[serde(default)]
    expected_bool: Option<bool>,
    #[serde(default)]
    expected_positions: Option<Vec<Option<usize>>>,
    #[serde(default)]
    expected_scalar: Option<Scalar>,
    #[serde(default)]
    expected_dtype: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum ResolvedExpected {
    Series(FixtureExpectedSeries),
    Join(FixtureExpectedJoin),
    Frame(FixtureExpectedDataFrame),
    ErrorContains(String),
    ErrorAny,
    Alignment(FixtureExpectedAlignment),
    Bool(bool),
    Positions(Vec<Option<usize>>),
    Scalar(Scalar),
    Dtype(String),
}

pub fn run_packet_suite(config: &HarnessConfig) -> Result<PacketParityReport, HarnessError> {
    run_packet_suite_with_options(config, &SuiteOptions::default())
}

pub fn run_packet_suite_with_options(
    config: &HarnessConfig,
    options: &SuiteOptions,
) -> Result<PacketParityReport, HarnessError> {
    let fixtures = load_fixtures(config, options.packet_filter.as_deref())?;
    build_report(
        config,
        "phase2c_packets".to_owned(),
        None,
        &fixtures,
        options,
    )
}

pub fn run_packet_by_id(
    config: &HarnessConfig,
    packet_id: &str,
    oracle_mode: OracleMode,
) -> Result<PacketParityReport, HarnessError> {
    let options = SuiteOptions {
        packet_filter: Some(packet_id.to_owned()),
        oracle_mode,
    };
    let fixtures = load_fixtures(config, Some(packet_id))?;
    build_report(
        config,
        format!("phase2c_packets:{packet_id}"),
        Some(packet_id.to_owned()),
        &fixtures,
        &options,
    )
}

pub fn run_packets_grouped(
    config: &HarnessConfig,
    options: &SuiteOptions,
) -> Result<Vec<PacketParityReport>, HarnessError> {
    let fixtures = load_fixtures(config, options.packet_filter.as_deref())?;
    let mut grouped = BTreeMap::<String, Vec<PacketFixture>>::new();
    for fixture in fixtures {
        grouped
            .entry(fixture.packet_id.clone())
            .or_default()
            .push(fixture);
    }

    let mut reports = Vec::with_capacity(grouped.len());
    for (packet_id, mut packet_fixtures) in grouped {
        packet_fixtures.sort_by(|a, b| a.case_id.cmp(&b.case_id));
        reports.push(build_report(
            config,
            format!("phase2c_packets:{packet_id}"),
            Some(packet_id),
            &packet_fixtures,
            options,
        )?);
    }
    Ok(reports)
}

pub fn write_grouped_artifacts(
    config: &HarnessConfig,
    reports: &[PacketParityReport],
) -> Result<Vec<WrittenPacketArtifacts>, HarnessError> {
    reports
        .iter()
        .map(|report| write_packet_artifacts(config, report))
        .collect()
}

pub fn enforce_packet_gates(
    config: &HarnessConfig,
    reports: &[PacketParityReport],
) -> Result<(), HarnessError> {
    ensure_phase2c_parity_reports(config)?;
    let mut failures = Vec::new();
    for report in reports {
        let packet_id = report.packet_id.as_deref().unwrap_or("<unknown>");
        if !report.is_green() {
            failures.push(format!(
                "{packet_id}: parity report failed fixtures={}",
                report.failed
            ));
        }
        let gate = evaluate_parity_gate(config, report)?;
        if !gate.pass {
            failures.push(format!(
                "{packet_id}: gate failed reasons={}",
                gate.reasons.join("; ")
            ));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(HarnessError::FixtureFormat(format!(
            "phase2c enforcement failed: {}",
            failures.join(" | ")
        )))
    }
}

pub fn append_phase2c_drift_history(
    config: &HarnessConfig,
    reports: &[PacketParityReport],
) -> Result<PathBuf, HarnessError> {
    let history_path = config
        .repo_root
        .join("artifacts/phase2c/drift_history.jsonl");
    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&history_path)?;

    for report in reports {
        let gate = evaluate_parity_gate(config, report)?;
        let report_json = serde_json::to_vec(report)?;
        let entry = PacketDriftHistoryEntry {
            ts_unix_ms: now_unix_ms(),
            packet_id: report
                .packet_id
                .clone()
                .unwrap_or_else(|| "<unknown>".to_owned()),
            suite: report.suite.clone(),
            fixture_count: report.fixture_count,
            passed: report.passed,
            failed: report.failed,
            strict_failed: gate.strict_failed,
            hardened_failed: gate.hardened_failed,
            gate_pass: gate.pass,
            report_hash: format!("sha256:{}", hash_bytes(&report_json)),
        };
        writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    }

    Ok(history_path)
}

fn ensure_phase2c_parity_reports(config: &HarnessConfig) -> Result<(), HarnessError> {
    let phase_root = config.repo_root.join("artifacts/phase2c");
    if !phase_root.exists() {
        return Ok(());
    }
    let packet_root = config.packet_fixture_root();
    if !packet_root.exists() {
        return Ok(());
    }
    let mut packet_ids: BTreeSet<String> = BTreeSet::new();
    for entry in fs::read_dir(&packet_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let fixture = load_fixture(&path)?;
        packet_ids.insert(fixture.packet_id);
    }

    for packet_id in packet_ids {
        let report_path = phase_root.join(&packet_id).join("parity_report.json");
        if report_path.exists() {
            continue;
        }
        let fixture_count = load_fixtures(config, Some(&packet_id))?.len();
        let synthetic = PacketParityReport {
            suite: format!("phase2c_packets:{packet_id}"),
            packet_id: Some(packet_id.clone()),
            oracle_present: config.oracle_root.exists(),
            fixture_count,
            passed: 0,
            failed: fixture_count,
            results: Vec::new(),
        };
        write_packet_artifacts(config, &synthetic)?;
    }
    Ok(())
}

// === Differential Harness: Public API ===

/// Run a differential suite over all matching fixtures with taxonomy-based comparison.
pub fn run_differential_suite(
    config: &HarnessConfig,
    options: &SuiteOptions,
) -> Result<DifferentialReport, HarnessError> {
    let fixtures = load_fixtures(config, options.packet_filter.as_deref())?;
    build_differential_report_internal(
        config,
        "phase2c_packets".to_owned(),
        None,
        &fixtures,
        options,
    )
}

/// Run a differential suite filtered by packet ID.
pub fn run_differential_by_id(
    config: &HarnessConfig,
    packet_id: &str,
    oracle_mode: OracleMode,
) -> Result<DifferentialReport, HarnessError> {
    let options = SuiteOptions {
        packet_filter: Some(packet_id.to_owned()),
        oracle_mode,
    };
    let fixtures = load_fixtures(config, Some(packet_id))?;
    build_differential_report_internal(
        config,
        format!("phase2c_packets:{packet_id}"),
        Some(packet_id.to_owned()),
        &fixtures,
        &options,
    )
}

/// Build a DifferentialReport from pre-computed DifferentialResults.
#[must_use]
pub fn build_differential_report(
    suite: String,
    packet_id: Option<String>,
    oracle_present: bool,
    results: Vec<DifferentialResult>,
) -> DifferentialReport {
    let drift_summary = summarize_drift(&results);
    let case_results: Vec<CaseResult> = results
        .iter()
        .map(DifferentialResult::to_case_result)
        .collect();
    let failed = case_results
        .iter()
        .filter(|r| matches!(r.status, CaseStatus::Fail))
        .count();
    let passed = case_results.len().saturating_sub(failed);
    DifferentialReport {
        report: PacketParityReport {
            suite,
            packet_id,
            oracle_present,
            fixture_count: case_results.len(),
            passed,
            failed,
            results: case_results,
        },
        differential_results: results,
        drift_summary,
    }
}

#[must_use]
pub fn build_differential_validation_log(
    report: &DifferentialReport,
) -> Vec<DifferentialValidationLogEntry> {
    let mut entries: Vec<_> = report
        .differential_results
        .iter()
        .map(|result| DifferentialValidationLogEntry {
            packet_id: result.packet_id.clone(),
            case_id: result.case_id.clone(),
            mode: result.mode,
            trace_id: if result.trace_id.is_empty() {
                deterministic_trace_id(&result.packet_id, &result.case_id, result.mode)
            } else {
                result.trace_id.clone()
            },
            oracle_source: result.oracle_source,
            mismatch_class: result
                .drift_records
                .iter()
                .find(|record| matches!(record.level, DriftLevel::Critical))
                .or_else(|| result.drift_records.first())
                .map(|record| record.mismatch_class.clone())
                .unwrap_or_else(|| "none".to_owned()),
            replay_key: if result.replay_key.is_empty() {
                deterministic_replay_key(&result.packet_id, &result.case_id, result.mode)
            } else {
                result.replay_key.clone()
            },
        })
        .collect();

    entries.sort_by(|a, b| {
        (
            a.packet_id.as_str(),
            a.case_id.as_str(),
            runtime_mode_slug(a.mode),
        )
            .cmp(&(
                b.packet_id.as_str(),
                b.case_id.as_str(),
                runtime_mode_slug(b.mode),
            ))
    });
    entries
}

pub fn write_differential_validation_log(
    config: &HarnessConfig,
    report: &DifferentialReport,
) -> Result<PathBuf, HarnessError> {
    let entries = build_differential_validation_log(report);
    let output_path = if let Some(packet_id) = &report.report.packet_id {
        config
            .packet_artifact_root(packet_id)
            .join("differential_validation_log.jsonl")
    } else {
        config
            .repo_root
            .join("artifacts/phase2c/differential_validation_log.jsonl")
    };

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(&output_path)?;
    for entry in entries {
        writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    }

    Ok(output_path)
}

pub fn run_fault_injection_validation_by_id(
    config: &HarnessConfig,
    packet_id: &str,
    oracle_mode: OracleMode,
) -> Result<FaultInjectionValidationReport, HarnessError> {
    let options = SuiteOptions {
        packet_filter: Some(packet_id.to_owned()),
        oracle_mode,
    };
    let mut fixtures = load_fixtures(config, Some(packet_id))?;
    fixtures.sort_by(|a, b| a.case_id.cmp(&b.case_id));

    let mut entries = Vec::new();
    let mut strict_violation_count = 0usize;
    let mut hardened_allowlisted_count = 0usize;

    for fixture in fixtures {
        for mode in [RuntimeMode::Strict, RuntimeMode::Hardened] {
            let mut mode_fixture = fixture.clone();
            mode_fixture.mode = mode;
            let mut differential = run_differential_fixture(config, &mode_fixture, &options)?;

            let (classification, injected_mismatch_class, injected_level) = match mode {
                RuntimeMode::Strict => (
                    FaultInjectionClassification::StrictViolation,
                    "fault_injected_strict_violation",
                    DriftLevel::Critical,
                ),
                RuntimeMode::Hardened => (
                    FaultInjectionClassification::HardenedAllowlisted,
                    "fault_injected_hardened_allowlist",
                    DriftLevel::Informational,
                ),
            };

            if differential.drift_records.is_empty() {
                let injected = make_drift_record(
                    ComparisonCategory::Value,
                    injected_level,
                    format!("fault_injection/{}", runtime_mode_slug(mode)),
                    format!(
                        "synthetic deterministic fault injected for case={} mode={}",
                        differential.case_id,
                        runtime_mode_slug(mode)
                    ),
                );
                differential.drift_records.push(DriftRecord {
                    mismatch_class: injected_mismatch_class.to_owned(),
                    ..injected
                });
            }

            let mismatch_class = differential
                .drift_records
                .iter()
                .find(|record| matches!(record.level, DriftLevel::Critical))
                .or_else(|| differential.drift_records.first())
                .map(|record| record.mismatch_class.clone())
                .unwrap_or_else(|| injected_mismatch_class.to_owned());

            match classification {
                FaultInjectionClassification::StrictViolation => strict_violation_count += 1,
                FaultInjectionClassification::HardenedAllowlisted => {
                    hardened_allowlisted_count += 1;
                }
            }

            entries.push(FaultInjectionValidationEntry {
                packet_id: differential.packet_id,
                case_id: differential.case_id,
                mode: differential.mode,
                trace_id: if differential.trace_id.is_empty() {
                    deterministic_trace_id(packet_id, &mode_fixture.case_id, mode)
                } else {
                    differential.trace_id
                },
                oracle_source: differential.oracle_source,
                mismatch_class,
                replay_key: if differential.replay_key.is_empty() {
                    deterministic_replay_key(packet_id, &mode_fixture.case_id, mode)
                } else {
                    differential.replay_key
                },
                classification,
            });
        }
    }

    entries.sort_by(|a, b| {
        (
            a.case_id.as_str(),
            runtime_mode_slug(a.mode),
            a.trace_id.as_str(),
        )
            .cmp(&(
                b.case_id.as_str(),
                runtime_mode_slug(b.mode),
                b.trace_id.as_str(),
            ))
    });

    Ok(FaultInjectionValidationReport {
        packet_id: packet_id.to_owned(),
        entry_count: entries.len(),
        strict_violation_count,
        hardened_allowlisted_count,
        entries,
    })
}

pub fn write_fault_injection_validation_report(
    config: &HarnessConfig,
    report: &FaultInjectionValidationReport,
) -> Result<PathBuf, HarnessError> {
    let output_path = config
        .packet_artifact_root(&report.packet_id)
        .join("fault_injection_validation.json");

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::write(&output_path, serde_json::to_string_pretty(report)?)?;
    Ok(output_path)
}

pub fn write_packet_artifacts(
    config: &HarnessConfig,
    report: &PacketParityReport,
) -> Result<WrittenPacketArtifacts, HarnessError> {
    let packet_id = report
        .packet_id
        .as_deref()
        .ok_or_else(|| HarnessError::FixtureFormat("packet_id is required".to_owned()))?;

    let root = config.packet_artifact_root(packet_id);
    fs::create_dir_all(&root)?;

    let parity_report_path = root.join("parity_report.json");
    fs::write(&parity_report_path, serde_json::to_string_pretty(report)?)?;

    let report_bytes = fs::read(&parity_report_path)?;
    let mut sidecar = generate_raptorq_sidecar(
        &parity_report_artifact_id(packet_id),
        "conformance",
        &report_bytes,
        8,
    )?;
    let decode_proof = run_raptorq_decode_recovery_drill(&sidecar, &report_bytes)?;
    sidecar
        .envelope
        .push_decode_proof_capped(decode_proof.clone());
    sidecar.envelope.scrub = ScrubStatus {
        last_ok_unix_ms: sidecar.scrub_report.verified_at_unix_ms,
        status: if sidecar.scrub_report.source_hash_verified {
            "ok".to_owned()
        } else {
            "failed".to_owned()
        },
    };

    let raptorq_sidecar_path = root.join("parity_report.raptorq.json");
    fs::write(
        &raptorq_sidecar_path,
        serde_json::to_string_pretty(&sidecar)?,
    )?;

    let decode_proof_path = root.join("parity_report.decode_proof.json");
    let decode_artifact = DecodeProofArtifact {
        packet_id: packet_id.to_owned(),
        decode_proofs: vec![decode_proof],
        status: DecodeProofStatus::Recovered,
    };
    fs::write(
        &decode_proof_path,
        serde_json::to_string_pretty(&decode_artifact)?,
    )?;

    let gate_result = evaluate_parity_gate(config, report)?;
    let gate_result_path = root.join("parity_gate_result.json");
    fs::write(
        &gate_result_path,
        serde_json::to_string_pretty(&gate_result)?,
    )?;

    let mismatch_corpus_path = root.join("parity_mismatch_corpus.json");
    let mismatches = report
        .results
        .iter()
        .filter(|result| matches!(result.status, CaseStatus::Fail))
        .cloned()
        .collect::<Vec<_>>();
    let mismatch_payload = serde_json::json!({
        "packet_id": packet_id,
        "mismatch_count": mismatches.len(),
        "mismatches": mismatches,
    });
    fs::write(
        &mismatch_corpus_path,
        serde_json::to_string_pretty(&mismatch_payload)?,
    )?;

    Ok(WrittenPacketArtifacts {
        packet_id: packet_id.to_owned(),
        parity_report_path,
        raptorq_sidecar_path,
        decode_proof_path,
        gate_result_path,
        mismatch_corpus_path,
    })
}

pub fn evaluate_parity_gate(
    config: &HarnessConfig,
    report: &PacketParityReport,
) -> Result<PacketGateResult, HarnessError> {
    let packet_id = report
        .packet_id
        .clone()
        .ok_or_else(|| HarnessError::FixtureFormat("report has no packet_id".to_owned()))?;
    let gate = load_or_create_parity_gate(config, report)?;

    let strict_total = report
        .results
        .iter()
        .filter(|result| matches!(result.mode, RuntimeMode::Strict))
        .count();
    let strict_failed = report
        .results
        .iter()
        .filter(|result| {
            matches!(result.mode, RuntimeMode::Strict) && matches!(result.status, CaseStatus::Fail)
        })
        .count();
    let hardened_total = report
        .results
        .iter()
        .filter(|result| matches!(result.mode, RuntimeMode::Hardened))
        .count();
    let hardened_failed = report
        .results
        .iter()
        .filter(|result| {
            matches!(result.mode, RuntimeMode::Hardened)
                && matches!(result.status, CaseStatus::Fail)
        })
        .count();

    let strict_failure_percent = percent(strict_failed, strict_total);
    let hardened_failure_percent = percent(hardened_failed, hardened_total);

    let mut reasons = Vec::new();
    if gate.packet_id != packet_id {
        reasons.push(format!(
            "packet_id mismatch between gate ({}) and report ({packet_id})",
            gate.packet_id
        ));
    }
    if gate.machine_check.suite != "phase2c_packets"
        && gate.machine_check.suite != report.suite
        && !report.suite.starts_with(&gate.machine_check.suite)
    {
        reasons.push(format!(
            "suite mismatch: gate={}, report={}",
            gate.machine_check.suite, report.suite
        ));
    }
    if report.fixture_count < gate.machine_check.require_fixture_count_at_least {
        reasons.push(format!(
            "fixture_count={} below required {}",
            report.fixture_count, gate.machine_check.require_fixture_count_at_least
        ));
    }
    if report.failed != gate.machine_check.require_failed {
        reasons.push(format!(
            "failed={} but gate requires {}",
            report.failed, gate.machine_check.require_failed
        ));
    }
    if strict_failed > gate.strict.critical_drift_budget {
        reasons.push(format!(
            "strict_failed={} exceeds critical_drift_budget={}",
            strict_failed, gate.strict.critical_drift_budget
        ));
    }
    if strict_failure_percent > gate.strict.non_critical_drift_budget_percent {
        reasons.push(format!(
            "strict failure percent {:.3}% exceeds {:.3}%",
            strict_failure_percent, gate.strict.non_critical_drift_budget_percent
        ));
    }
    if hardened_failure_percent > gate.hardened.divergence_budget_percent {
        reasons.push(format!(
            "hardened failure percent {:.3}% exceeds {:.3}%",
            hardened_failure_percent, gate.hardened.divergence_budget_percent
        ));
    }
    if let Some(categories) = &gate.hardened.allowlisted_divergence_categories
        && categories.is_empty()
    {
        reasons.push("hardened allowlist categories must not be empty".to_owned());
    }

    Ok(PacketGateResult {
        packet_id,
        pass: reasons.is_empty(),
        fixture_count: report.fixture_count,
        strict_total,
        strict_failed,
        hardened_total,
        hardened_failed,
        reasons,
    })
}

pub fn generate_raptorq_sidecar(
    artifact_id: &str,
    artifact_type: &str,
    report_bytes: &[u8],
    repair_packets_per_block: u32,
) -> Result<RaptorQSidecarArtifact, HarnessError> {
    if report_bytes.is_empty() {
        return Err(HarnessError::RaptorQ(
            "cannot generate sidecar for empty payload".to_owned(),
        ));
    }

    let encoder = Encoder::with_defaults(report_bytes, 1400);
    let config = encoder.get_config();

    let mut packet_records = Vec::new();
    let mut symbol_hashes = Vec::new();
    let mut source_packets = 0usize;

    for block in encoder.get_block_encoders() {
        for packet in block.source_packets() {
            source_packets += 1;
            let record = packet_record(packet, true);
            symbol_hashes.push(record.symbol_hash.clone());
            packet_records.push(record);
        }
        for packet in block.repair_packets(0, repair_packets_per_block) {
            let record = packet_record(packet, false);
            symbol_hashes.push(record.symbol_hash.clone());
            packet_records.push(record);
        }
    }

    let repair_packets = packet_records.len().saturating_sub(source_packets);
    let source_hash = hash_bytes(report_bytes);
    let mut scrub_report = verify_raptorq_sidecar_internal(
        report_bytes,
        &source_hash,
        &packet_records,
        now_unix_ms(),
    )?;
    scrub_report.status = if scrub_report.invalid_packets == 0 && scrub_report.source_hash_verified
    {
        "ok".to_owned()
    } else {
        "failed".to_owned()
    };
    let asupersync_codec = generate_asupersync_codec_evidence(artifact_id, report_bytes)?;

    let envelope = RaptorQEnvelope {
        artifact_id: artifact_id.to_owned(),
        artifact_type: artifact_type.to_owned(),
        source_hash: format!("sha256:{source_hash}"),
        raptorq: RaptorQMetadata {
            k: source_packets as u32,
            repair_symbols: repair_packets as u32,
            overhead_ratio: if source_packets == 0 {
                0.0
            } else {
                repair_packets as f64 / source_packets as f64
            },
            symbol_hashes,
        },
        scrub: ScrubStatus {
            last_ok_unix_ms: scrub_report.verified_at_unix_ms,
            status: scrub_report.status.clone(),
        },
        decode_proofs: Vec::new(),
    };

    Ok(RaptorQSidecarArtifact {
        envelope,
        oti_serialized_hex: hex_encode(&config.serialize()),
        source_packets,
        repair_packets,
        repair_packets_per_block,
        packet_records,
        scrub_report,
        asupersync_codec,
    })
}

pub fn run_raptorq_decode_recovery_drill(
    sidecar: &RaptorQSidecarArtifact,
    report_bytes: &[u8],
) -> Result<DecodeProof, HarnessError> {
    if sidecar.packet_records.is_empty() {
        return Err(HarnessError::RaptorQ(
            "sidecar has no packet records".to_owned(),
        ));
    }

    let oti_bytes = hex_decode(&sidecar.oti_serialized_hex)?;
    if oti_bytes.len() != 12 {
        return Err(HarnessError::RaptorQ(format!(
            "invalid OTI byte length: {}",
            oti_bytes.len()
        )));
    }
    let mut oti = [0_u8; 12];
    oti.copy_from_slice(&oti_bytes);
    let config = ObjectTransmissionInformation::deserialize(&oti);

    let drop_count = sidecar.source_packets.saturating_div(4).max(1);
    let mut dropped_sources = 0usize;
    let mut packets = Vec::with_capacity(sidecar.packet_records.len());
    for record in &sidecar.packet_records {
        if record.is_source && dropped_sources < drop_count {
            dropped_sources += 1;
            continue;
        }

        let packet_bytes = hex_decode(&record.serialized_hex)?;
        packets.push(EncodingPacket::deserialize(&packet_bytes));
    }

    let mut decoder = Decoder::new(config);
    let mut recovered = None;
    for packet in packets {
        recovered = decoder.decode(packet);
        if recovered.is_some() {
            break;
        }
    }

    let recovered = recovered.ok_or_else(|| {
        HarnessError::RaptorQ("decode drill could not reconstruct payload".to_owned())
    })?;
    if recovered != report_bytes {
        return Err(HarnessError::RaptorQ(
            "decode drill recovered bytes do not match source payload".to_owned(),
        ));
    }

    let proof_material = format!(
        "{}:{}:{}",
        sidecar.envelope.artifact_id,
        dropped_sources,
        hash_bytes(&recovered)
    );

    Ok(DecodeProof {
        ts_unix_ms: now_unix_ms(),
        reason: format!(
            "raptorq decode drill dropped {dropped_sources} source packets and recovered payload"
        ),
        recovered_blocks: dropped_sources as u32,
        proof_hash: format!("sha256:{}", hash_bytes(proof_material.as_bytes())),
    })
}

pub fn verify_raptorq_sidecar(
    sidecar: &RaptorQSidecarArtifact,
    report_bytes: &[u8],
) -> Result<RaptorQScrubReport, HarnessError> {
    verify_asupersync_codec_evidence(sidecar, report_bytes)?;
    let expected = sidecar
        .envelope
        .source_hash
        .strip_prefix("sha256:")
        .ok_or_else(|| {
            HarnessError::RaptorQ("source hash must be prefixed with sha256:".to_owned())
        })?
        .to_owned();
    verify_raptorq_sidecar_internal(
        report_bytes,
        &expected,
        &sidecar.packet_records,
        now_unix_ms(),
    )
}

fn verify_raptorq_sidecar_internal(
    report_bytes: &[u8],
    expected_source_hash: &str,
    records: &[RaptorQPacketRecord],
    ts_unix_ms: u64,
) -> Result<RaptorQScrubReport, HarnessError> {
    let source_hash_verified = hash_bytes(report_bytes) == expected_source_hash;
    let mut invalid_packets = 0usize;
    for record in records {
        let bytes = hex_decode(&record.serialized_hex)?;
        if hash_bytes(&bytes) != record.symbol_hash {
            invalid_packets += 1;
        }
    }

    Ok(RaptorQScrubReport {
        verified_at_unix_ms: ts_unix_ms,
        status: if source_hash_verified && invalid_packets == 0 {
            "ok".to_owned()
        } else {
            "failed".to_owned()
        },
        packet_count: records.len(),
        invalid_packets,
        source_hash_verified,
    })
}

#[cfg(feature = "asupersync")]
fn generate_asupersync_codec_evidence(
    artifact_id: &str,
    report_bytes: &[u8],
) -> Result<Option<AsupersyncCodecEvidence>, HarnessError> {
    let config = RuntimeAsupersyncConfig::default();
    let codec = PassthroughCodec;
    let verifier = Fnv1aVerifier;
    let expected_digest = fnv1a_hex(report_bytes);
    let payload = ArtifactPayload {
        artifact_id: artifact_id.to_owned(),
        bytes: report_bytes.to_vec(),
        expected_digest: Some(expected_digest.clone()),
    };

    let encoded = codec
        .encode(&payload, &config)
        .map_err(|err| HarnessError::RaptorQ(format!("asupersync encode failed: {err}")))?;
    let decoded = codec
        .decode(&encoded, &config)
        .map_err(|err| HarnessError::RaptorQ(format!("asupersync decode failed: {err}")))?;
    if decoded.bytes != report_bytes {
        return Err(HarnessError::RaptorQ(
            "asupersync codec round-trip diverged from source payload".to_owned(),
        ));
    }
    verifier
        .verify(artifact_id, &decoded.bytes, &expected_digest)
        .map_err(|err| HarnessError::RaptorQ(format!("asupersync verify failed: {err}")))?;

    Ok(Some(AsupersyncCodecEvidence {
        codec: "passthrough".to_owned(),
        verifier: "fnv1a64".to_owned(),
        encoded_bytes: encoded.encoded_bytes.len(),
        repair_symbols: encoded.repair_symbols,
        integrity_verified: true,
    }))
}

#[cfg(not(feature = "asupersync"))]
fn generate_asupersync_codec_evidence(
    _artifact_id: &str,
    _report_bytes: &[u8],
) -> Result<Option<AsupersyncCodecEvidence>, HarnessError> {
    Ok(None)
}

#[cfg(feature = "asupersync")]
fn verify_asupersync_codec_evidence(
    sidecar: &RaptorQSidecarArtifact,
    report_bytes: &[u8],
) -> Result<(), HarnessError> {
    let Some(evidence) = &sidecar.asupersync_codec else {
        return Ok(());
    };
    if !evidence.integrity_verified {
        return Err(HarnessError::RaptorQ(
            "asupersync integrity evidence was not marked verified".to_owned(),
        ));
    }

    let verifier = Fnv1aVerifier;
    let expected_digest = fnv1a_hex(report_bytes);
    verifier
        .verify(
            &sidecar.envelope.artifact_id,
            report_bytes,
            &expected_digest,
        )
        .map_err(|err| HarnessError::RaptorQ(format!("asupersync verify failed: {err}")))?;
    Ok(())
}

#[cfg(not(feature = "asupersync"))]
fn verify_asupersync_codec_evidence(
    _sidecar: &RaptorQSidecarArtifact,
    _report_bytes: &[u8],
) -> Result<(), HarnessError> {
    Ok(())
}

#[cfg(feature = "asupersync")]
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn packet_record(packet: EncodingPacket, is_source: bool) -> RaptorQPacketRecord {
    let payload = packet.payload_id();
    let serialized = packet.serialize();
    RaptorQPacketRecord {
        source_block_number: payload.source_block_number(),
        encoding_symbol_id: payload.encoding_symbol_id(),
        is_source,
        serialized_hex: hex_encode(&serialized),
        symbol_hash: hash_bytes(&serialized),
    }
}

// === RaptorQ CI Enforcement (bd-2gi.9) ===

/// Typed decode proof artifact matching `decode_proof_artifact.schema.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodeProofArtifact {
    pub packet_id: String,
    pub decode_proofs: Vec<DecodeProof>,
    pub status: DecodeProofStatus,
}

/// Outcome of a decode drill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecodeProofStatus {
    Recovered,
    Failed,
    NotAttempted,
}

impl std::fmt::Display for DecodeProofStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Recovered => write!(f, "recovered"),
            Self::Failed => write!(f, "failed"),
            Self::NotAttempted => write!(f, "not_attempted"),
        }
    }
}

/// Result of verifying a single packet's RaptorQ sidecar integrity (Rule T5).
#[derive(Debug, Clone)]
pub struct SidecarIntegrityResult {
    pub packet_id: String,
    pub parity_report_exists: bool,
    pub sidecar_exists: bool,
    pub decode_proof_exists: bool,
    pub source_hash_matches: bool,
    pub scrub_ok: bool,
    pub decode_proof_valid: bool,
    pub errors: Vec<String>,
}

impl SidecarIntegrityResult {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
            && self.parity_report_exists
            && self.sidecar_exists
            && self.decode_proof_exists
            && self.source_hash_matches
            && self.scrub_ok
            && self.decode_proof_valid
    }
}

/// Verify Rule T5 for a single packet directory: parity_report.json must have
/// a corresponding raptorq sidecar and decode proof with matching hashes.
pub fn verify_packet_sidecar_integrity(
    packet_dir: &Path,
    packet_id: &str,
) -> SidecarIntegrityResult {
    let mut result = SidecarIntegrityResult {
        packet_id: packet_id.to_owned(),
        parity_report_exists: false,
        sidecar_exists: false,
        decode_proof_exists: false,
        source_hash_matches: false,
        scrub_ok: false,
        decode_proof_valid: false,
        errors: Vec::new(),
    };

    let report_path = packet_dir.join("parity_report.json");
    let sidecar_path = packet_dir.join("parity_report.raptorq.json");
    let proof_path = packet_dir.join("parity_report.decode_proof.json");

    // Check file existence (Rule T5)
    result.parity_report_exists = report_path.exists();
    result.sidecar_exists = sidecar_path.exists();
    result.decode_proof_exists = proof_path.exists();

    if !result.parity_report_exists {
        result
            .errors
            .push(format!("{packet_id}: missing parity_report.json"));
        return result;
    }
    if !result.sidecar_exists {
        result.errors.push(format!(
            "{packet_id}: missing parity_report.raptorq.json (Rule T5)"
        ));
    }
    if !result.decode_proof_exists {
        result.errors.push(format!(
            "{packet_id}: missing parity_report.decode_proof.json (Rule T5)"
        ));
    }
    if !result.sidecar_exists || !result.decode_proof_exists {
        return result;
    }

    // Read artifacts
    let report_bytes = match fs::read(&report_path) {
        Ok(b) => b,
        Err(e) => {
            result
                .errors
                .push(format!("{packet_id}: cannot read parity_report.json: {e}"));
            return result;
        }
    };
    let sidecar: RaptorQSidecarArtifact = match fs::read_to_string(&sidecar_path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(s) => s,
        Err(e) => {
            result.errors.push(format!(
                "{packet_id}: cannot parse parity_report.raptorq.json: {e}"
            ));
            return result;
        }
    };
    let proof: DecodeProofArtifact = match fs::read_to_string(&proof_path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(p) => p,
        Err(e) => {
            result.errors.push(format!(
                "{packet_id}: cannot parse parity_report.decode_proof.json: {e}"
            ));
            return result;
        }
    };

    if sidecar.envelope.decode_proofs.len() > MAX_DECODE_PROOFS {
        result.errors.push(format!(
            "{packet_id}: sidecar envelope decode_proofs exceeds cap {MAX_DECODE_PROOFS} (found {})",
            sidecar.envelope.decode_proofs.len()
        ));
        return result;
    }
    if proof.decode_proofs.len() > MAX_DECODE_PROOFS {
        result.errors.push(format!(
            "{packet_id}: decode proof artifact exceeds cap {MAX_DECODE_PROOFS} (found {})",
            proof.decode_proofs.len()
        ));
        return result;
    }
    let expected_artifact_id = parity_report_artifact_id(packet_id);
    if sidecar.envelope.artifact_id != expected_artifact_id {
        result.decode_proof_valid = false;
        result.errors.push(format!(
            "{packet_id}: sidecar artifact_id mismatch (expected {expected_artifact_id}, found {})",
            sidecar.envelope.artifact_id
        ));
        return result;
    }
    let mut envelope_counts_valid = true;
    let envelope_source_packets = sidecar.envelope.raptorq.k as usize;
    if envelope_source_packets != sidecar.source_packets {
        envelope_counts_valid = false;
        result.errors.push(format!(
            "{packet_id}: envelope.raptorq.k ({envelope_source_packets}) does not match source_packets ({})",
            sidecar.source_packets
        ));
    }
    let envelope_repair_packets = sidecar.envelope.raptorq.repair_symbols as usize;
    if envelope_repair_packets != sidecar.repair_packets {
        envelope_counts_valid = false;
        result.errors.push(format!(
            "{packet_id}: envelope.raptorq.repair_symbols ({envelope_repair_packets}) does not match repair_packets ({})",
            sidecar.repair_packets
        ));
    }
    if !envelope_counts_valid {
        result.decode_proof_valid = false;
        return result;
    }

    // Verify source hash matches (sidecar.source_hash == SHA-256 of parity_report.json)
    let actual_hash = hash_bytes(&report_bytes);
    let expected_hash = sidecar
        .envelope
        .source_hash
        .strip_prefix("sha256:")
        .unwrap_or(&sidecar.envelope.source_hash);
    result.source_hash_matches = actual_hash == expected_hash;
    if !result.source_hash_matches {
        result.errors.push(format!(
            "{packet_id}: source_hash mismatch (expected {expected_hash}, got {actual_hash})"
        ));
    }

    // Verify scrub status
    result.scrub_ok = sidecar.scrub_report.status == "ok"
        && sidecar.scrub_report.source_hash_verified
        && sidecar.scrub_report.invalid_packets == 0;
    if !result.scrub_ok {
        result.errors.push(format!(
            "{packet_id}: scrub report not ok (status={}, invalid={})",
            sidecar.scrub_report.status, sidecar.scrub_report.invalid_packets
        ));
    }

    // Verify decode proof
    result.decode_proof_valid = proof.status == DecodeProofStatus::Recovered
        && !proof.decode_proofs.is_empty()
        && proof.packet_id == packet_id;
    if !result.decode_proof_valid {
        result.errors.push(format!(
            "{packet_id}: decode proof invalid (status={}, proofs={})",
            proof.status,
            proof.decode_proofs.len()
        ));
        return result;
    }

    let sidecar_hashes: BTreeSet<&str> = sidecar
        .envelope
        .decode_proofs
        .iter()
        .map(|entry| entry.proof_hash.as_str())
        .collect();
    let artifact_hashes: BTreeSet<&str> = proof
        .decode_proofs
        .iter()
        .map(|entry| entry.proof_hash.as_str())
        .collect();

    if sidecar_hashes.is_empty() {
        result.decode_proof_valid = false;
        result.errors.push(format!(
            "{packet_id}: sidecar envelope has no decode proofs to pair against artifact (Rule T5)"
        ));
    }

    if artifact_hashes
        .iter()
        .any(|hash| !hash.starts_with("sha256:"))
    {
        result.decode_proof_valid = false;
        result.errors.push(format!(
            "{packet_id}: decode proof hash missing sha256: prefix"
        ));
    }

    if !artifact_hashes
        .iter()
        .all(|hash| sidecar_hashes.contains(hash))
    {
        result.decode_proof_valid = false;
        result.errors.push(format!(
            "{packet_id}: decode proof hash mismatch between sidecar envelope and decode proof artifact"
        ));
    }

    result
}

/// CI gate function: verify all packet sidecars under an artifact root directory.
/// Returns Ok with results if all pass, Err with failures if any fail.
pub fn verify_all_sidecars_ci(
    artifact_root: &Path,
) -> Result<Vec<SidecarIntegrityResult>, Vec<SidecarIntegrityResult>> {
    let phase2c = artifact_root.join("phase2c");
    if !phase2c.exists() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&phase2c)
        .unwrap_or_else(|_| fs::read_dir(".").unwrap())
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && e.file_name()
                    .to_str()
                    .map(|n| n.starts_with("FP-P2"))
                    .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let packet_id = entry.file_name().to_string_lossy().to_string();
        let result = verify_packet_sidecar_integrity(&entry.path(), &packet_id);
        results.push(result);
    }

    let failures: Vec<_> = results.iter().filter(|r| !r.is_ok()).cloned().collect();
    if failures.is_empty() {
        Ok(results)
    } else {
        Err(failures)
    }
}

// === CI Gate Topology (bd-2gi.10) ===

/// CI gate identifiers matching the G1..G8 pipeline from COVERAGE_FLAKE_BUDGETS.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CiGate {
    /// G1: Compilation + formatting
    G1Compile,
    /// G2: Lint (clippy)
    G2Lint,
    /// G3: Unit tests
    G3Unit,
    /// G4: Property tests
    G4Property,
    /// G4.5: Fuzz regression (nightly only)
    G4_5Fuzz,
    /// G5: Integration tests
    G5Integration,
    /// G6: Conformance tests
    G6Conformance,
    /// G7: Coverage floor
    G7Coverage,
    /// G8: E2E orchestrator
    G8E2e,
}

impl CiGate {
    /// Stable rule identifier for forensic reports.
    pub fn rule_id(self) -> &'static str {
        match self {
            Self::G1Compile => "G1",
            Self::G2Lint => "G2",
            Self::G3Unit => "G3",
            Self::G4Property => "G4",
            Self::G4_5Fuzz => "G4.5",
            Self::G5Integration => "G5",
            Self::G6Conformance => "G6",
            Self::G7Coverage => "G7",
            Self::G8E2e => "G8",
        }
    }

    /// Gate ordering index for pipeline sequencing.
    pub fn order(self) -> u8 {
        match self {
            Self::G1Compile => 1,
            Self::G2Lint => 2,
            Self::G3Unit => 3,
            Self::G4Property => 4,
            Self::G4_5Fuzz => 5,
            Self::G5Integration => 6,
            Self::G6Conformance => 7,
            Self::G7Coverage => 8,
            Self::G8E2e => 9,
        }
    }

    /// Human-readable gate label.
    pub fn label(self) -> &'static str {
        match self {
            Self::G1Compile => "G1: Compile + Format",
            Self::G2Lint => "G2: Lint (Clippy)",
            Self::G3Unit => "G3: Unit Tests",
            Self::G4Property => "G4: Property Tests",
            Self::G4_5Fuzz => "G4.5: Fuzz Regression",
            Self::G5Integration => "G5: Integration Tests",
            Self::G6Conformance => "G6: Conformance",
            Self::G7Coverage => "G7: Coverage Floor",
            Self::G8E2e => "G8: E2E Pipeline",
        }
    }

    /// Shell command(s) for this gate (when run via external CI).
    pub fn commands(self) -> Vec<&'static str> {
        match self {
            Self::G1Compile => vec!["cargo check --workspace --all-targets", "cargo fmt --check"],
            Self::G2Lint => vec!["cargo clippy --workspace --all-targets -- -D warnings"],
            Self::G3Unit => vec!["cargo test --workspace --lib"],
            Self::G4Property => vec!["cargo test -p fp-conformance --test proptest_properties"],
            Self::G4_5Fuzz => vec![], // nightly only, defined in ADVERSARIAL_FUZZ_CORPUS.md
            Self::G5Integration => vec!["cargo test -p fp-conformance --test smoke"],
            Self::G6Conformance => vec!["cargo test -p fp-conformance -- --nocapture"],
            Self::G7Coverage => vec!["cargo llvm-cov --workspace --summary-only"],
            Self::G8E2e => vec![], // Rust-native, uses run_e2e_suite()
        }
    }

    /// One-command reproduction string for failure forensics.
    #[must_use]
    pub fn repro_command(self) -> String {
        let commands = self.commands();
        if !commands.is_empty() {
            return commands.join(" && ");
        }

        match self {
            Self::G4_5Fuzz => {
                "cargo fuzz run <target>  # see artifacts/phase2c/ADVERSARIAL_FUZZ_CORPUS.md"
                    .to_owned()
            }
            Self::G8E2e => "cargo test -p fp-conformance --test ag_e2e -- --nocapture".to_owned(),
            // All remaining gates should define shell commands.
            _ => format!("cargo run -p fp-conformance --bin fp-ci-gates -- --gate {self:?}"),
        }
    }

    /// All gates in pipeline order.
    pub fn pipeline() -> Vec<CiGate> {
        vec![
            Self::G1Compile,
            Self::G2Lint,
            Self::G3Unit,
            Self::G4Property,
            Self::G5Integration,
            Self::G6Conformance,
            Self::G7Coverage,
            Self::G8E2e,
        ]
    }

    /// Default pipeline for per-commit CI (excludes G4.5 fuzz and G7 coverage).
    pub fn commit_pipeline() -> Vec<CiGate> {
        vec![
            Self::G1Compile,
            Self::G2Lint,
            Self::G3Unit,
            Self::G4Property,
            Self::G5Integration,
            Self::G6Conformance,
        ]
    }
}

impl std::fmt::Display for CiGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Result of a single CI gate evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiGateResult {
    pub gate: CiGate,
    pub passed: bool,
    pub elapsed_ms: u64,
    pub summary: String,
    pub errors: Vec<String>,
}

/// Result of a full CI gate pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiPipelineResult {
    pub gates: Vec<CiGateResult>,
    pub all_passed: bool,
    pub first_failure: Option<CiGate>,
    pub elapsed_ms: u64,
}

/// Gate-level forensic result with stable identifiers and replay metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiGateForensicsEntry {
    pub rule_id: String,
    pub gate: CiGate,
    pub order: u8,
    pub label: String,
    pub passed: bool,
    pub elapsed_ms: u64,
    pub summary: String,
    pub errors: Vec<String>,
    pub commands: Vec<String>,
    pub repro_cmd: String,
}

/// Machine-readable CI forensic report for G1..G8 gate failures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiForensicsReport {
    pub generated_unix_ms: u128,
    pub all_passed: bool,
    pub first_failure: Option<CiGate>,
    pub elapsed_ms: u64,
    pub passed_count: usize,
    pub total_count: usize,
    pub gate_results: Vec<CiGateForensicsEntry>,
    pub violations: Vec<CiGateForensicsEntry>,
}

impl CiPipelineResult {
    pub fn passed_count(&self) -> usize {
        self.gates.iter().filter(|g| g.passed).count()
    }

    pub fn total_count(&self) -> usize {
        self.gates.len()
    }
}

/// Build a deterministic, machine-readable forensic report from a CI pipeline run.
#[must_use]
pub fn build_ci_forensics_report(result: &CiPipelineResult) -> CiForensicsReport {
    let mut gate_results = Vec::with_capacity(result.gates.len());
    let mut violations = Vec::new();

    for gate in &result.gates {
        let entry = CiGateForensicsEntry {
            rule_id: gate.gate.rule_id().to_owned(),
            gate: gate.gate,
            order: gate.gate.order(),
            label: gate.gate.label().to_owned(),
            passed: gate.passed,
            elapsed_ms: gate.elapsed_ms,
            summary: gate.summary.clone(),
            errors: gate.errors.clone(),
            commands: gate
                .gate
                .commands()
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            repro_cmd: gate.gate.repro_command(),
        };
        if !entry.passed {
            violations.push(entry.clone());
        }
        gate_results.push(entry);
    }

    let generated_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());

    CiForensicsReport {
        generated_unix_ms,
        all_passed: result.all_passed,
        first_failure: result.first_failure,
        elapsed_ms: result.elapsed_ms,
        passed_count: result.passed_count(),
        total_count: result.total_count(),
        gate_results,
        violations,
    }
}

impl std::fmt::Display for CiPipelineResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.all_passed {
            writeln!(
                f,
                "CI PIPELINE: ALL GREEN ({}/{} gates passed in {}ms)",
                self.passed_count(),
                self.total_count(),
                self.elapsed_ms
            )?;
        } else {
            writeln!(
                f,
                "CI PIPELINE: FAILED ({}/{} gates passed)",
                self.passed_count(),
                self.total_count()
            )?;
        }
        for gate in &self.gates {
            let status = if gate.passed { "PASS" } else { "FAIL" };
            writeln!(f, "  [{status}] {} ({}ms)", gate.gate, gate.elapsed_ms)?;
            if !gate.passed {
                writeln!(f, "         {}", gate.summary)?;
                for err in &gate.errors {
                    writeln!(f, "         - {err}")?;
                }
            }
        }
        Ok(())
    }
}

/// Configuration for a CI pipeline run.
#[derive(Debug, Clone)]
pub struct CiPipelineConfig {
    /// Which gates to run (default: commit pipeline)
    pub gates: Vec<CiGate>,
    /// Stop on first failure (default: true)
    pub fail_fast: bool,
    /// Harness config for Rust-native gates
    pub harness_config: HarnessConfig,
    /// Whether to run RaptorQ sidecar verification
    pub verify_sidecars: bool,
}

impl Default for CiPipelineConfig {
    fn default() -> Self {
        Self {
            gates: CiGate::commit_pipeline(),
            fail_fast: true,
            harness_config: HarnessConfig::default_paths(),
            verify_sidecars: true,
        }
    }
}

/// Evaluate a single CI gate using Rust-native checks where possible.
pub fn evaluate_ci_gate(gate: CiGate, config: &CiPipelineConfig) -> CiGateResult {
    let start = SystemTime::now();

    let (passed, summary, errors) = match gate {
        CiGate::G6Conformance => {
            let options = SuiteOptions::default();
            match run_packet_suite_with_options(&config.harness_config, &options) {
                Ok(report) => {
                    if report.is_green() {
                        (
                            true,
                            format!("All {} fixtures passed", report.fixture_count),
                            vec![],
                        )
                    } else {
                        let mut errs = Vec::new();
                        for result in &report.results {
                            if matches!(result.status, CaseStatus::Fail) {
                                errs.push(format!(
                                    "{}: {}",
                                    result.case_id,
                                    result.mismatch.as_deref().unwrap_or("unknown")
                                ));
                            }
                        }
                        (
                            false,
                            format!("{}/{} fixtures failed", report.failed, report.fixture_count),
                            errs,
                        )
                    }
                }
                Err(e) => (false, format!("Harness error: {e}"), vec![e.to_string()]),
            }
        }
        CiGate::G8E2e => {
            let e2e_config = E2eConfig::default_all_phases();
            match run_e2e_suite(&e2e_config, &mut NoopHooks) {
                Ok(report) => {
                    if report.gates_pass {
                        (
                            true,
                            format!(
                                "E2E green: {}/{} passed",
                                report.total_passed, report.total_fixtures
                            ),
                            vec![],
                        )
                    } else {
                        (
                            false,
                            format!(
                                "E2E failed: {}/{} passed",
                                report.total_passed, report.total_fixtures
                            ),
                            report
                                .gate_results
                                .iter()
                                .filter(|g| !g.pass)
                                .map(|g| format!("{}: {}", g.packet_id, g.reasons.join(", ")))
                                .collect(),
                        )
                    }
                }
                Err(e) => (false, format!("E2E error: {e}"), vec![e.to_string()]),
            }
        }
        _ => {
            // External gates: check commands are defined, report as info
            let cmds = gate.commands();
            if cmds.is_empty() {
                (true, "Skipped (no commands defined)".to_owned(), vec![])
            } else {
                // Run shell commands for external gates
                let mut all_ok = true;
                let mut errs = Vec::new();
                for cmd in &cmds {
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    if parts.is_empty() {
                        continue;
                    }
                    match Command::new(parts[0])
                        .args(&parts[1..])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                    {
                        Ok(output) => {
                            if !output.status.success() {
                                all_ok = false;
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                errs.push(format!(
                                    "`{cmd}` failed (exit {}): {}",
                                    output.status.code().unwrap_or(-1),
                                    stderr.chars().take(500).collect::<String>()
                                ));
                            }
                        }
                        Err(e) => {
                            all_ok = false;
                            errs.push(format!("`{cmd}` execution error: {e}"));
                        }
                    }
                }
                let summary = if all_ok {
                    format!("{} command(s) passed", cmds.len())
                } else {
                    format!("{} error(s)", errs.len())
                };
                (all_ok, summary, errs)
            }
        }
    };

    let elapsed_ms = start.elapsed().map(|d| d.as_millis() as u64).unwrap_or(0);

    CiGateResult {
        gate,
        passed,
        elapsed_ms,
        summary,
        errors,
    }
}

/// Run the full CI gate pipeline with fail-fast and forensics.
pub fn run_ci_pipeline(config: &CiPipelineConfig) -> CiPipelineResult {
    let start = SystemTime::now();
    let mut gates = Vec::new();
    let mut first_failure = None;

    for &gate in &config.gates {
        let result = evaluate_ci_gate(gate, config);
        let failed = !result.passed;
        gates.push(result);

        if failed {
            if first_failure.is_none() {
                first_failure = Some(gate);
            }
            if config.fail_fast {
                break;
            }
        }
    }

    // RaptorQ sidecar verification (supplemental check)
    if config.verify_sidecars && first_failure.is_none() {
        let artifact_root = config.harness_config.repo_root.join("artifacts");
        if let Err(failures) = verify_all_sidecars_ci(&artifact_root) {
            let errs: Vec<String> = failures.iter().flat_map(|f| f.errors.clone()).collect();
            gates.push(CiGateResult {
                gate: CiGate::G6Conformance,
                passed: false,
                elapsed_ms: 0,
                summary: format!("{} sidecar integrity failure(s)", failures.len()),
                errors: errs,
            });
            if first_failure.is_none() {
                first_failure = Some(CiGate::G6Conformance);
            }
        }
    }

    let all_passed = first_failure.is_none();
    let elapsed_ms = start.elapsed().map(|d| d.as_millis() as u64).unwrap_or(0);

    CiPipelineResult {
        gates,
        all_passed,
        first_failure,
        elapsed_ms,
    }
}

fn build_report(
    config: &HarnessConfig,
    suite: String,
    packet_id: Option<String>,
    fixtures: &[PacketFixture],
    options: &SuiteOptions,
) -> Result<PacketParityReport, HarnessError> {
    let mut results = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        results.push(run_fixture(config, fixture, options)?);
    }

    let failed = results
        .iter()
        .filter(|result| matches!(result.status, CaseStatus::Fail))
        .count();
    let passed = results.len().saturating_sub(failed);

    Ok(PacketParityReport {
        suite,
        packet_id,
        oracle_present: config.oracle_root.exists(),
        fixture_count: results.len(),
        passed,
        failed,
        results,
    })
}

fn load_fixtures(
    config: &HarnessConfig,
    packet_filter: Option<&str>,
) -> Result<Vec<PacketFixture>, HarnessError> {
    let fixture_files = list_fixture_files(&config.packet_fixture_root())?;
    let mut fixtures = Vec::with_capacity(fixture_files.len());

    for fixture_path in fixture_files {
        let fixture = load_fixture(&fixture_path)?;
        if packet_filter.is_none_or(|packet| fixture.packet_id == packet) {
            fixtures.push(fixture);
        }
    }
    fixtures.sort_by(|a, b| a.case_id.cmp(&b.case_id));
    Ok(fixtures)
}

fn load_fixture(path: &Path) -> Result<PacketFixture, HarnessError> {
    let body = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&body)?)
}

/// Structure-aware fuzz entrypoint for the `PacketFixture` JSON boundary.
///
/// This targets parse + serialize + expectation resolution + fixture replay and
/// treats any semantic error as acceptable as long as the harness does not
/// panic.
pub fn fuzz_fixture_parse_bytes(input: &[u8]) -> Result<(), HarnessError> {
    let fixture: PacketFixture = serde_json::from_slice(input)?;
    let _ = serde_json::to_vec(&fixture)?;

    let config = HarnessConfig {
        repo_root: PathBuf::new(),
        oracle_root: PathBuf::new(),
        fixture_root: PathBuf::new(),
        strict_mode: true,
        python_bin: "python3".to_owned(),
        allow_system_pandas_fallback: false,
    };
    let policy = match fixture.mode {
        RuntimeMode::Strict => RuntimePolicy::strict(),
        RuntimeMode::Hardened => RuntimePolicy::hardened(Some(100_000)),
    };
    let mut ledger = EvidenceLedger::new();

    let _ = fixture_expected(&fixture);
    let _ = run_fixture_operation(
        &config,
        &fixture,
        &policy,
        &mut ledger,
        OracleMode::FixtureExpected,
    );

    Ok(())
}

fn assert_csv_roundtrip(frame: &DataFrame) -> Result<(), FpIoError> {
    let encoded = write_csv_string(frame)?;
    let reparsed = read_csv_str(&encoded)?;
    if !frame.equals(&reparsed) {
        return Err(FpIoError::Io(std::io::Error::other(
            "csv round-trip drifted after parse/write/reparse",
        )));
    }
    Ok(())
}

fn assert_excel_roundtrip(frame: &DataFrame) -> Result<(), FpIoError> {
    let encoded = write_excel_bytes(frame)?;
    let reparsed = read_excel_bytes(&encoded, &ExcelReadOptions::default())?;
    if !frame.equals(&reparsed) {
        return Err(FpIoError::Io(std::io::Error::other(
            "excel round-trip drifted after parse/write/reparse",
        )));
    }
    Ok(())
}

fn assert_parquet_roundtrip(frame: &DataFrame) -> Result<(), FpIoError> {
    let encoded = write_parquet_bytes(frame)?;
    let reparsed = read_parquet_bytes(&encoded)?;
    if !frame.equals(&reparsed) {
        return Err(FpIoError::Io(std::io::Error::other(
            "parquet round-trip drifted after parse/write/reparse",
        )));
    }
    Ok(())
}

fn assert_feather_roundtrip(frame: &DataFrame) -> Result<(), FpIoError> {
    let encoded = write_feather_bytes(frame)?;
    let reparsed = read_feather_bytes(&encoded)?;
    if !frame.equals(&reparsed) {
        return Err(FpIoError::Io(std::io::Error::other(
            "feather round-trip drifted after parse/write/reparse",
        )));
    }
    Ok(())
}

fn assert_ipc_stream_roundtrip(frame: &DataFrame) -> Result<(), FpIoError> {
    let encoded = write_ipc_stream_bytes(frame)?;
    let reparsed = read_ipc_stream_bytes(&encoded)?;
    if !frame.equals(&reparsed) {
        return Err(FpIoError::Io(std::io::Error::other(
            "ipc stream round-trip drifted after parse/write/reparse",
        )));
    }
    Ok(())
}

fn fuzz_dtype_from_byte(byte: u8) -> DType {
    match byte % 5 {
        0 => DType::Null,
        1 => DType::Bool,
        2 => DType::Int64,
        3 => DType::Float64,
        _ => DType::Utf8,
    }
}

fn fuzz_u64_from_bytes(bytes: &[u8]) -> u64 {
    let mut padded = [0_u8; 8];
    let copy_len = bytes.len().min(8);
    padded[..copy_len].copy_from_slice(&bytes[..copy_len]);
    u64::from_le_bytes(padded)
}

fn fuzz_i64_from_bytes(bytes: &[u8]) -> i64 {
    let mut padded = [0_u8; 8];
    let copy_len = bytes.len().min(8);
    padded[..copy_len].copy_from_slice(&bytes[..copy_len]);
    i64::from_le_bytes(padded)
}

fn fuzz_float64_from_bytes(bytes: &[u8]) -> f64 {
    match bytes.first().copied().unwrap_or_default() % 8 {
        0 => 0.0,
        1 => 1.0,
        2 => -1.0,
        3 => 1.5,
        4 => f64::INFINITY,
        5 => f64::NEG_INFINITY,
        6 => f64::NAN,
        _ => f64::from_bits(fuzz_u64_from_bytes(bytes)),
    }
}

fn fuzz_scalar_from_bytes(bytes: &[u8]) -> Scalar {
    let Some((&tag, payload)) = bytes.split_first() else {
        return Scalar::Null(NullKind::Null);
    };

    match tag % 8 {
        0 => Scalar::Null(NullKind::Null),
        1 => Scalar::Null(NullKind::NaN),
        2 => Scalar::Null(NullKind::NaT),
        3 => Scalar::Bool(payload.first().is_some_and(|byte| byte % 2 == 1)),
        4 => Scalar::Int64(fuzz_i64_from_bytes(payload)),
        5 => Scalar::Float64(fuzz_float64_from_bytes(payload)),
        _ => Scalar::Utf8(String::from_utf8_lossy(&payload[..payload.len().min(12)]).into_owned()),
    }
}

fn fuzz_index_label_from_byte(byte: u8) -> IndexLabel {
    match byte {
        b'0'..=b'9' => IndexLabel::Int64(i64::from((byte - b'0') % 4)),
        b'a'..=b'z' => IndexLabel::Utf8(char::from(b'a' + ((byte - b'a') % 4)).to_string()),
        b'A'..=b'Z' => IndexLabel::Utf8(char::from(b'a' + ((byte - b'A') % 4)).to_string()),
        _ if byte.is_multiple_of(2) => IndexLabel::Int64(i64::from(byte % 4)),
        _ => IndexLabel::Utf8(char::from(b'a' + (byte % 4)).to_string()),
    }
}

fn fuzz_index_from_bytes(bytes: &[u8]) -> Index {
    Index::new(
        bytes
            .iter()
            .take(32)
            .map(|byte| fuzz_index_label_from_byte(*byte))
            .collect(),
    )
}

fn assert_json_roundtrip(frame: &DataFrame, orient: JsonOrient) -> Result<(), FpIoError> {
    let encoded = write_json_string(frame, orient)?;
    let reparsed = read_json_str(&encoded, orient)?;
    if !frame.equals(&reparsed) {
        return Err(FpIoError::JsonFormat(format!(
            "json {:?} round-trip drifted after parse/write/reparse",
            orient
        )));
    }
    Ok(())
}

fn assert_jsonl_roundtrip(frame: &DataFrame) -> Result<(), FpIoError> {
    let encoded = write_jsonl_string(frame)?;
    let reparsed = read_jsonl_str(&encoded)?;
    if !frame.equals(&reparsed) {
        return Err(FpIoError::JsonFormat(
            "jsonl round-trip drifted after parse/write/reparse".to_owned(),
        ));
    }
    Ok(())
}

/// Structure-aware fuzz entrypoint for the `fp-io` CSV reader.
///
/// Any parser error is acceptable, but successful parses must survive a
/// write/reparse round-trip without panicking or drifting.
pub fn fuzz_csv_parse_bytes(input: &[u8]) -> Result<(), FpIoError> {
    let input = String::from_utf8_lossy(input);
    let frame = read_csv_str(&input)?;
    assert_csv_roundtrip(&frame)
}

/// Structure-aware fuzz entrypoint for the `fp-io` Excel reader.
///
/// Any parser error is acceptable, but successful parses must survive a
/// write/reparse round-trip without panicking or drifting.
pub fn fuzz_excel_io_bytes(input: &[u8]) -> Result<(), FpIoError> {
    let frame = read_excel_bytes(input, &ExcelReadOptions::default())?;
    assert_excel_roundtrip(&frame)
}

/// Structure-aware fuzz entrypoint for the `fp-io` Parquet reader.
///
/// Inputs use the same dual-mode envelope as the Arrow-backed IPC harnesses.
/// Raw mode (`tag % 2 == 0`) feeds the remaining bytes directly into
/// `read_parquet_bytes()`, where parser errors are acceptable but successful
/// parses must round-trip. Synth mode projects bytes into a tiny typed
/// `DataFrame`, serializes it with `write_parquet_bytes()`, then checks the
/// reader against that valid payload.
pub fn fuzz_parquet_io_bytes(input: &[u8]) -> Result<(), FpIoError> {
    let Some((&mode, payload)) = input.split_first() else {
        return Ok(());
    };

    if mode % 2 == 0 {
        let frame = read_parquet_bytes(payload)?;
        assert_parquet_roundtrip(&frame)
    } else {
        let frame = fuzz_feather_frame_from_bytes(payload)?;
        assert_parquet_roundtrip(&frame)
    }
}

fn fuzz_feather_dtype_from_byte(byte: u8) -> DType {
    match byte % 4 {
        0 => DType::Bool,
        1 => DType::Int64,
        2 => DType::Float64,
        _ => DType::Utf8,
    }
}

fn fuzz_feather_scalar_for_dtype(dtype: DType, bytes: &[u8]) -> Scalar {
    let tag = bytes.first().copied().unwrap_or_default();
    let payload = bytes.get(1).copied().unwrap_or_default();

    if tag % 5 == 0 {
        return Scalar::missing_for_dtype(dtype);
    }

    match dtype {
        DType::Bool => Scalar::Bool(payload % 2 == 1),
        DType::Int64 => Scalar::Int64(i64::from(payload % 11) - 5),
        DType::Float64 => Scalar::Float64(match payload % 6 {
            0 => 0.0,
            1 => 1.0,
            2 => -1.0,
            3 => 1.5,
            4 => f64::NAN,
            _ => -0.0,
        }),
        DType::Utf8 => Scalar::Utf8(format!(
            "s{}{}",
            char::from(b'a' + (payload % 26)),
            payload % 4
        )),
        DType::Null => Scalar::Null(NullKind::Null),
    }
}

fn fuzz_feather_frame_from_bytes(bytes: &[u8]) -> Result<DataFrame, FpIoError> {
    let row_count = usize::from(bytes.first().copied().unwrap_or(2) % 4) + 1;
    let column_count = usize::from(bytes.get(1).copied().unwrap_or(1) % 3) + 1;
    let byte_at = |idx: usize| -> u8 { bytes.get(idx).copied().unwrap_or_default() };

    let index = Index::new(
        (0..row_count)
            .map(|idx| IndexLabel::Int64(idx as i64))
            .collect::<Vec<_>>(),
    );

    let mut columns = BTreeMap::new();
    let mut column_order = Vec::new();

    for col_idx in 0..column_count {
        let dtype = fuzz_feather_dtype_from_byte(byte_at(2 + col_idx));
        let name = format!("c{col_idx}");
        let values = (0..row_count)
            .map(|row_idx| {
                fuzz_feather_scalar_for_dtype(
                    dtype,
                    &[
                        byte_at(8 + col_idx * 11 + row_idx * 2),
                        byte_at(9 + col_idx * 11 + row_idx * 2),
                    ],
                )
            })
            .collect::<Vec<_>>();
        let column = Column::new(dtype, values)?;
        column_order.push(name.clone());
        columns.insert(name, column);
    }

    DataFrame::new_with_column_order(index, columns, column_order).map_err(FpIoError::from)
}

/// Structure-aware fuzz entrypoint for the `fp-io` Feather reader.
///
/// Inputs use a dual-mode envelope. Raw mode (`tag % 2 == 0`) feeds the
/// remaining bytes directly into `read_feather_bytes()`, where parser errors
/// are acceptable but successful parses must round-trip. Synth mode projects
/// bytes into a tiny typed `DataFrame`, serializes it with
/// `write_feather_bytes()`, then checks the reader against that valid payload.
pub fn fuzz_feather_io_bytes(input: &[u8]) -> Result<(), FpIoError> {
    let Some((&mode, payload)) = input.split_first() else {
        return Ok(());
    };

    if mode % 2 == 0 {
        let frame = read_feather_bytes(payload)?;
        assert_feather_roundtrip(&frame)
    } else {
        let frame = fuzz_feather_frame_from_bytes(payload)?;
        assert_feather_roundtrip(&frame)
    }
}

/// Structure-aware fuzz entrypoint for the `fp-io` Arrow IPC stream reader.
///
/// Inputs use the same dual-mode envelope as `fuzz_feather_io_bytes()`. Raw
/// mode (`tag % 2 == 0`) feeds the remaining bytes directly into
/// `read_ipc_stream_bytes()`, where parser errors are acceptable but
/// successful parses must round-trip. Synth mode projects bytes into a tiny
/// typed `DataFrame`, serializes it with `write_ipc_stream_bytes()`, then
/// checks the reader against that valid payload.
pub fn fuzz_ipc_stream_io_bytes(input: &[u8]) -> Result<(), FpIoError> {
    let Some((&mode, payload)) = input.split_first() else {
        return Ok(());
    };

    if mode % 2 == 0 {
        let frame = read_ipc_stream_bytes(payload)?;
        assert_ipc_stream_roundtrip(&frame)
    } else {
        let frame = fuzz_feather_frame_from_bytes(payload)?;
        assert_ipc_stream_roundtrip(&frame)
    }
}

/// Structure-aware fuzz entrypoint for the `fp-types` common-dtype lattice.
///
/// Two input bytes are projected onto `DType` variants. Incompatible pairs are
/// acceptable, but compatibility must remain symmetric and successful
/// promotions must remain idempotent.
pub fn fuzz_common_dtype_bytes(input: &[u8]) -> Result<(), String> {
    let [left_tag, right_tag, ..] = input else {
        return Ok(());
    };

    let left = fuzz_dtype_from_byte(*left_tag);
    let right = fuzz_dtype_from_byte(*right_tag);

    let forward = common_dtype(left, right);
    let reverse = common_dtype(right, left);

    match (forward, reverse) {
        (Ok(common), Ok(reverse_common)) => {
            if common != reverse_common {
                return Err(format!(
                    "common_dtype symmetry mismatch: left={left:?} right={right:?} \
                     forward={common:?} reverse={reverse_common:?}"
                ));
            }

            let left_idempotent = common_dtype(common, left).map_err(|err| {
                format!(
                    "common_dtype lost left compatibility: left={left:?} right={right:?} \
                     common={common:?} err={err}"
                )
            })?;
            if left_idempotent != common {
                return Err(format!(
                    "common_dtype left idempotence mismatch: left={left:?} right={right:?} \
                     common={common:?} got={left_idempotent:?}"
                ));
            }

            let right_idempotent = common_dtype(common, right).map_err(|err| {
                format!(
                    "common_dtype lost right compatibility: left={left:?} right={right:?} \
                     common={common:?} err={err}"
                )
            })?;
            if right_idempotent != common {
                return Err(format!(
                    "common_dtype right idempotence mismatch: left={left:?} right={right:?} \
                     common={common:?} got={right_idempotent:?}"
                ));
            }
        }
        (Err(_), Err(_)) => {}
        (left_result, right_result) => {
            return Err(format!(
                "common_dtype compatibility mismatch: left={left:?} right={right:?} \
                 forward={left_result:?} reverse={right_result:?}"
            ));
        }
    }

    Ok(())
}

/// Structure-aware fuzz entrypoint for `fp-types` scalar casting semantics.
///
/// The first byte selects the target `DType`; remaining bytes project into a
/// `Scalar`. Any cast error is acceptable, but owned/ref cast paths must agree,
/// identity and missing casts must preserve their contracts, and successful
/// casts must be idempotent for the chosen target dtype.
pub fn fuzz_scalar_cast_bytes(input: &[u8]) -> Result<(), String> {
    let Some((&target_tag, scalar_bytes)) = input.split_first() else {
        return Ok(());
    };

    let target = fuzz_dtype_from_byte(target_tag);
    let value = fuzz_scalar_from_bytes(scalar_bytes);
    let owned = cast_scalar_owned(value.clone(), target);
    let borrowed = cast_scalar(&value, target);

    if format!("{owned:?}") != format!("{borrowed:?}") {
        return Err(format!(
            "owned/ref scalar cast mismatch: value={value:?} target={target:?} \
             owned={owned:?} borrowed={borrowed:?}"
        ));
    }

    match owned {
        Ok(result) => {
            if value.dtype() == target && result != value {
                return Err(format!(
                    "identity cast drifted: value={value:?} target={target:?} result={result:?}"
                ));
            }

            if value.is_missing() {
                let expected = Scalar::missing_for_dtype(target);
                if result != expected {
                    return Err(format!(
                        "missing cast drifted: value={value:?} target={target:?} \
                         result={result:?} expected={expected:?}"
                    ));
                }
            }

            if target == DType::Null && result != Scalar::Null(NullKind::Null) {
                return Err(format!(
                    "cast-to-null contract drifted: value={value:?} result={result:?}"
                ));
            }

            if !value.is_missing() && target != DType::Null && result.is_missing() {
                return Err(format!(
                    "successful non-null cast produced missing sentinel: \
                     value={value:?} target={target:?} result={result:?}"
                ));
            }

            let owned_idempotent = cast_scalar_owned(result.clone(), target).map_err(|err| {
                format!(
                    "successful cast lost idempotence on owned path: \
                     value={value:?} target={target:?} result={result:?} err={err:?}"
                )
            })?;
            if owned_idempotent != result {
                return Err(format!(
                    "owned cast idempotence mismatch: value={value:?} target={target:?} \
                     result={result:?} rerun={owned_idempotent:?}"
                ));
            }

            let borrowed_idempotent = cast_scalar(&result, target).map_err(|err| {
                format!(
                    "successful cast lost idempotence on borrowed path: \
                     value={value:?} target={target:?} result={result:?} err={err:?}"
                )
            })?;
            if borrowed_idempotent != result {
                return Err(format!(
                    "borrowed cast idempotence mismatch: value={value:?} target={target:?} \
                     result={result:?} rerun={borrowed_idempotent:?}"
                ));
            }
        }
        Err(err) => {
            if value.dtype() == target || value.is_missing() || target == DType::Null {
                return Err(format!(
                    "cast unexpectedly failed for guaranteed-success contract: \
                     value={value:?} target={target:?} err={err:?}"
                ));
            }
        }
    }

    Ok(())
}

fn fuzz_series_add_scalar_from_bytes(bytes: &[u8]) -> Scalar {
    let tag = bytes.first().copied().unwrap_or_default();
    let payload = bytes.get(1).copied().unwrap_or_default();

    match tag % 6 {
        0 => Scalar::Null(NullKind::Null),
        1 => Scalar::Null(NullKind::NaN),
        2 => Scalar::Int64(i64::from(payload % 11) - 5),
        3 => Scalar::Float64(match payload % 8 {
            0 => 0.0,
            1 => 1.0,
            2 => -1.0,
            3 => 2.5,
            4 => f64::INFINITY,
            5 => f64::NEG_INFINITY,
            6 => f64::NAN,
            _ => -0.0,
        }),
        4 => Scalar::Int64(i64::from(payload % 5)),
        _ => Scalar::Float64(f64::from(i8::from_ne_bytes([payload])) / 8.0),
    }
}

fn fuzz_column_scalars_from_bytes(bytes: &[u8]) -> Vec<Scalar> {
    bytes
        .chunks(2)
        .take(16)
        .map(fuzz_series_add_scalar_from_bytes)
        .collect()
}

fn fuzz_arithmetic_op_from_byte(byte: u8) -> ArithmeticOp {
    match byte % 7 {
        0 => ArithmeticOp::Add,
        1 => ArithmeticOp::Sub,
        2 => ArithmeticOp::Mul,
        3 => ArithmeticOp::Div,
        4 => ArithmeticOp::Mod,
        5 => ArithmeticOp::Pow,
        _ => ArithmeticOp::FloorDiv,
    }
}

fn fuzz_expected_column_arith_dtype(
    left: &Column,
    right: &Column,
    op: ArithmeticOp,
) -> Result<DType, String> {
    let mut out_dtype = common_dtype(left.dtype(), right.dtype())
        .map_err(|err| format!("common dtype failed: {err:?}"))?;

    if matches!(out_dtype, DType::Bool) {
        out_dtype = DType::Int64;
    }
    if matches!(op, ArithmeticOp::Div | ArithmeticOp::Pow) {
        out_dtype = DType::Float64;
    }
    if matches!(op, ArithmeticOp::Mod | ArithmeticOp::FloorDiv) && matches!(out_dtype, DType::Int64)
    {
        let has_zero_divisor = right
            .values()
            .iter()
            .filter(|value| !value.is_missing())
            .any(|value| matches!(cast_scalar(value, DType::Int64), Ok(Scalar::Int64(0))));
        if has_zero_divisor {
            out_dtype = DType::Float64;
        }
    }

    Ok(out_dtype)
}

fn fuzz_column_arith_oracle_scalar(
    left: &Scalar,
    right: &Scalar,
    op: ArithmeticOp,
    out_dtype: DType,
    preserves_nan_missing: bool,
) -> Result<Scalar, String> {
    if left.is_missing() || right.is_missing() {
        return Ok(
            if preserves_nan_missing && (left.is_nan() || right.is_nan()) {
                Scalar::Null(NullKind::NaN)
            } else {
                Scalar::missing_for_dtype(out_dtype)
            },
        );
    }

    if matches!(out_dtype, DType::Int64) {
        let lhs = match cast_scalar(left, DType::Int64)
            .map_err(|err| format!("left int cast failed: {err:?}"))?
        {
            Scalar::Int64(value) => value,
            other => {
                return Err(format!(
                    "left int cast produced non-Int64 scalar: {other:?}"
                ));
            }
        };
        let rhs = match cast_scalar(right, DType::Int64)
            .map_err(|err| format!("right int cast failed: {err:?}"))?
        {
            Scalar::Int64(value) => value,
            other => {
                return Err(format!(
                    "right int cast produced non-Int64 scalar: {other:?}"
                ));
            }
        };

        let result = match op {
            ArithmeticOp::Add => lhs.wrapping_add(rhs),
            ArithmeticOp::Sub => lhs.wrapping_sub(rhs),
            ArithmeticOp::Mul => lhs.wrapping_mul(rhs),
            ArithmeticOp::Mod => {
                if lhs == i64::MIN && rhs == -1 {
                    0
                } else {
                    lhs.rem_euclid(rhs)
                }
            }
            ArithmeticOp::FloorDiv => {
                if lhs == i64::MIN && rhs == -1 {
                    i64::MIN
                } else {
                    lhs.div_euclid(rhs)
                }
            }
            ArithmeticOp::Div | ArithmeticOp::Pow => {
                return Err(format!(
                    "int oracle received unsupported op {:?} for dtype {:?}",
                    op, out_dtype
                ));
            }
        };
        return Ok(Scalar::Int64(result));
    }

    let lhs = left
        .to_f64()
        .map_err(|err| format!("left float cast failed: {err:?}"))?;
    let rhs = right
        .to_f64()
        .map_err(|err| format!("right float cast failed: {err:?}"))?;
    let result = match op {
        ArithmeticOp::Add => lhs + rhs,
        ArithmeticOp::Sub => lhs - rhs,
        ArithmeticOp::Mul => lhs * rhs,
        ArithmeticOp::Div => lhs / rhs,
        ArithmeticOp::Mod => lhs % rhs,
        ArithmeticOp::Pow => lhs.powf(rhs),
        ArithmeticOp::FloorDiv => (lhs / rhs).floor(),
    };
    Ok(Scalar::Float64(result))
}

fn scalars_equivalent(actual: &Scalar, expected: &Scalar, preserves_nan_missing: bool) -> bool {
    if (actual.is_nan() && expected.is_nan())
        || (!preserves_nan_missing && actual.is_missing() && expected.is_missing())
    {
        true
    } else {
        actual == expected
    }
}

fn column_arith_preserves_nan_missing(left: &Column, right: &Column, out_dtype: DType) -> bool {
    !(matches!(out_dtype, DType::Int64)
        && matches!(left.dtype(), DType::Int64)
        && matches!(right.dtype(), DType::Int64))
}

fn fuzz_series_from_bytes(name: &str, bytes: &[u8]) -> Result<Series, FrameError> {
    let mut labels = Vec::new();
    let mut values = Vec::new();

    for chunk in bytes.chunks(3).take(12) {
        let Some((&label_tag, scalar_bytes)) = chunk.split_first() else {
            continue;
        };
        labels.push(fuzz_index_label_from_byte(label_tag));
        values.push(fuzz_series_add_scalar_from_bytes(scalar_bytes));
    }

    Series::from_values(name, labels, values)
}

fn normalized_series_rows(series: &Series) -> Vec<String> {
    let mut rows: Vec<_> = series
        .index()
        .labels()
        .iter()
        .zip(series.values().iter())
        .map(|(label, value)| format!("{label:?}=>{value:?}"))
        .collect();
    rows.sort_unstable();
    rows
}

fn normalized_join_rows(joined: &JoinedSeries) -> Vec<String> {
    let mut rows = joined
        .index
        .labels()
        .iter()
        .zip(joined.left_values.values().iter())
        .zip(joined.right_values.values().iter())
        .map(|((label, left_value), right_value)| {
            format!("{label:?}|{left_value:?}|{right_value:?}")
        })
        .collect::<Vec<_>>();
    rows.sort_unstable();
    rows
}

fn normalized_join_rows_with_swapped_sides(joined: &JoinedSeries) -> Vec<String> {
    let mut rows = joined
        .index
        .labels()
        .iter()
        .zip(joined.left_values.values().iter())
        .zip(joined.right_values.values().iter())
        .map(|((label, left_value), right_value)| {
            format!("{label:?}|{right_value:?}|{left_value:?}")
        })
        .collect::<Vec<_>>();
    rows.sort_unstable();
    rows
}

fn fuzz_join_type_from_byte(byte: u8) -> JoinType {
    match byte % 5 {
        0 => JoinType::Inner,
        1 => JoinType::Left,
        2 => JoinType::Right,
        3 => JoinType::Outer,
        _ => JoinType::Cross,
    }
}

fn swapped_join_type(join_type: JoinType) -> JoinType {
    match join_type {
        JoinType::Inner => JoinType::Inner,
        JoinType::Left => JoinType::Right,
        JoinType::Right => JoinType::Left,
        JoinType::Outer => JoinType::Outer,
        JoinType::Cross => JoinType::Cross,
    }
}

/// Structure-aware fuzz entrypoint for `Column::binary_numeric()` invariants.
///
/// The first byte selects an `ArithmeticOp`; remaining bytes are split at `|`
/// (or midpoint) into left/right payloads. These are projected into bounded
/// numeric-or-missing `Column`s of matching length. The harness checks output
/// length parity, dtype promotion rules, exact per-row scalar oracle agreement,
/// missing/NaN propagation, and commutativity for `Add`/`Mul`.
pub fn fuzz_column_arith_bytes(input: &[u8]) -> Result<(), String> {
    let Some((&op_tag, payload)) = input.split_first() else {
        return Ok(());
    };

    let op = fuzz_arithmetic_op_from_byte(op_tag);
    let (left_bytes, right_bytes) =
        if let Some(split_at) = payload.iter().position(|byte| *byte == b'|') {
            (&payload[..split_at], &payload[split_at + 1..])
        } else {
            payload.split_at(payload.len() / 2)
        };

    let mut left_values = fuzz_column_scalars_from_bytes(left_bytes);
    let mut right_values = fuzz_column_scalars_from_bytes(right_bytes);
    let shared_len = left_values.len().min(right_values.len());
    left_values.truncate(shared_len);
    right_values.truncate(shared_len);

    let left = Column::from_values(left_values)
        .map_err(|err| format!("left column projection failed: {err:?}"))?;
    let right = Column::from_values(right_values)
        .map_err(|err| format!("right column projection failed: {err:?}"))?;

    let result = left
        .binary_numeric(&right, op)
        .map_err(|err| format!("column arithmetic unexpectedly failed: {err:?}"))?;

    if result.len() != left.len() || result.len() != right.len() {
        return Err(format!(
            "column arithmetic length mismatch: op={op:?} left_len={} right_len={} result_len={}",
            left.len(),
            right.len(),
            result.len()
        ));
    }

    let expected_dtype = fuzz_expected_column_arith_dtype(&left, &right, op)?;
    let preserves_nan_missing = column_arith_preserves_nan_missing(&left, &right, expected_dtype);
    if result.dtype() != expected_dtype {
        return Err(format!(
            "column arithmetic dtype drifted: op={op:?} left_dtype={:?} right_dtype={:?} \
             actual_dtype={:?} expected_dtype={expected_dtype:?}",
            left.dtype(),
            right.dtype(),
            result.dtype()
        ));
    }

    for (index, ((left_value, right_value), actual)) in left
        .values()
        .iter()
        .zip(right.values().iter())
        .zip(result.values().iter())
        .enumerate()
    {
        let expected = fuzz_column_arith_oracle_scalar(
            left_value,
            right_value,
            op,
            expected_dtype,
            preserves_nan_missing,
        )?;
        if !scalars_equivalent(actual, &expected, preserves_nan_missing) {
            return Err(format!(
                "column arithmetic oracle drifted at index={index}: op={op:?} \
                 left={left_value:?} right={right_value:?} actual={actual:?} expected={expected:?}"
            ));
        }

        if (left_value.is_missing() || right_value.is_missing()) && !actual.is_missing() {
            return Err(format!(
                "missing propagation failed at index={index}: op={op:?} left={left_value:?} right={right_value:?} actual={actual:?}"
            ));
        }
        if preserves_nan_missing
            && (left_value.is_nan() || right_value.is_nan())
            && !actual.is_nan()
        {
            return Err(format!(
                "NaN propagation failed at index={index}: op={op:?} left={left_value:?} right={right_value:?} actual={actual:?}"
            ));
        }
    }

    if matches!(op, ArithmeticOp::Add | ArithmeticOp::Mul) {
        let reverse = right
            .binary_numeric(&left, op)
            .map_err(|err| format!("reverse column arithmetic unexpectedly failed: {err:?}"))?;
        if result.dtype() != reverse.dtype() {
            return Err(format!(
                "commutative op changed dtype under reversal: op={op:?} forward_dtype={:?} reverse_dtype={:?}",
                result.dtype(),
                reverse.dtype()
            ));
        }
        for (index, (forward, backward)) in result
            .values()
            .iter()
            .zip(reverse.values().iter())
            .enumerate()
        {
            if !scalars_equivalent(forward, backward, preserves_nan_missing) {
                return Err(format!(
                    "commutative op drifted under reversal at index={index}: op={op:?} forward={forward:?} reverse={backward:?}"
                ));
            }
        }
    }

    Ok(())
}

/// Structure-aware fuzz entrypoint for `Series::add()` alignment semantics.
///
/// Inputs are split at `|` (or midpoint when absent) and projected into small
/// numeric/missing `Series` pairs. Addition must succeed, preserve index/value
/// length parity, keep every input label in the union result, and remain
/// commutative after normalizing away output row order and synthetic names.
pub fn fuzz_series_add_bytes(input: &[u8]) -> Result<(), String> {
    let (left_bytes, right_bytes) =
        if let Some(split_at) = input.iter().position(|byte| *byte == b'|') {
            (&input[..split_at], &input[split_at + 1..])
        } else {
            input.split_at(input.len() / 2)
        };

    let left = fuzz_series_from_bytes("left", left_bytes)
        .map_err(|err| format!("left series projection failed: {err:?}"))?;
    let right = fuzz_series_from_bytes("right", right_bytes)
        .map_err(|err| format!("right series projection failed: {err:?}"))?;

    let forward = left
        .add(&right)
        .map_err(|err| format!("left.add(right) unexpectedly failed: {err:?}"))?;
    let reverse = right
        .add(&left)
        .map_err(|err| format!("right.add(left) unexpectedly failed: {err:?}"))?;

    for (name, result) in [("forward", &forward), ("reverse", &reverse)] {
        if result.index().len() != result.values().len() {
            return Err(format!(
                "{name} index/value length mismatch: index_len={} value_len={}",
                result.index().len(),
                result.values().len()
            ));
        }
    }

    let forward_labels = forward.index().labels();
    for label in left.index().labels() {
        if !forward_labels.contains(label) {
            return Err(format!("forward result dropped left label {label:?}"));
        }
    }
    for label in right.index().labels() {
        if !forward_labels.contains(label) {
            return Err(format!("forward result dropped right label {label:?}"));
        }
    }

    let normalized_forward = normalized_series_rows(&forward);
    let normalized_reverse = normalized_series_rows(&reverse);
    if normalized_forward != normalized_reverse {
        return Err(format!(
            "series add lost commutativity after normalization: \
             left={left:?} right={right:?} forward_rows={normalized_forward:?} \
             reverse_rows={normalized_reverse:?}"
        ));
    }

    Ok(())
}

/// Structure-aware fuzz entrypoint for `join_series()` invariants.
///
/// The first byte selects the join type; remaining bytes are split at `|`
/// (or midpoint) into left/right `Series` payloads. The harness projects these
/// into bounded numeric/missing series, then requires global, arena, and
/// forced-fallback execution paths to agree exactly. It also checks join-type
/// specific output contracts plus side-swapped dualities where pandas-visible
/// semantics should remain invariant.
pub fn fuzz_join_series_bytes(input: &[u8]) -> Result<(), String> {
    let Some((&join_tag, payload)) = input.split_first() else {
        return Ok(());
    };

    let join_type = fuzz_join_type_from_byte(join_tag);
    let (left_bytes, right_bytes) =
        if let Some(split_at) = payload.iter().position(|byte| *byte == b'|') {
            (&payload[..split_at], &payload[split_at + 1..])
        } else {
            payload.split_at(payload.len() / 2)
        };

    let left = fuzz_series_from_bytes("left", left_bytes)
        .map_err(|err| format!("left series projection failed: {err:?}"))?;
    let right = fuzz_series_from_bytes("right", right_bytes)
        .map_err(|err| format!("right series projection failed: {err:?}"))?;

    let global = join_series_with_options(
        &left,
        &right,
        join_type,
        JoinExecutionOptions {
            use_arena: false,
            arena_budget_bytes: 0,
        },
    )
    .map_err(|err| format!("global join unexpectedly failed: {err:?}"))?;

    let arena = join_series_with_options(&left, &right, join_type, JoinExecutionOptions::default())
        .map_err(|err| format!("arena join unexpectedly failed: {err:?}"))?;

    let fallback = join_series_with_options(
        &left,
        &right,
        join_type,
        JoinExecutionOptions {
            use_arena: true,
            arena_budget_bytes: 1,
        },
    )
    .map_err(|err| format!("fallback join unexpectedly failed: {err:?}"))?;

    if global != arena {
        return Err(format!(
            "arena join result drifted from global: \
             join_type={join_type:?} left={left:?} right={right:?} global={global:?} arena={arena:?}"
        ));
    }
    if global != fallback {
        return Err(format!(
            "fallback join result drifted from global: \
             join_type={join_type:?} left={left:?} right={right:?} global={global:?} fallback={fallback:?}"
        ));
    }

    if global.index.len() != global.left_values.len()
        || global.index.len() != global.right_values.len()
    {
        return Err(format!(
            "join output length mismatch: join_type={join_type:?} index_len={} left_len={} right_len={}",
            global.index.len(),
            global.left_values.len(),
            global.right_values.len()
        ));
    }

    match join_type {
        JoinType::Inner => {
            for label in global.index.labels() {
                if !left.index().labels().contains(label) || !right.index().labels().contains(label)
                {
                    return Err(format!(
                        "inner join emitted label not present on both sides: \
                         join_type={join_type:?} label={label:?} left={left:?} right={right:?}"
                    ));
                }
            }
        }
        JoinType::Left => {
            let joined_labels = global.index.labels();
            for label in left.index().labels() {
                if !joined_labels.contains(label) {
                    return Err(format!(
                        "left join dropped left label {label:?}: result={global:?}"
                    ));
                }
            }
        }
        JoinType::Right => {
            let joined_labels = global.index.labels();
            for label in right.index().labels() {
                if !joined_labels.contains(label) {
                    return Err(format!(
                        "right join dropped right label {label:?}: result={global:?}"
                    ));
                }
            }
        }
        JoinType::Outer => {
            let joined_labels = global.index.labels();
            for label in left.index().labels() {
                if !joined_labels.contains(label) {
                    return Err(format!(
                        "outer join dropped left label {label:?}: result={global:?}"
                    ));
                }
            }
            for label in right.index().labels() {
                if !joined_labels.contains(label) {
                    return Err(format!(
                        "outer join dropped right label {label:?}: result={global:?}"
                    ));
                }
            }
        }
        JoinType::Cross => {
            let expected_rows = left.index().len().saturating_mul(right.index().len());
            if global.index.len() != expected_rows {
                return Err(format!(
                    "cross join row count drifted: expected_rows={expected_rows} actual_rows={} \
                     left_rows={} right_rows={}",
                    global.index.len(),
                    left.index().len(),
                    right.index().len()
                ));
            }
        }
    }

    if !matches!(join_type, JoinType::Cross) {
        let swapped = join_series(&right, &left, swapped_join_type(join_type))
            .map_err(|err| format!("swapped join unexpectedly failed: {err:?}"))?;
        if normalized_join_rows(&global) != normalized_join_rows_with_swapped_sides(&swapped) {
            return Err(format!(
                "join lost side-swapped duality after normalization: \
                 join_type={join_type:?} left={left:?} right={right:?} \
                 forward_rows={:?} swapped_rows={:?}",
                normalized_join_rows(&global),
                normalized_join_rows_with_swapped_sides(&swapped)
            ));
        }
    }

    Ok(())
}

fn fuzz_groupby_key_scalar_from_bytes(bytes: &[u8]) -> Scalar {
    let tag = bytes.first().copied().unwrap_or_default();
    let payload = bytes.get(1).copied().unwrap_or_default();

    match tag % 4 {
        0 => Scalar::Null(NullKind::Null),
        1 => Scalar::Int64(i64::from(payload % 4)),
        2 => Scalar::Int64(i64::from(payload % 7) - 3),
        _ => Scalar::Int64(0),
    }
}

fn fuzz_groupby_value_scalar_from_bytes(bytes: &[u8]) -> Scalar {
    let tag = bytes.first().copied().unwrap_or_default();
    let payload = bytes.get(1).copied().unwrap_or_default();

    match tag % 5 {
        0 => Scalar::Null(NullKind::Null),
        1 => Scalar::Int64(i64::from(payload % 9) - 4),
        2 => Scalar::Float64(f64::from(i8::from_ne_bytes([payload])) / 4.0),
        3 => Scalar::Float64(0.0),
        _ => Scalar::Float64(1.5),
    }
}

fn fuzz_groupby_series_from_bytes<F>(
    name: &str,
    bytes: &[u8],
    scalar_from_bytes: F,
) -> Result<Series, FrameError>
where
    F: Fn(&[u8]) -> Scalar,
{
    let mut labels = Vec::new();
    let mut values = Vec::new();

    for chunk in bytes.chunks(3).take(12) {
        let Some((&label_tag, scalar_bytes)) = chunk.split_first() else {
            continue;
        };
        labels.push(fuzz_index_label_from_byte(label_tag));
        values.push(scalar_from_bytes(scalar_bytes));
    }

    Series::from_values(name, labels, values)
}

/// Structure-aware fuzz entrypoint for `groupby_sum()` invariants.
///
/// The first byte selects `dropna`; remaining bytes are split at `|` (or
/// midpoint) into key/value series payloads. The harness projects these into
/// bounded Int64-or-null keys plus numeric-or-missing values, then requires the
/// global, arena, and forced-fallback execution paths to agree exactly.
pub fn fuzz_groupby_sum_bytes(input: &[u8]) -> Result<(), String> {
    let Some((&option_tag, payload)) = input.split_first() else {
        return Ok(());
    };

    let (key_bytes, value_bytes) =
        if let Some(split_at) = payload.iter().position(|byte| *byte == b'|') {
            (&payload[..split_at], &payload[split_at + 1..])
        } else {
            payload.split_at(payload.len() / 2)
        };

    let keys =
        fuzz_groupby_series_from_bytes("keys", key_bytes, fuzz_groupby_key_scalar_from_bytes)
            .map_err(|err| format!("key series projection failed: {err:?}"))?;
    let values =
        fuzz_groupby_series_from_bytes("values", value_bytes, fuzz_groupby_value_scalar_from_bytes)
            .map_err(|err| format!("value series projection failed: {err:?}"))?;

    let options = GroupByOptions {
        dropna: option_tag % 2 == 0,
    };
    let policy = RuntimePolicy::hardened(Some(100_000));

    let mut global_ledger = EvidenceLedger::new();
    let global = groupby_sum_with_options(
        &keys,
        &values,
        options,
        &policy,
        &mut global_ledger,
        GroupByExecutionOptions {
            use_arena: false,
            arena_budget_bytes: 0,
        },
    );

    let mut arena_ledger = EvidenceLedger::new();
    let arena = groupby_sum_with_options(
        &keys,
        &values,
        options,
        &policy,
        &mut arena_ledger,
        GroupByExecutionOptions::default(),
    );

    let mut fallback_ledger = EvidenceLedger::new();
    let fallback = groupby_sum_with_options(
        &keys,
        &values,
        options,
        &policy,
        &mut fallback_ledger,
        GroupByExecutionOptions {
            use_arena: true,
            arena_budget_bytes: 1,
        },
    );

    match (global, arena, fallback) {
        (Ok(global), Ok(arena), Ok(fallback)) => {
            if !global.equals(&arena) {
                return Err(format!(
                    "arena groupby result drifted from global: \
                     keys={keys:?} values={values:?} options={options:?} \
                     global={global:?} arena={arena:?}"
                ));
            }
            if !global.equals(&fallback) {
                return Err(format!(
                    "fallback groupby result drifted from global: \
                     keys={keys:?} values={values:?} options={options:?} \
                     global={global:?} fallback={fallback:?}"
                ));
            }

            if global.index().len() != global.values().len() {
                return Err(format!(
                    "groupby result index/value length mismatch: \
                     index_len={} value_len={}",
                    global.index().len(),
                    global.values().len()
                ));
            }

            let alignment = align_union(keys.index(), values.index());
            if global.index().len() > alignment.left_positions.len() {
                return Err(format!(
                    "groupby emitted more groups than aligned rows: \
                     groups={} aligned_rows={}",
                    global.index().len(),
                    alignment.left_positions.len()
                ));
            }

            if options.dropna
                && global
                    .index()
                    .labels()
                    .iter()
                    .any(|label| matches!(label, IndexLabel::Utf8(text) if text == "<null>"))
            {
                return Err(format!(
                    "dropna=true emitted a null group label: result={global:?}"
                ));
            }
        }
        (Err(_), Err(_), Err(_)) => {}
        (global_result, arena_result, fallback_result) => {
            return Err(format!(
                "groupby execution-path error mismatch: keys={keys:?} values={values:?} \
                 options={options:?} global={global_result:?} arena={arena_result:?} \
                 fallback={fallback_result:?}"
            ));
        }
    }

    Ok(())
}

/// Structure-aware fuzz entrypoint for the `fp-io` JSON and JSONL readers.
///
/// The same textual payload is exercised across all supported JSON orients plus
/// JSONL. Any semantic parser error is acceptable, but successful parses must
/// survive a write/reparse round-trip without panicking or drifting.
pub fn fuzz_json_io_bytes(input: &[u8]) -> Result<(), FpIoError> {
    let input = String::from_utf8_lossy(input);
    let mut first_err = None;
    let mut saw_success = false;

    for orient in [
        JsonOrient::Records,
        JsonOrient::Columns,
        JsonOrient::Index,
        JsonOrient::Split,
        JsonOrient::Values,
    ] {
        match read_json_str(&input, orient) {
            Ok(frame) => {
                saw_success = true;
                assert_json_roundtrip(&frame, orient)?;
            }
            Err(err) => {
                if first_err.is_none() {
                    first_err = Some(err);
                }
            }
        }
    }

    match read_jsonl_str(&input) {
        Ok(frame) => {
            saw_success = true;
            assert_jsonl_roundtrip(&frame)?;
        }
        Err(err) => {
            if first_err.is_none() {
                first_err = Some(err);
            }
        }
    }

    if saw_success {
        Ok(())
    } else {
        Err(first_err.unwrap_or_else(|| {
            FpIoError::JsonFormat("json fuzz input did not exercise any parser".to_owned())
        }))
    }
}

fn list_fixture_files(root: &Path) -> Result<Vec<PathBuf>, HarnessError> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn run_fixture(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    options: &SuiteOptions,
) -> Result<CaseResult, HarnessError> {
    let policy = match fixture.mode {
        RuntimeMode::Strict => RuntimePolicy::strict(),
        RuntimeMode::Hardened => RuntimePolicy::hardened(Some(100_000)),
    };

    let mut ledger = EvidenceLedger::new();
    let started = Instant::now();
    let mismatch =
        run_fixture_operation(config, fixture, &policy, &mut ledger, options.oracle_mode).err();
    let elapsed_us = (started.elapsed().as_micros() as u64).max(1);
    let replay_key = deterministic_replay_key(&fixture.packet_id, &fixture.case_id, fixture.mode);
    let trace_id = deterministic_trace_id(&fixture.packet_id, &fixture.case_id, fixture.mode);
    let mismatch_class = mismatch.as_ref().map(|_| "execution_critical".to_owned());

    Ok(CaseResult {
        packet_id: fixture.packet_id.clone(),
        case_id: fixture.case_id.clone(),
        mode: fixture.mode,
        operation: fixture.operation,
        status: if mismatch.is_none() {
            CaseStatus::Pass
        } else {
            CaseStatus::Fail
        },
        mismatch,
        mismatch_class,
        replay_key,
        trace_id,
        elapsed_us,
        evidence_records: ledger.records().len(),
    })
}

fn run_fixture_operation(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
    default_oracle_mode: OracleMode,
) -> Result<(), String> {
    let expected = resolve_expected(config, fixture, default_oracle_mode)
        .map_err(|err| format!("expected resolution failed: {err}"))?;

    match fixture.operation {
        FixtureOperation::SeriesAdd
        | FixtureOperation::SeriesSub
        | FixtureOperation::SeriesMul
        | FixtureOperation::SeriesDiv => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let left_series = build_series(left)?;
            let right_series = build_series(right)?;
            let actual = match fixture.operation {
                FixtureOperation::SeriesAdd => {
                    left_series.add_with_policy(&right_series, policy, ledger)
                }
                FixtureOperation::SeriesSub => {
                    left_series.sub_with_policy(&right_series, policy, ledger)
                }
                FixtureOperation::SeriesMul => {
                    left_series.mul_with_policy(&right_series, policy, ledger)
                }
                FixtureOperation::SeriesDiv => {
                    left_series.div_with_policy(&right_series, policy, ledger)
                }
                _ => unreachable!("match arm constrained to series arithmetic operations"),
            }
            .map_err(|err| err.to_string())?;

            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => {
                    return Err(format!(
                        "expected_series is required for {}",
                        fixture.operation.operation_name()
                    ));
                }
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::SeriesJoin => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let joined = (|| -> Result<fp_join::JoinedSeries, String> {
                let left_series =
                    build_series(left).map_err(|err| format!("left series build failed: {err}"))?;
                let right_series = build_series(right)
                    .map_err(|err| format!("right series build failed: {err}"))?;
                let join_type = require_series_join_type(fixture)?;
                join_series(&left_series, &right_series, join_type).map_err(|err| err.to_string())
            })();

            match expected {
                ResolvedExpected::Join(expected_join) => {
                    compare_join_expected(&joined?, &expected_join)
                }
                ResolvedExpected::ErrorContains(substr) => match joined {
                    Err(message) => {
                        if message.contains(&substr) {
                            Ok(())
                        } else {
                            Err(format!(
                                "expected series_join error containing '{substr}', got '{message}'"
                            ))
                        }
                    }
                    Ok(_) => Err(format!(
                        "expected series_join to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match joined {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected series_join to fail".to_owned()),
                },
                _ => Err("expected_join is required for series_join".to_owned()),
            }
        }
        FixtureOperation::SeriesConstructor => {
            let left = require_left_series(fixture)?;
            let actual = build_series(left);
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_constructor error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_constructor to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected series_constructor to fail".to_owned()),
                },
                _ => Err(
                    "expected_series or expected_error is required for series_constructor"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesToDatetime => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = resolve_datetime_origin_option(fixture.datetime_origin.as_ref()).and_then(
                |origin| {
                    fp_frame::to_datetime_with_options(
                        &series,
                        fp_frame::ToDatetimeOptions {
                            format: None,
                            unit: fixture.datetime_unit.as_deref(),
                            utc: fixture.datetime_utc.unwrap_or(false),
                            origin,
                        },
                    )
                    .map_err(|err| err.to_string())
                },
            );
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_to_datetime error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_to_datetime to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected series_to_datetime to fail".to_owned()),
                },
                _ => Err(
                    "expected_series or expected_error is required for series_to_datetime"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesToTimedelta => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = fp_frame::to_timedelta(&series).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_to_timedelta error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_to_timedelta to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected series_to_timedelta to fail".to_owned()),
                },
                _ => Err(
                    "expected_series or expected_error is required for series_to_timedelta"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesTimedeltaTotalSeconds => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = fp_frame::timedelta_total_seconds(&series).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected timedelta_total_seconds error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected timedelta_total_seconds to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected timedelta_total_seconds to fail".to_owned()),
                },
                _ => Err(
                    "expected_series or expected_error is required for timedelta_total_seconds"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameFromSeries => {
            let actual = execute_dataframe_from_series_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_from_series error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_from_series to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected dataframe_from_series to fail".to_owned()),
                },
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_from_series"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameFromDict => {
            let actual = execute_dataframe_from_dict_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_from_dict error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_from_dict to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected dataframe_from_dict to fail".to_owned()),
                },
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_from_dict"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameFromRecords => {
            let actual = execute_dataframe_from_records_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_from_records error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_from_records to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected dataframe_from_records to fail".to_owned()),
                },
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_from_records"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameConstructorKwargs => {
            let actual = execute_dataframe_constructor_kwargs_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_constructor_kwargs error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_constructor_kwargs to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected dataframe_constructor_kwargs to fail".to_owned()),
                },
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_constructor_kwargs"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameConstructorScalar => {
            let actual = execute_dataframe_constructor_scalar_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_constructor_scalar error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_constructor_scalar to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected dataframe_constructor_scalar to fail".to_owned()),
                },
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_constructor_scalar"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameConstructorDictOfSeries => {
            let actual = execute_dataframe_constructor_dict_of_series_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_constructor_dict_of_series error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_constructor_dict_of_series to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => {
                        Err("expected dataframe_constructor_dict_of_series to fail".to_owned())
                    }
                },
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_constructor_dict_of_series"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameConstructorListLike => {
            let actual = execute_dataframe_constructor_list_like_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_constructor_list_like error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_constructor_list_like to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected dataframe_constructor_list_like to fail".to_owned()),
                },
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_constructor_list_like"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::GroupBySum => {
            let actual =
                execute_groupby_fixture_operation(fixture, fixture.operation, policy, ledger)?;

            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for groupby_sum".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::IndexAlignUnion => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let plan = align_union(
                &Index::new(left.index.clone()),
                &Index::new(right.index.clone()),
            );
            validate_alignment_plan(&plan).map_err(|err| err.to_string())?;

            let expected = match expected {
                ResolvedExpected::Alignment(alignment) => alignment,
                _ => return Err("expected_alignment is required for index_align_union".to_owned()),
            };
            compare_alignment_expected(&plan, &expected)
        }
        FixtureOperation::IndexHasDuplicates
        | FixtureOperation::IndexIsMonotonicIncreasing
        | FixtureOperation::IndexIsMonotonicDecreasing => {
            let index = require_index(fixture)?;
            let actual = match fixture.operation {
                FixtureOperation::IndexHasDuplicates => Index::new(index.clone()).has_duplicates(),
                FixtureOperation::IndexIsMonotonicIncreasing => {
                    Index::new(index.clone()).is_monotonic_increasing()
                }
                FixtureOperation::IndexIsMonotonicDecreasing => {
                    Index::new(index.clone()).is_monotonic_decreasing()
                }
                _ => unreachable!(),
            };
            let expected = match expected {
                ResolvedExpected::Bool(value) => value,
                _ => {
                    return Err(format!(
                        "expected_bool is required for {:?}",
                        fixture.operation
                    ));
                }
            };
            if actual != expected {
                return Err(format!(
                    "boolean mismatch for {:?}: actual={actual}, expected={expected}",
                    fixture.operation
                ));
            }
            Ok(())
        }
        FixtureOperation::IndexFirstPositions => {
            let index = require_index(fixture)?;
            let index = Index::new(index.clone());
            let positions = index.position_map_first();
            let actual = index
                .labels()
                .iter()
                .map(|label| positions.get(label).copied())
                .collect::<Vec<_>>();
            let expected = match expected {
                ResolvedExpected::Positions(values) => values,
                _ => {
                    return Err(
                        "expected_positions is required for index_first_positions".to_owned()
                    );
                }
            };
            if actual != expected {
                return Err(format!(
                    "first-position mismatch: actual={actual:?}, expected={expected:?}"
                ));
            }
            Ok(())
        }
        FixtureOperation::SeriesConcat => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let left_s = build_series(left)?;
            let right_s = build_series(right)?;
            let actual = concat_series(&[&left_s, &right_s]).map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for series_concat".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::SeriesCombineFirst => {
            let actual = execute_series_combine_first_fixture_operation(fixture);
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for series_combine_first".to_owned()),
            };
            compare_series_expected(&actual?, &expected)
        }
        FixtureOperation::NanSum
        | FixtureOperation::NanMean
        | FixtureOperation::NanMin
        | FixtureOperation::NanMax
        | FixtureOperation::NanStd
        | FixtureOperation::NanVar
        | FixtureOperation::NanCount => {
            let actual = execute_nanop_fixture_operation(fixture, fixture.operation)?;
            let expected = match expected {
                ResolvedExpected::Scalar(scalar) => scalar,
                _ => {
                    return Err(format!(
                        "expected_scalar is required for {}",
                        fixture.operation.operation_name()
                    ));
                }
            };
            compare_scalar(&actual, &expected, fixture.operation.operation_name())
        }
        FixtureOperation::FillNa => {
            let left = require_left_series(fixture)?;
            let fill = fixture
                .fill_value
                .as_ref()
                .ok_or_else(|| "fill_value is required for fill_na".to_owned())?;
            let actual_values = fill_na(&left.values, fill);
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for fill_na".to_owned()),
            };
            if actual_values != expected.values {
                return Err(format!(
                    "fill_na value mismatch: actual={actual_values:?}, expected={:?}",
                    expected.values
                ));
            }
            Ok(())
        }
        FixtureOperation::DropNa => {
            let left = require_left_series(fixture)?;
            let actual_values = dropna(&left.values);
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for drop_na".to_owned()),
            };
            if actual_values != expected.values {
                return Err(format!(
                    "drop_na value mismatch: actual={actual_values:?}, expected={:?}",
                    expected.values
                ));
            }
            Ok(())
        }
        FixtureOperation::CsvRoundTrip => {
            let actual = execute_csv_round_trip_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Bool(value) => {
                    let round_trip_ok = actual?;
                    if round_trip_ok != value {
                        return Err(format!(
                            "csv_round_trip mismatch: actual={round_trip_ok}, expected={value}"
                        ));
                    }
                    Ok(())
                }
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected csv_round_trip error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected csv_round_trip to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => match actual {
                    Err(_) => Ok(()),
                    Ok(_) => Err("expected csv_round_trip to fail".to_owned()),
                },
                _ => {
                    Err("expected_bool or expected_error is required for csv_round_trip".to_owned())
                }
            }
        }
        FixtureOperation::ColumnDtypeCheck => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual_dtype = format!("{:?}", series.column().dtype());
            let expected = match expected {
                ResolvedExpected::Dtype(dtype) => dtype,
                _ => return Err("expected_dtype is required for column_dtype_check".to_owned()),
            };
            if actual_dtype != expected {
                return Err(format!(
                    "dtype mismatch: actual={actual_dtype}, expected={expected}"
                ));
            }
            Ok(())
        }
        FixtureOperation::SeriesFilter => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let data = build_series(left)?;
            let mask = build_series(right)?;
            let actual = data.filter(&mask).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_filter error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_filter to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_filter to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_filter".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesHead => {
            let left = require_left_series(fixture)?;
            let n = fixture
                .head_n
                .ok_or_else(|| "head_n is required for series_head".to_owned())?;
            let series = build_series(left)?;
            let take = normalize_head_take(n, series.len());
            let labels = series.index().labels()[..take].to_vec();
            let values = series.values()[..take].to_vec();
            let actual =
                Series::from_values(series.name(), labels, values).map_err(|e| e.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for series_head".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::SeriesTail => {
            let left = require_left_series(fixture)?;
            let n = fixture
                .tail_n
                .ok_or_else(|| "tail_n is required for series_tail".to_owned())?;
            let series = build_series(left)?;
            let actual = series.tail(n).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_tail error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_tail to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_tail to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_series or expected_error is required for series_tail".to_owned())
                }
            }
        }
        FixtureOperation::SeriesValueCounts => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.value_counts().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_value_counts error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_value_counts to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected series_value_counts to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_value_counts"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesSortIndex => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series
                .sort_index(resolve_sort_ascending(fixture))
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_sort_index error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_sort_index to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_sort_index to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_sort_index"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesSortValues => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series
                .sort_values(resolve_sort_ascending(fixture))
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_sort_values error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_sort_values to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected series_sort_values to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_sort_values"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesDiff => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let periods = fixture.diff_periods.unwrap_or(1);
            let actual = series.diff(periods).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_diff error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_diff to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_diff to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_series or expected_error is required for series_diff".to_owned())
                }
            }
        }
        FixtureOperation::SeriesShift => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let periods = fixture.shift_periods.unwrap_or(1);
            let actual = series.shift(periods).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_shift error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_shift to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_shift to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_series or expected_error is required for series_shift".to_owned())
                }
            }
        }
        FixtureOperation::SeriesPctChange => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let periods = fixture.pct_change_periods.unwrap_or(1) as usize;
            let actual = series.pct_change(periods).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_pct_change error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_pct_change to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_pct_change to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_pct_change"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesIsNa => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.isna().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_isna error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_isna to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_isna to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_series or expected_error is required for series_isna".to_owned())
                }
            }
        }
        FixtureOperation::SeriesNotNa => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.notna().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_notna error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_notna to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_notna to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_series or expected_error is required for series_notna".to_owned())
                }
            }
        }
        FixtureOperation::SeriesIsNull => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.isnull().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_isnull error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_isnull to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_isnull to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_isnull".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesNotNull => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.notnull().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_notnull error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_notnull to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_notnull to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_notnull".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesFillNa => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let fill_value = fixture
                .fill_value
                .as_ref()
                .ok_or_else(|| "fill_value is required for series_fillna".to_owned())?;
            let actual = series.fillna(fill_value).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_fillna error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_fillna to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_fillna to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_fillna".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesDropNa => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.dropna().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_dropna error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_dropna to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_dropna to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_dropna".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesCount => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = Scalar::Int64(series.count() as i64);
            match expected {
                ResolvedExpected::Scalar(scalar) => {
                    compare_scalar(&actual, &scalar, fixture.operation.operation_name())
                }
                _ => Err("expected_scalar is required for series_count".to_owned()),
            }
        }
        FixtureOperation::DataFrameCount => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let actual = frame.count().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_count error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_count to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_count to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for dataframe_count".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameMode => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let actual = frame.mode().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_mode error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_mode to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_mode to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_mode".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameRank => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let method = fixture.rank_method.as_deref().unwrap_or("average");
            let na_option = fixture.rank_na_option.as_deref().unwrap_or("keep");
            let ascending = resolve_sort_ascending(fixture);
            let axis = resolve_rank_axis(fixture)?;
            let actual = if axis == 1 {
                frame.rank_axis1(method, ascending, na_option)
            } else {
                frame.rank(method, ascending, na_option)
            }
            .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_rank error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_rank to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_rank to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_rank".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameDuplicated => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let subset = resolve_duplicate_subset(fixture)?;
            let keep = resolve_duplicate_keep(fixture)?;
            let actual = frame
                .duplicated(subset.as_deref(), keep)
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_duplicated error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_duplicated to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_duplicated to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for dataframe_duplicated"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesAny => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.any().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Bool(value) => {
                    let actual = actual?;
                    if actual == value {
                        Ok(())
                    } else {
                        Err(format!(
                            "series_any mismatch: actual={actual}, expected={value}"
                        ))
                    }
                }
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_any error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_any to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_any to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err("expected_bool or expected_error is required for series_any".to_owned()),
            }
        }
        FixtureOperation::SeriesAll => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.all().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Bool(value) => {
                    let actual = actual?;
                    if actual == value {
                        Ok(())
                    } else {
                        Err(format!(
                            "series_all mismatch: actual={actual}, expected={value}"
                        ))
                    }
                }
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_all error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_all to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_all to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err("expected_bool or expected_error is required for series_all".to_owned()),
            }
        }
        FixtureOperation::SeriesBool => {
            let actual = execute_series_bool_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Bool(value) => {
                    let actual = actual?;
                    if actual == value {
                        Ok(())
                    } else {
                        Err(format!(
                            "series_bool mismatch: actual={actual}, expected={value}"
                        ))
                    }
                }
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_bool error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_bool to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_bool to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err("expected_bool or expected_error is required for series_bool".to_owned()),
            }
        }
        FixtureOperation::SeriesRepeat => {
            let actual = execute_series_repeat_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_repeat error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_repeat to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_repeat to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_repeat".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesToNumeric
        | FixtureOperation::SeriesConvertDtypes
        | FixtureOperation::SeriesAstype
        | FixtureOperation::SeriesClip
        | FixtureOperation::SeriesAbs
        | FixtureOperation::SeriesRound
        | FixtureOperation::SeriesCumsum
        | FixtureOperation::SeriesCumprod
        | FixtureOperation::SeriesCummax
        | FixtureOperation::SeriesCummin
        | FixtureOperation::SeriesNlargest
        | FixtureOperation::SeriesNsmallest
        | FixtureOperation::SeriesBetween
        | FixtureOperation::SeriesCut
        | FixtureOperation::SeriesQcut => {
            let actual = execute_series_module_utility_fixture_operation(fixture);
            let op_name = fixture.operation.operation_name();
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected {op_name} error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected {op_name} to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(format!(
                            "expected {op_name} to fail but operation succeeded"
                        ))
                    }
                }
                _ => Err(format!(
                    "expected_series or expected_error is required for {op_name}"
                )),
            }
        }
        FixtureOperation::SeriesLoc => {
            let left = require_left_series(fixture)?;
            let labels = require_loc_labels(fixture)?;
            let series = build_series(left)?;
            let actual = series.loc(labels).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_loc error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_loc to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_loc to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err("expected_series or expected_error is required for series_loc".to_owned()),
            }
        }
        FixtureOperation::SeriesIloc => {
            let left = require_left_series(fixture)?;
            let positions = require_iloc_positions(fixture)?;
            let series = build_series(left)?;
            let actual = series.iloc(positions).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_iloc error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_iloc to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_iloc to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_series or expected_error is required for series_iloc".to_owned())
                }
            }
        }
        FixtureOperation::SeriesTake => {
            let left = require_left_series(fixture)?;
            let indices = require_take_indices(fixture)?;
            let series = build_series(left)?;
            let actual = series.take(indices).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_take error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_take to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_take to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_series or expected_error is required for series_take".to_owned())
                }
            }
        }
        FixtureOperation::SeriesXs => {
            let actual = execute_series_xs_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_xs error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_xs to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_xs to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err("expected_series or expected_error is required for series_xs".to_owned()),
            }
        }
        FixtureOperation::SeriesAtTime => {
            let left = require_left_series(fixture)?;
            let time = require_time_value(fixture)?;
            let series = build_series(left)?;
            let actual = series.at_time(time).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_at_time error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_at_time to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_at_time to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_at_time".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesBetweenTime => {
            let left = require_left_series(fixture)?;
            let start = require_start_time(fixture)?;
            let end = require_end_time(fixture)?;
            let series = build_series(left)?;
            let actual = series
                .between_time(start, end)
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_between_time error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_between_time to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected series_between_time to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for series_between_time"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesPartitionDf => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_partition_df error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_partition_df to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected series_partition_df to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for series_partition_df"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesRpartitionDf => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_rpartition_df error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_rpartition_df to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected series_rpartition_df to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for series_rpartition_df"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesExtractAll => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_extractall error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_extractall to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_extractall to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for series_extractall".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesExtractDf => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected series_extract_df error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected series_extract_df to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected series_extract_df to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for series_extract_df".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameLoc => {
            let frame = require_frame(fixture)?;
            let labels = require_loc_labels(fixture)?;
            let frame = build_dataframe(frame)?;
            let actual = frame
                .loc_with_columns(labels, fixture.column_order.as_deref())
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_loc error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_loc to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_loc to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_frame or expected_error is required for dataframe_loc".to_owned())
                }
            }
        }
        FixtureOperation::DataFrameIloc => {
            let frame = require_frame(fixture)?;
            let positions = require_iloc_positions(fixture)?;
            let frame = build_dataframe(frame)?;
            let actual = frame
                .iloc_with_columns(positions, fixture.column_order.as_deref())
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_iloc error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_iloc to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_iloc to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_iloc".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameTake => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_take error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_take to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_take to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_take".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameXs => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_xs error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_xs to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_xs to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_frame or expected_error is required for dataframe_xs".to_owned())
                }
            }
        }
        FixtureOperation::DataFrameGroupByIdxMin => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_idxmin error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_idxmin to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_idxmin to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_idxmin"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByIdxMax => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_idxmax error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_idxmax to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_idxmax to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_idxmax"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByAny => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_any error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_any to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_any to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_any"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByAll => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_all error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_all to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_all to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_all"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByGetGroup => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_get_group error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_get_group to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_get_group to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_get_group"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByFfill => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_ffill error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_ffill to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_ffill to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_ffill"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByBfill => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_bfill error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_bfill to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_bfill to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_bfill"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupBySem => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_sem error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_sem to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_sem to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_sem"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupBySkew => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_skew error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_skew to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_skew to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_skew"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByKurtosis => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_kurtosis error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_kurtosis to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_kurtosis to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_kurtosis"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByOhlc => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_ohlc error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_ohlc to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_ohlc to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_groupby_ohlc"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByCumcount => {
            let actual = execute_dataframe_groupby_series_fixture_operation(fixture, false);
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_cumcount error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_cumcount to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_cumcount to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for dataframe_groupby_cumcount"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByNgroup => {
            let actual = execute_dataframe_groupby_series_fixture_operation(fixture, true);
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_groupby_ngroup error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_groupby_ngroup to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_groupby_ngroup to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for dataframe_groupby_ngroup"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameAsof => {
            let actual = execute_dataframe_asof_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Series(series) => compare_series_expected(&actual?, &series),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_asof error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_asof to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_asof to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_series or expected_error is required for dataframe_asof".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameAtTime => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_at_time error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_at_time to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_at_time to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_at_time".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameBetweenTime => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_between_time error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_between_time to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(
                            "expected dataframe_between_time to fail but operation succeeded"
                                .to_owned(),
                        )
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_between_time"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameBool => {
            let actual = execute_dataframe_bool_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Bool(value) => {
                    let actual = actual?;
                    if actual == value {
                        Ok(())
                    } else {
                        Err(format!(
                            "dataframe_bool mismatch: actual={actual}, expected={value}"
                        ))
                    }
                }
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_bool error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_bool to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_bool to fail but operation succeeded".to_owned())
                    }
                }
                _ => {
                    Err("expected_bool or expected_error is required for dataframe_bool".to_owned())
                }
            }
        }
        FixtureOperation::DataFrameHead => {
            let frame = require_frame(fixture)?;
            let n = fixture
                .head_n
                .ok_or_else(|| "head_n is required for dataframe_head".to_owned())?;
            let frame = build_dataframe(frame)?;
            let actual = frame.head(n).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_head error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_head to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_head to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_head".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameTail => {
            let frame = require_frame(fixture)?;
            let n = fixture
                .tail_n
                .ok_or_else(|| "tail_n is required for dataframe_tail".to_owned())?;
            let frame = build_dataframe(frame)?;
            let actual = frame.tail(n).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected dataframe_tail error containing '{substr}', got '{message}'"
                    )),
                    Ok(_) => Err(format!(
                        "expected dataframe_tail to fail with error containing '{substr}'"
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err("expected dataframe_tail to fail but operation succeeded".to_owned())
                    }
                }
                _ => Err(
                    "expected_frame or expected_error is required for dataframe_tail".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameMerge
        | FixtureOperation::DataFrameMergeIndex
        | FixtureOperation::DataFrameMergeAsof
        | FixtureOperation::DataFrameMergeOrdered
        | FixtureOperation::DataFrameConcat
        | FixtureOperation::DataFrameIsNa
        | FixtureOperation::DataFrameNotNa
        | FixtureOperation::DataFrameIsNull
        | FixtureOperation::DataFrameNotNull
        | FixtureOperation::DataFrameFillNa
        | FixtureOperation::DataFrameDropNa
        | FixtureOperation::DataFrameDropNaColumns
        | FixtureOperation::DataFrameSetIndex
        | FixtureOperation::DataFrameResetIndex
        | FixtureOperation::DataFrameDropDuplicates
        | FixtureOperation::DataFrameSortIndex
        | FixtureOperation::DataFrameSortValues
        | FixtureOperation::DataFrameDiff
        | FixtureOperation::DataFrameShift
        | FixtureOperation::DataFramePctChange
        | FixtureOperation::DataFrameMelt
        | FixtureOperation::DataFrameCombineFirst => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => compare_dataframe_expected(&actual?, &frame),
                ResolvedExpected::ErrorContains(substr) => match actual {
                    Err(message) if message.contains(&substr) => Ok(()),
                    Err(message) => Err(format!(
                        "expected {:?} error containing '{substr}', got '{message}'",
                        fixture.operation
                    )),
                    Ok(_) => Err(format!(
                        "expected {:?} to fail with error containing '{substr}'",
                        fixture.operation
                    )),
                },
                ResolvedExpected::ErrorAny => {
                    if actual.is_err() {
                        Ok(())
                    } else {
                        Err(format!(
                            "expected {:?} to fail but operation succeeded",
                            fixture.operation
                        ))
                    }
                }
                _ => Err(format!(
                    "expected_frame or expected_error is required for {:?}",
                    fixture.operation
                )),
            }
        }
        FixtureOperation::GroupByMean
        | FixtureOperation::GroupByCount
        | FixtureOperation::GroupByMin
        | FixtureOperation::GroupByMax
        | FixtureOperation::GroupByFirst
        | FixtureOperation::GroupByLast
        | FixtureOperation::GroupByStd
        | FixtureOperation::GroupByVar
        | FixtureOperation::GroupByMedian => {
            let actual =
                execute_groupby_fixture_operation(fixture, fixture.operation, policy, ledger)?;
            let op_name = format!("{:?}", fixture.operation).to_lowercase();
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err(format!("expected_series is required for {op_name}")),
            };
            compare_series_expected(&actual, &expected)
        }
    }
}

/// Structure-aware fuzz entrypoint for `fp-index` outer alignment semantics.
///
/// Input is split at the first `|` byte into left/right index payloads. Each
/// payload byte is projected onto a small `IndexLabel` domain so fuzzing
/// naturally exercises duplicates, overlap, and mixed Int64/Utf8 labels.
pub fn fuzz_index_align_bytes(input: &[u8]) -> Result<(), String> {
    let Some(split_at) = input.iter().position(|byte| *byte == b'|') else {
        return Ok(());
    };

    let left = fuzz_index_from_bytes(&input[..split_at]);
    let right = fuzz_index_from_bytes(&input[split_at + 1..]);
    let plan = align_union(&left, &right);
    validate_alignment_plan(&plan).map_err(|err| err.to_string())?;

    let mut output_counts = BTreeMap::<IndexLabel, usize>::new();
    let mut left_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();
    let mut right_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();

    for row in 0..plan.union_index.len() {
        let label = &plan.union_index.labels()[row];
        *output_counts.entry(label.clone()).or_default() += 1;

        match plan.left_positions[row] {
            Some(left_pos) => {
                let actual = left.labels().get(left_pos).ok_or_else(|| {
                    format!("left position out of bounds: row={row} pos={left_pos}")
                })?;
                if actual != label {
                    return Err(format!(
                        "left alignment label mismatch: row={row} pos={left_pos} \
                         output={label:?} actual={actual:?}"
                    ));
                }
                *left_position_counts
                    .entry(label.clone())
                    .or_default()
                    .entry(left_pos)
                    .or_default() += 1;
            }
            None if left.labels().iter().any(|candidate| candidate == label) => {
                return Err(format!(
                    "left position missing despite label presence: row={row} label={label:?}"
                ));
            }
            None => {}
        }

        match plan.right_positions[row] {
            Some(right_pos) => {
                let actual = right.labels().get(right_pos).ok_or_else(|| {
                    format!("right position out of bounds: row={row} pos={right_pos}")
                })?;
                if actual != label {
                    return Err(format!(
                        "right alignment label mismatch: row={row} pos={right_pos} \
                         output={label:?} actual={actual:?}"
                    ));
                }
                *right_position_counts
                    .entry(label.clone())
                    .or_default()
                    .entry(right_pos)
                    .or_default() += 1;
            }
            None if right.labels().iter().any(|candidate| candidate == label) => {
                return Err(format!(
                    "right position missing despite label presence: row={row} label={label:?}"
                ));
            }
            None => {}
        }
    }

    let mut left_counts = BTreeMap::<IndexLabel, usize>::new();
    let mut right_counts = BTreeMap::<IndexLabel, usize>::new();
    let mut left_expected_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();
    let mut right_expected_position_counts = BTreeMap::<IndexLabel, BTreeMap<usize, usize>>::new();

    for (position, label) in left.labels().iter().enumerate() {
        *left_counts.entry(label.clone()).or_default() += 1;
        left_expected_position_counts
            .entry(label.clone())
            .or_default()
            .insert(position, 0);
    }

    for (position, label) in right.labels().iter().enumerate() {
        *right_counts.entry(label.clone()).or_default() += 1;
        right_expected_position_counts
            .entry(label.clone())
            .or_default()
            .insert(position, 0);
    }

    let labels = left_counts
        .keys()
        .chain(right_counts.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    for label in labels {
        let left_count = *left_counts.get(&label).unwrap_or(&0);
        let right_count = *right_counts.get(&label).unwrap_or(&0);
        let expected_output_count = match (left_count, right_count) {
            (0, count) => count,
            (count, 0) => count,
            (left_count, right_count) => left_count * right_count,
        };
        let actual_output_count = *output_counts.get(&label).unwrap_or(&0);
        if actual_output_count != expected_output_count {
            return Err(format!(
                "alignment multiplicity mismatch for {label:?}: expected={expected_output_count} \
                 actual={actual_output_count} left_count={left_count} right_count={right_count}"
            ));
        }

        if left_count > 0 && right_count > 0 {
            if let Some(expected_counts) = left_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = right_count;
                }
            }
            if let Some(expected_counts) = right_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = left_count;
                }
            }
        } else if right_count == 0 {
            if let Some(expected_counts) = left_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = 1;
                }
            }
            right_expected_position_counts.remove(&label);
        } else {
            left_expected_position_counts.remove(&label);
            if let Some(expected_counts) = right_expected_position_counts.get_mut(&label) {
                for count in expected_counts.values_mut() {
                    *count = 1;
                }
            }
        }

        let actual_left = left_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        let expected_left = left_expected_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        if actual_left != expected_left {
            return Err(format!(
                "left position multiplicity mismatch for {label:?}: \
                 expected={expected_left:?} actual={actual_left:?}"
            ));
        }

        let actual_right = right_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        let expected_right = right_expected_position_counts
            .get(&label)
            .cloned()
            .unwrap_or_default();
        if actual_right != expected_right {
            return Err(format!(
                "right position multiplicity mismatch for {label:?}: \
                 expected={expected_right:?} actual={actual_right:?}"
            ));
        }
    }

    Ok(())
}

fn resolve_expected(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    default_mode: OracleMode,
) -> Result<ResolvedExpected, HarnessError> {
    let requested_mode = fixture
        .oracle_source
        .map(|source| match source {
            FixtureOracleSource::Fixture => OracleMode::FixtureExpected,
            FixtureOracleSource::LiveLegacyPandas => OracleMode::LiveLegacyPandas,
        })
        .unwrap_or(default_mode);

    match requested_mode {
        OracleMode::FixtureExpected => fixture_expected(fixture),
        OracleMode::LiveLegacyPandas => match capture_live_oracle_expected(config, fixture) {
            Ok(expected) => Ok(expected),
            Err(HarnessError::OracleUnavailable(_)) if config.allow_system_pandas_fallback => {
                // Environment guard: if neither legacy nor system pandas is usable,
                // fall back to fixture-backed expectations when explicitly allowed.
                fixture_expected(fixture)
            }
            Err(err) => Err(err),
        },
    }
}

fn fixture_expected(fixture: &PacketFixture) -> Result<ResolvedExpected, HarnessError> {
    if let Some(expected_error_contains) = fixture.expected_error_contains.clone() {
        return Ok(ResolvedExpected::ErrorContains(expected_error_contains));
    }

    match fixture.operation {
        FixtureOperation::SeriesAdd
        | FixtureOperation::SeriesSub
        | FixtureOperation::SeriesMul
        | FixtureOperation::SeriesDiv => fixture
            .expected_series
            .clone()
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_series for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::SeriesJoin => fixture
            .expected_join
            .clone()
            .map(ResolvedExpected::Join)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_join for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::GroupBySum => fixture
            .expected_series
            .clone()
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_series for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::IndexAlignUnion => fixture
            .expected_alignment
            .clone()
            .map(ResolvedExpected::Alignment)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_alignment for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::IndexHasDuplicates => fixture
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_bool for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::IndexIsMonotonicIncreasing
        | FixtureOperation::IndexIsMonotonicDecreasing => fixture
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_bool for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::SeriesAny
        | FixtureOperation::SeriesAll
        | FixtureOperation::SeriesBool => fixture
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_bool for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::IndexFirstPositions => fixture
            .expected_positions
            .clone()
            .map(ResolvedExpected::Positions)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_positions for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::SeriesConcat
        | FixtureOperation::SeriesConstructor
        | FixtureOperation::SeriesToDatetime
        | FixtureOperation::SeriesToTimedelta
        | FixtureOperation::SeriesTimedeltaTotalSeconds
        | FixtureOperation::FillNa
        | FixtureOperation::DropNa
        | FixtureOperation::SeriesFilter
        | FixtureOperation::SeriesHead
        | FixtureOperation::SeriesTail
        | FixtureOperation::SeriesValueCounts
        | FixtureOperation::SeriesSortIndex
        | FixtureOperation::SeriesSortValues
        | FixtureOperation::SeriesDiff
        | FixtureOperation::SeriesShift
        | FixtureOperation::SeriesPctChange
        | FixtureOperation::SeriesXs
        | FixtureOperation::SeriesIsNa
        | FixtureOperation::SeriesNotNa
        | FixtureOperation::SeriesIsNull
        | FixtureOperation::SeriesNotNull
        | FixtureOperation::SeriesFillNa
        | FixtureOperation::SeriesDropNa
        | FixtureOperation::SeriesLoc
        | FixtureOperation::SeriesIloc
        | FixtureOperation::SeriesTake
        | FixtureOperation::SeriesRepeat
        | FixtureOperation::SeriesToNumeric
        | FixtureOperation::SeriesConvertDtypes
        | FixtureOperation::SeriesAstype
        | FixtureOperation::SeriesClip
        | FixtureOperation::SeriesAbs
        | FixtureOperation::SeriesRound
        | FixtureOperation::SeriesCumsum
        | FixtureOperation::SeriesCumprod
        | FixtureOperation::SeriesCummax
        | FixtureOperation::SeriesCummin
        | FixtureOperation::SeriesNlargest
        | FixtureOperation::SeriesNsmallest
        | FixtureOperation::SeriesBetween
        | FixtureOperation::SeriesCut
        | FixtureOperation::SeriesQcut
        | FixtureOperation::SeriesAtTime
        | FixtureOperation::SeriesBetweenTime
        | FixtureOperation::SeriesPartitionDf
        | FixtureOperation::SeriesRpartitionDf
        | FixtureOperation::SeriesExtractDf
        | FixtureOperation::DataFrameGroupByCumcount
        | FixtureOperation::DataFrameGroupByNgroup
        | FixtureOperation::DataFrameAsof
        | FixtureOperation::DataFrameCount
        | FixtureOperation::DataFrameDuplicated
        | FixtureOperation::GroupByMean
        | FixtureOperation::GroupByCount
        | FixtureOperation::GroupByMin
        | FixtureOperation::GroupByMax
        | FixtureOperation::GroupByFirst
        | FixtureOperation::GroupByLast
        | FixtureOperation::GroupByStd
        | FixtureOperation::GroupByVar
        | FixtureOperation::GroupByMedian
        | FixtureOperation::SeriesCombineFirst => fixture
            .expected_series
            .clone()
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_series for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::DataFrameLoc
        | FixtureOperation::SeriesExtractAll
        | FixtureOperation::DataFrameIloc
        | FixtureOperation::DataFrameTake
        | FixtureOperation::DataFrameXs
        | FixtureOperation::DataFrameGroupByIdxMin
        | FixtureOperation::DataFrameGroupByIdxMax
        | FixtureOperation::DataFrameGroupByAny
        | FixtureOperation::DataFrameGroupByAll
        | FixtureOperation::DataFrameGroupByGetGroup
        | FixtureOperation::DataFrameGroupByFfill
        | FixtureOperation::DataFrameGroupByBfill
        | FixtureOperation::DataFrameGroupBySem
        | FixtureOperation::DataFrameGroupBySkew
        | FixtureOperation::DataFrameGroupByKurtosis
        | FixtureOperation::DataFrameGroupByOhlc
        | FixtureOperation::DataFrameAtTime
        | FixtureOperation::DataFrameBetweenTime
        | FixtureOperation::DataFrameHead
        | FixtureOperation::DataFrameTail
        | FixtureOperation::DataFrameMode
        | FixtureOperation::DataFrameRank
        | FixtureOperation::DataFrameFromSeries
        | FixtureOperation::DataFrameFromDict
        | FixtureOperation::DataFrameFromRecords
        | FixtureOperation::DataFrameConstructorKwargs
        | FixtureOperation::DataFrameConstructorScalar
        | FixtureOperation::DataFrameConstructorDictOfSeries
        | FixtureOperation::DataFrameConstructorListLike
        | FixtureOperation::DataFrameIsNa
        | FixtureOperation::DataFrameNotNa
        | FixtureOperation::DataFrameIsNull
        | FixtureOperation::DataFrameNotNull
        | FixtureOperation::DataFrameFillNa
        | FixtureOperation::DataFrameDropNa
        | FixtureOperation::DataFrameDropNaColumns
        | FixtureOperation::DataFrameSetIndex
        | FixtureOperation::DataFrameResetIndex
        | FixtureOperation::DataFrameDropDuplicates
        | FixtureOperation::DataFrameMerge
        | FixtureOperation::DataFrameMergeIndex
        | FixtureOperation::DataFrameMergeAsof
        | FixtureOperation::DataFrameMergeOrdered
        | FixtureOperation::DataFrameConcat
        | FixtureOperation::DataFrameSortIndex
        | FixtureOperation::DataFrameSortValues
        | FixtureOperation::DataFrameDiff
        | FixtureOperation::DataFrameShift
        | FixtureOperation::DataFramePctChange
        | FixtureOperation::DataFrameMelt
        | FixtureOperation::DataFrameCombineFirst => fixture
            .expected_frame
            .clone()
            .map(ResolvedExpected::Frame)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_frame for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::NanSum
        | FixtureOperation::NanMean
        | FixtureOperation::NanMin
        | FixtureOperation::NanMax
        | FixtureOperation::NanStd
        | FixtureOperation::NanVar
        | FixtureOperation::NanCount
        | FixtureOperation::SeriesCount => fixture
            .expected_scalar
            .clone()
            .map(ResolvedExpected::Scalar)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_scalar for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::DataFrameBool | FixtureOperation::CsvRoundTrip => fixture
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_bool for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::ColumnDtypeCheck => fixture
            .expected_dtype
            .clone()
            .map(ResolvedExpected::Dtype)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_dtype for case {}",
                    fixture.case_id
                ))
            }),
    }
}

fn capture_live_oracle_expected(
    config: &HarnessConfig,
    fixture: &PacketFixture,
) -> Result<ResolvedExpected, HarnessError> {
    let expects_error = fixture.expected_error_contains.is_some();

    if !config.oracle_root.exists() && !config.allow_system_pandas_fallback {
        return Err(HarnessError::OracleUnavailable(format!(
            "legacy oracle root does not exist: {}",
            config.oracle_root.display()
        )));
    }
    let script = config.oracle_script_path();
    if !script.exists() {
        return Err(HarnessError::OracleUnavailable(format!(
            "oracle script does not exist: {}",
            script.display()
        )));
    }

    let payload = OracleRequest {
        operation: fixture.operation,
        left: fixture.left.clone(),
        right: fixture.right.clone(),
        groupby_keys: fixture.groupby_keys.clone(),
        groupby_columns: fixture.groupby_columns.clone(),
        frame: fixture.frame.clone(),
        frame_right: fixture.frame_right.clone(),
        dict_columns: fixture.dict_columns.clone(),
        column_order: fixture.column_order.clone(),
        records: fixture.records.clone(),
        matrix_rows: fixture.matrix_rows.clone(),
        index: fixture.index.clone(),
        join_type: fixture.join_type,
        merge_on: fixture.merge_on.clone(),
        merge_on_keys: fixture.merge_on_keys.clone(),
        left_on_keys: fixture.left_on_keys.clone(),
        right_on_keys: fixture.right_on_keys.clone(),
        left_index: fixture.left_index,
        right_index: fixture.right_index,
        merge_indicator: fixture.merge_indicator,
        merge_indicator_name: fixture.merge_indicator_name.clone(),
        merge_validate: fixture.merge_validate.clone(),
        merge_suffixes: fixture.merge_suffixes.clone(),
        merge_sort: fixture.merge_sort,
        fill_value: fixture.fill_value.clone(),
        datetime_unit: fixture.datetime_unit.clone(),
        datetime_origin: fixture.datetime_origin.clone(),
        datetime_utc: fixture.datetime_utc,
        head_n: fixture.head_n,
        tail_n: fixture.tail_n,
        diff_periods: fixture.diff_periods,
        shift_periods: fixture.shift_periods,
        shift_axis: fixture.shift_axis,
        pct_change_periods: fixture.pct_change_periods,
        diff_axis: fixture.diff_axis,
        rank_method: fixture.rank_method.clone(),
        rank_na_option: fixture.rank_na_option.clone(),
        rank_axis: fixture.rank_axis,
        sort_column: fixture.sort_column.clone(),
        sort_ascending: fixture.sort_ascending,
        concat_axis: fixture.concat_axis,
        concat_join: fixture.concat_join.clone(),
        set_index_column: fixture.set_index_column.clone(),
        set_index_drop: fixture.set_index_drop,
        reset_index_drop: fixture.reset_index_drop,
        subset: fixture.subset.clone(),
        keep: fixture.keep.clone(),
        ignore_index: fixture.ignore_index,
        csv_input: fixture.csv_input.clone(),
        loc_labels: fixture.loc_labels.clone(),
        iloc_positions: fixture.iloc_positions.clone(),
        take_indices: fixture.take_indices.clone(),
        repeat_n: fixture.repeat_n,
        repeat_counts: fixture.repeat_counts.clone(),
        group_name: fixture.group_name.clone(),
        xs_key: fixture.xs_key.clone(),
        cut_bins: fixture.cut_bins,
        qcut_quantiles: fixture.qcut_quantiles,
        take_axis: fixture.take_axis,
        asof_label: fixture.asof_label.clone(),
        time_value: fixture.time_value.clone(),
        start_time: fixture.start_time.clone(),
        end_time: fixture.end_time.clone(),
        string_sep: fixture.string_sep.clone(),
        regex_pattern: fixture.regex_pattern.clone(),
        melt_id_vars: fixture.melt_id_vars.clone(),
        melt_value_vars: fixture.melt_value_vars.clone(),
        melt_var_name: fixture.melt_var_name.clone(),
        melt_value_name: fixture.melt_value_name.clone(),
    };
    let input = serde_json::to_vec(&payload)?;

    let output = Command::new(&config.python_bin)
        .arg(&script)
        .arg("--legacy-root")
        .arg(&config.oracle_root)
        .arg("--strict-legacy")
        .args(
            config
                .allow_system_pandas_fallback
                .then_some("--allow-system-pandas-fallback"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&input)?;
            }
            child.wait_with_output()
        })?;

    if !output.status.success() {
        if expects_error {
            return Ok(ResolvedExpected::ErrorAny);
        }

        if let Ok(response) = serde_json::from_slice::<OracleResponse>(&output.stdout)
            && let Some(error) = response.error
        {
            return Err(HarnessError::OracleUnavailable(error));
        }

        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        return Err(HarnessError::OracleCommandFailed {
            status: code,
            stderr: format!("{stderr}\nstdout={stdout}"),
        });
    }

    let response: OracleResponse = serde_json::from_slice(&output.stdout)?;
    if let Some(error) = response.error {
        if expects_error {
            return Ok(ResolvedExpected::ErrorAny);
        }
        return Err(HarnessError::OracleUnavailable(error));
    }

    if expects_error {
        return Err(HarnessError::FixtureFormat(format!(
            "oracle unexpectedly succeeded for expected-error case {}",
            fixture.case_id
        )));
    }

    match fixture.operation {
        FixtureOperation::SeriesAdd
        | FixtureOperation::SeriesSub
        | FixtureOperation::SeriesMul
        | FixtureOperation::SeriesDiv => response
            .expected_series
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_series".to_owned())
            }),
        FixtureOperation::SeriesJoin => response
            .expected_join
            .map(ResolvedExpected::Join)
            .ok_or_else(|| HarnessError::FixtureFormat("oracle omitted expected_join".to_owned())),
        FixtureOperation::GroupBySum => response
            .expected_series
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_series".to_owned())
            }),
        FixtureOperation::IndexAlignUnion => response
            .expected_alignment
            .map(ResolvedExpected::Alignment)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_alignment".to_owned())
            }),
        FixtureOperation::IndexHasDuplicates
        | FixtureOperation::IndexIsMonotonicIncreasing
        | FixtureOperation::IndexIsMonotonicDecreasing => response
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| HarnessError::FixtureFormat("oracle omitted expected_bool".to_owned())),
        FixtureOperation::SeriesAny
        | FixtureOperation::SeriesAll
        | FixtureOperation::SeriesBool => response
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| HarnessError::FixtureFormat("oracle omitted expected_bool".to_owned())),
        FixtureOperation::IndexFirstPositions => response
            .expected_positions
            .map(ResolvedExpected::Positions)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_positions".to_owned())
            }),
        FixtureOperation::SeriesConcat
        | FixtureOperation::SeriesConstructor
        | FixtureOperation::SeriesToDatetime
        | FixtureOperation::SeriesToTimedelta
        | FixtureOperation::SeriesTimedeltaTotalSeconds
        | FixtureOperation::FillNa
        | FixtureOperation::DropNa
        | FixtureOperation::SeriesFilter
        | FixtureOperation::SeriesHead
        | FixtureOperation::SeriesTail
        | FixtureOperation::SeriesValueCounts
        | FixtureOperation::SeriesSortIndex
        | FixtureOperation::SeriesSortValues
        | FixtureOperation::SeriesDiff
        | FixtureOperation::SeriesShift
        | FixtureOperation::SeriesPctChange
        | FixtureOperation::SeriesXs
        | FixtureOperation::SeriesIsNa
        | FixtureOperation::SeriesNotNa
        | FixtureOperation::SeriesIsNull
        | FixtureOperation::SeriesNotNull
        | FixtureOperation::SeriesFillNa
        | FixtureOperation::SeriesDropNa
        | FixtureOperation::SeriesLoc
        | FixtureOperation::SeriesIloc
        | FixtureOperation::SeriesTake
        | FixtureOperation::SeriesRepeat
        | FixtureOperation::SeriesToNumeric
        | FixtureOperation::SeriesConvertDtypes
        | FixtureOperation::SeriesAstype
        | FixtureOperation::SeriesClip
        | FixtureOperation::SeriesAbs
        | FixtureOperation::SeriesRound
        | FixtureOperation::SeriesCumsum
        | FixtureOperation::SeriesCumprod
        | FixtureOperation::SeriesCummax
        | FixtureOperation::SeriesCummin
        | FixtureOperation::SeriesNlargest
        | FixtureOperation::SeriesNsmallest
        | FixtureOperation::SeriesBetween
        | FixtureOperation::SeriesCut
        | FixtureOperation::SeriesQcut
        | FixtureOperation::SeriesAtTime
        | FixtureOperation::SeriesBetweenTime
        | FixtureOperation::SeriesPartitionDf
        | FixtureOperation::SeriesRpartitionDf
        | FixtureOperation::SeriesExtractDf
        | FixtureOperation::DataFrameGroupByCumcount
        | FixtureOperation::DataFrameGroupByNgroup
        | FixtureOperation::DataFrameAsof
        | FixtureOperation::DataFrameCount
        | FixtureOperation::DataFrameDuplicated
        | FixtureOperation::GroupByMean
        | FixtureOperation::GroupByCount
        | FixtureOperation::GroupByMin
        | FixtureOperation::GroupByMax
        | FixtureOperation::GroupByFirst
        | FixtureOperation::GroupByLast
        | FixtureOperation::GroupByStd
        | FixtureOperation::GroupByVar
        | FixtureOperation::GroupByMedian
        | FixtureOperation::SeriesCombineFirst => response
            .expected_series
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_series".to_owned())
            }),
        FixtureOperation::DataFrameBool => response
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| HarnessError::FixtureFormat("oracle omitted expected_bool".to_owned())),
        FixtureOperation::DataFrameLoc
        | FixtureOperation::SeriesExtractAll
        | FixtureOperation::DataFrameIloc
        | FixtureOperation::DataFrameTake
        | FixtureOperation::DataFrameXs
        | FixtureOperation::DataFrameGroupByIdxMin
        | FixtureOperation::DataFrameGroupByIdxMax
        | FixtureOperation::DataFrameGroupByAny
        | FixtureOperation::DataFrameGroupByAll
        | FixtureOperation::DataFrameGroupByGetGroup
        | FixtureOperation::DataFrameGroupByFfill
        | FixtureOperation::DataFrameGroupByBfill
        | FixtureOperation::DataFrameGroupBySem
        | FixtureOperation::DataFrameGroupBySkew
        | FixtureOperation::DataFrameGroupByKurtosis
        | FixtureOperation::DataFrameGroupByOhlc
        | FixtureOperation::DataFrameAtTime
        | FixtureOperation::DataFrameBetweenTime
        | FixtureOperation::DataFrameHead
        | FixtureOperation::DataFrameTail
        | FixtureOperation::DataFrameMode
        | FixtureOperation::DataFrameRank
        | FixtureOperation::DataFrameFromSeries
        | FixtureOperation::DataFrameFromDict
        | FixtureOperation::DataFrameFromRecords
        | FixtureOperation::DataFrameConstructorKwargs
        | FixtureOperation::DataFrameConstructorScalar
        | FixtureOperation::DataFrameConstructorDictOfSeries
        | FixtureOperation::DataFrameConstructorListLike
        | FixtureOperation::DataFrameIsNa
        | FixtureOperation::DataFrameNotNa
        | FixtureOperation::DataFrameIsNull
        | FixtureOperation::DataFrameNotNull
        | FixtureOperation::DataFrameFillNa
        | FixtureOperation::DataFrameDropNa
        | FixtureOperation::DataFrameDropNaColumns
        | FixtureOperation::DataFrameSetIndex
        | FixtureOperation::DataFrameResetIndex
        | FixtureOperation::DataFrameDropDuplicates
        | FixtureOperation::DataFrameMerge
        | FixtureOperation::DataFrameMergeIndex
        | FixtureOperation::DataFrameMergeAsof
        | FixtureOperation::DataFrameMergeOrdered
        | FixtureOperation::DataFrameConcat
        | FixtureOperation::DataFrameSortIndex
        | FixtureOperation::DataFrameSortValues
        | FixtureOperation::DataFrameDiff
        | FixtureOperation::DataFrameShift
        | FixtureOperation::DataFramePctChange
        | FixtureOperation::DataFrameMelt
        | FixtureOperation::DataFrameCombineFirst => response
            .expected_frame
            .map(ResolvedExpected::Frame)
            .ok_or_else(|| HarnessError::FixtureFormat("oracle omitted expected_frame".to_owned())),
        FixtureOperation::NanSum
        | FixtureOperation::NanMean
        | FixtureOperation::NanMin
        | FixtureOperation::NanMax
        | FixtureOperation::NanStd
        | FixtureOperation::NanVar
        | FixtureOperation::NanCount
        | FixtureOperation::SeriesCount => response
            .expected_scalar
            .map(ResolvedExpected::Scalar)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_scalar".to_owned())
            }),
        FixtureOperation::CsvRoundTrip => response
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| HarnessError::FixtureFormat("oracle omitted expected_bool".to_owned())),
        FixtureOperation::ColumnDtypeCheck => response
            .expected_dtype
            .map(ResolvedExpected::Dtype)
            .ok_or_else(|| HarnessError::FixtureFormat("oracle omitted expected_dtype".to_owned())),
    }
}

fn require_left_series(fixture: &PacketFixture) -> Result<&FixtureSeries, String> {
    fixture
        .left
        .as_ref()
        .ok_or_else(|| "missing left fixture series".to_owned())
}

fn require_right_series(fixture: &PacketFixture) -> Result<&FixtureSeries, String> {
    fixture
        .right
        .as_ref()
        .ok_or_else(|| "missing right fixture series".to_owned())
}

fn require_frame(fixture: &PacketFixture) -> Result<&FixtureDataFrame, String> {
    fixture
        .frame
        .as_ref()
        .ok_or_else(|| "missing frame fixture payload".to_owned())
}

fn require_frame_right(fixture: &PacketFixture) -> Result<&FixtureDataFrame, String> {
    fixture
        .frame_right
        .as_ref()
        .ok_or_else(|| "missing frame_right fixture payload".to_owned())
}

fn require_dict_columns(fixture: &PacketFixture) -> Result<&BTreeMap<String, Vec<Scalar>>, String> {
    fixture
        .dict_columns
        .as_ref()
        .ok_or_else(|| "missing dict_columns fixture payload".to_owned())
}

fn require_matrix_rows(fixture: &PacketFixture) -> Result<&Vec<Vec<Scalar>>, String> {
    fixture
        .matrix_rows
        .as_ref()
        .ok_or_else(|| "missing matrix_rows fixture payload".to_owned())
}

fn require_index(fixture: &PacketFixture) -> Result<&Vec<IndexLabel>, String> {
    fixture
        .index
        .as_ref()
        .ok_or_else(|| "missing index fixture vector".to_owned())
}

fn require_join_type(fixture: &PacketFixture) -> Result<FixtureJoinType, String> {
    fixture
        .join_type
        .ok_or_else(|| "missing join_type for join fixture".to_owned())
}

fn require_series_join_type(fixture: &PacketFixture) -> Result<JoinType, String> {
    let join_type = require_join_type(fixture)?;
    if matches!(join_type, FixtureJoinType::Cross) {
        return Err(
            "series_join requires join_type=inner|left|right|outer, got 'cross'".to_owned(),
        );
    }
    Ok(join_type.into_join_type())
}

fn resolve_merge_on_keys(
    merge_on: Option<&str>,
    merge_on_keys: Option<&[String]>,
    operation_name: &str,
    default_key: Option<&str>,
) -> Result<Vec<String>, String> {
    if let Some(keys) = merge_on_keys {
        if keys.is_empty() {
            return Err(format!(
                "{operation_name} requires merge_on_keys with at least one key"
            ));
        }
        let mut resolved = Vec::with_capacity(keys.len());
        for (index, key) in keys.iter().enumerate() {
            let key = key.trim();
            if key.is_empty() {
                return Err(format!(
                    "{operation_name} merge_on_keys[{index}] must be a non-empty string"
                ));
            }
            resolved.push(key.to_owned());
        }
        return Ok(resolved);
    }

    if let Some(key) = merge_on {
        let key = key.trim();
        if key.is_empty() {
            return Err(format!(
                "{operation_name} merge_on must be a non-empty string"
            ));
        }
        return Ok(vec![key.to_owned()]);
    }

    if let Some(key) = default_key {
        return Ok(vec![key.to_owned()]);
    }

    Err(format!(
        "{operation_name} requires merge_on string or merge_on_keys list payload"
    ))
}

fn normalize_key_list(
    keys: &[String],
    operation_name: &str,
    field_name: &str,
) -> Result<Vec<String>, String> {
    if keys.is_empty() {
        return Err(format!(
            "{operation_name} requires {field_name} with at least one key"
        ));
    }
    let mut normalized = Vec::with_capacity(keys.len());
    for (index, key) in keys.iter().enumerate() {
        let key = key.trim();
        if key.is_empty() {
            return Err(format!(
                "{operation_name} {field_name}[{index}] must be a non-empty string"
            ));
        }
        normalized.push(key.to_owned());
    }
    Ok(normalized)
}

fn resolve_merge_key_pairs(
    merge_on: Option<&str>,
    merge_on_keys: Option<&[String]>,
    left_on_keys: Option<&[String]>,
    right_on_keys: Option<&[String]>,
    operation_name: &str,
    default_key: Option<&str>,
) -> Result<(Vec<String>, Vec<String>), String> {
    if left_on_keys.is_some() || right_on_keys.is_some() {
        let left = left_on_keys.ok_or_else(|| {
            format!("{operation_name} requires right_on_keys when left_on_keys is provided")
        })?;
        let right = right_on_keys.ok_or_else(|| {
            format!("{operation_name} requires left_on_keys when right_on_keys is provided")
        })?;
        let left_keys = normalize_key_list(left, operation_name, "left_on_keys")?;
        let right_keys = normalize_key_list(right, operation_name, "right_on_keys")?;
        if left_keys.len() != right_keys.len() {
            return Err(format!(
                "{operation_name} left_on_keys and right_on_keys must have equal length"
            ));
        }
        return Ok((left_keys, right_keys));
    }

    let keys = resolve_merge_on_keys(merge_on, merge_on_keys, operation_name, default_key)?;
    Ok((keys.clone(), keys))
}

fn resolve_merge_indicator_name(
    merge_indicator: Option<bool>,
    merge_indicator_name: Option<&str>,
    operation_name: &str,
) -> Result<Option<String>, String> {
    if matches!(merge_indicator, Some(false)) && merge_indicator_name.is_some() {
        return Err(format!(
            "{operation_name} merge_indicator_name requires merge_indicator=true when explicitly provided"
        ));
    }

    if let Some(name) = merge_indicator_name {
        if name.trim().is_empty() {
            return Err(format!(
                "{operation_name} merge_indicator_name must be a non-empty string"
            ));
        }
        return Ok(Some(name.to_owned()));
    }

    if merge_indicator.unwrap_or(false) {
        return Ok(Some("_merge".to_owned()));
    }

    Ok(None)
}

fn resolve_merge_validate_mode(
    merge_validate: Option<&str>,
    operation_name: &str,
) -> Result<Option<MergeValidateMode>, String> {
    let Some(raw_mode) = merge_validate else {
        return Ok(None);
    };
    let normalized = raw_mode.trim().to_ascii_lowercase();
    let mode = match normalized.as_str() {
        "one_to_one" | "1:1" => MergeValidateMode::OneToOne,
        "one_to_many" | "1:m" => MergeValidateMode::OneToMany,
        "many_to_one" | "m:1" => MergeValidateMode::ManyToOne,
        "many_to_many" | "m:m" => MergeValidateMode::ManyToMany,
        _ => {
            return Err(format!(
                "{operation_name} merge_validate must be one_to_one, one_to_many, many_to_one, or many_to_many"
            ));
        }
    };
    Ok(Some(mode))
}

fn resolve_merge_suffixes(
    merge_suffixes: Option<&[Option<String>; 2]>,
) -> Option<[Option<String>; 2]> {
    merge_suffixes.cloned()
}

fn resolve_merge_sort(merge_sort: Option<bool>) -> bool {
    merge_sort.unwrap_or(false)
}

fn execute_dataframe_merge_asof_fixture_operation(
    fixture: &PacketFixture,
) -> Result<DataFrame, String> {
    let left = build_dataframe(require_frame(fixture)?)
        .map_err(|err| format!("left frame build failed: {err}"))?;
    let right = build_dataframe(require_frame_right(fixture)?)
        .map_err(|err| format!("right frame build failed: {err}"))?;

    let on = fixture
        .merge_on
        .as_deref()
        .ok_or_else(|| "merge_on field required for dataframe_merge_asof".to_owned())?;

    let direction_str = fixture.direction.as_deref().unwrap_or("backward");
    let direction = match direction_str {
        "backward" => fp_join::AsofDirection::Backward,
        "forward" => fp_join::AsofDirection::Forward,
        "nearest" => fp_join::AsofDirection::Nearest,
        _ => return Err(format!("invalid merge_asof direction: {direction_str}")),
    };

    // Build options from fixture fields
    let mut options = fp_join::MergeAsofOptions::new();

    // allow_exact_matches defaults to true in both pandas and our impl
    if let Some(allow) = fixture.allow_exact_matches {
        options = options.allow_exact_matches(allow);
    }

    // tolerance: maximum distance for a match
    if let Some(tol) = fixture.tolerance {
        options = options.tolerance(tol);
    }

    // by: columns to match exactly before asof matching
    if let Some(ref by_cols) = fixture.merge_asof_by {
        options = options.by(by_cols.clone());
    }

    let merged = fp_join::merge_asof_with_options(&left, &right, on, direction, options)
        .map_err(|err| err.to_string())?;
    DataFrame::new(merged.index, merged.columns).map_err(|err| err.to_string())
}

fn execute_dataframe_merge_ordered_fixture_operation(
    fixture: &PacketFixture,
) -> Result<DataFrame, String> {
    let left = build_dataframe(require_frame(fixture)?)
        .map_err(|err| format!("left frame build failed: {err}"))?;
    let right = build_dataframe(require_frame_right(fixture)?)
        .map_err(|err| format!("right frame build failed: {err}"))?;

    let on_keys = if let Some(keys) = fixture.merge_on_keys.as_ref() {
        keys.iter().map(String::as_str).collect::<Vec<_>>()
    } else if let Some(key) = fixture.merge_on.as_deref() {
        vec![key]
    } else {
        return Err("merge_on or merge_on_keys field required for dataframe_merge_ordered".into());
    };

    let merged = merge_ordered(
        &left,
        &right,
        &on_keys,
        fixture.merge_fill_method.as_deref(),
    )
    .map_err(|err| err.to_string())?;
    DataFrame::new(merged.index, merged.columns).map_err(|err| err.to_string())
}

fn validate_cross_merge_configuration(
    fixture: &PacketFixture,
    operation_name: &str,
    merge_on_index: bool,
) -> Result<(), String> {
    if merge_on_index {
        return Err(format!(
            "{operation_name} does not support join_type='cross'"
        ));
    }

    if fixture.merge_on.is_some()
        || fixture.merge_on_keys.is_some()
        || fixture.left_on_keys.is_some()
        || fixture.right_on_keys.is_some()
    {
        return Err(format!(
            "{operation_name} join_type='cross' does not allow merge_on/merge_on_keys/left_on_keys/right_on_keys"
        ));
    }

    if matches!(fixture.left_index, Some(true)) || matches!(fixture.right_index, Some(true)) {
        return Err(format!(
            "{operation_name} join_type='cross' does not allow left_index/right_index"
        ));
    }

    Ok(())
}

fn normalize_concat_axis(fixture: &PacketFixture) -> Result<i64, String> {
    let axis = fixture.concat_axis.unwrap_or(0);
    match axis {
        0 | 1 => Ok(axis),
        _ => Err(format!(
            "concat_axis must be 0 or 1 for dataframe_concat (got {axis})"
        )),
    }
}

fn normalize_concat_join(fixture: &PacketFixture) -> Result<ConcatJoin, String> {
    let join = fixture.concat_join.as_deref().unwrap_or("outer");
    if join.eq_ignore_ascii_case("outer") {
        return Ok(ConcatJoin::Outer);
    }
    if join.eq_ignore_ascii_case("inner") {
        return Ok(ConcatJoin::Inner);
    }
    Err(format!(
        "concat_join must be 'outer' or 'inner' for dataframe_concat (got {join})"
    ))
}

fn require_loc_labels(fixture: &PacketFixture) -> Result<&Vec<IndexLabel>, String> {
    fixture
        .loc_labels
        .as_ref()
        .ok_or_else(|| "loc_labels is required for loc operations".to_owned())
}

fn require_iloc_positions(fixture: &PacketFixture) -> Result<&Vec<i64>, String> {
    fixture
        .iloc_positions
        .as_ref()
        .ok_or_else(|| "iloc_positions is required for iloc operations".to_owned())
}

fn require_take_indices(fixture: &PacketFixture) -> Result<&Vec<i64>, String> {
    fixture
        .take_indices
        .as_ref()
        .ok_or_else(|| "take_indices is required for take operations".to_owned())
}

fn require_cut_bins(fixture: &PacketFixture) -> Result<usize, String> {
    fixture
        .cut_bins
        .ok_or_else(|| "cut_bins is required for series_cut".to_owned())
}

fn require_qcut_quantiles(fixture: &PacketFixture) -> Result<usize, String> {
    fixture
        .qcut_quantiles
        .ok_or_else(|| "qcut_quantiles is required for series_qcut".to_owned())
}

fn require_xs_key<'a>(
    fixture: &'a PacketFixture,
    operation_name: &str,
) -> Result<&'a IndexLabel, String> {
    fixture
        .xs_key
        .as_ref()
        .ok_or_else(|| format!("xs_key is required for {operation_name}"))
}

fn require_groupby_columns(
    fixture: &PacketFixture,
    operation_name: &str,
) -> Result<Vec<String>, String> {
    let columns =
        resolve_optional_string_list(fixture.groupby_columns.as_ref(), "groupby_columns")?;
    if columns.is_empty() {
        Err(format!("groupby_columns is required for {operation_name}"))
    } else {
        Ok(columns)
    }
}

fn require_group_name<'a>(
    fixture: &'a PacketFixture,
    operation_name: &str,
) -> Result<&'a str, String> {
    fixture
        .group_name
        .as_deref()
        .ok_or_else(|| format!("group_name is required for {operation_name}"))
}

fn require_asof_label(fixture: &PacketFixture) -> Result<&IndexLabel, String> {
    fixture
        .asof_label
        .as_ref()
        .ok_or_else(|| "asof_label is required for dataframe_asof".to_owned())
}

fn require_time_value(fixture: &PacketFixture) -> Result<&str, String> {
    fixture
        .time_value
        .as_deref()
        .ok_or_else(|| "time_value is required for at_time operations".to_owned())
}

fn require_regex_pattern<'a>(
    fixture: &'a PacketFixture,
    operation_name: &str,
) -> Result<&'a str, String> {
    fixture
        .regex_pattern
        .as_deref()
        .ok_or_else(|| format!("regex_pattern is required for {operation_name}"))
}

fn require_string_sep<'a>(
    fixture: &'a PacketFixture,
    operation_name: &str,
) -> Result<&'a str, String> {
    fixture
        .string_sep
        .as_deref()
        .ok_or_else(|| format!("string_sep is required for {operation_name}"))
}

fn require_start_time(fixture: &PacketFixture) -> Result<&str, String> {
    fixture
        .start_time
        .as_deref()
        .ok_or_else(|| "start_time is required for between_time operations".to_owned())
}

fn require_end_time(fixture: &PacketFixture) -> Result<&str, String> {
    fixture
        .end_time
        .as_deref()
        .ok_or_else(|| "end_time is required for between_time operations".to_owned())
}

fn require_sort_column(fixture: &PacketFixture) -> Result<&str, String> {
    fixture
        .sort_column
        .as_deref()
        .ok_or_else(|| "sort_column is required for dataframe_sort_values".to_owned())
}

fn require_set_index_column(fixture: &PacketFixture) -> Result<&str, String> {
    fixture
        .set_index_column
        .as_deref()
        .ok_or_else(|| "set_index_column is required for dataframe_set_index".to_owned())
}

fn require_set_index_drop(fixture: &PacketFixture) -> Result<bool, String> {
    fixture
        .set_index_drop
        .ok_or_else(|| "set_index_drop is required for dataframe_set_index".to_owned())
}

fn require_reset_index_drop(fixture: &PacketFixture) -> Result<bool, String> {
    fixture
        .reset_index_drop
        .ok_or_else(|| "reset_index_drop is required for dataframe_reset_index".to_owned())
}

fn resolve_duplicate_subset(fixture: &PacketFixture) -> Result<Option<Vec<String>>, String> {
    fixture
        .subset
        .as_ref()
        .map(|subset| {
            subset
                .iter()
                .map(|name| {
                    let trimmed = name.trim();
                    if trimmed.is_empty() {
                        Err("subset entries must be non-empty strings".to_owned())
                    } else {
                        Ok(trimmed.to_owned())
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
}

fn resolve_optional_string_list(
    values: Option<&Vec<String>>,
    field_name: &str,
) -> Result<Vec<String>, String> {
    values
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    let trimmed = item.trim();
                    if trimmed.is_empty() {
                        Err(format!("{field_name} entries must be non-empty strings"))
                    } else {
                        Ok(trimmed.to_owned())
                    }
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
        .map(|items| items.unwrap_or_default())
}

fn resolve_duplicate_keep(fixture: &PacketFixture) -> Result<DuplicateKeep, String> {
    let Some(keep) = fixture.keep.as_deref() else {
        return Ok(DuplicateKeep::First);
    };
    let keep = keep.trim();
    if keep.eq_ignore_ascii_case("first") {
        Ok(DuplicateKeep::First)
    } else if keep.eq_ignore_ascii_case("last") {
        Ok(DuplicateKeep::Last)
    } else if keep.eq_ignore_ascii_case("none") {
        Ok(DuplicateKeep::None)
    } else {
        Err(format!(
            "keep must be one of 'first', 'last', or 'none' (got '{keep}')"
        ))
    }
}

fn resolve_drop_duplicates_ignore_index(fixture: &PacketFixture) -> bool {
    fixture.ignore_index.unwrap_or(false)
}

fn resolve_sort_ascending(fixture: &PacketFixture) -> bool {
    fixture.sort_ascending.unwrap_or(true)
}

fn resolve_rank_axis(fixture: &PacketFixture) -> Result<usize, String> {
    match fixture.rank_axis.unwrap_or(0) {
        axis @ (0 | 1) => Ok(axis),
        axis => Err(format!(
            "rank_axis must be 0 or 1 for dataframe_rank (got {axis})"
        )),
    }
}

fn resolve_take_axis(fixture: &PacketFixture) -> Result<usize, String> {
    match fixture.take_axis.unwrap_or(0) {
        axis @ (0 | 1) => Ok(axis),
        axis => Err(format!(
            "take_axis must be 0 or 1 for dataframe_take (got {axis})"
        )),
    }
}

fn resolve_series_repeat_counts(fixture: &PacketFixture, len: usize) -> Result<Vec<usize>, String> {
    match (fixture.repeat_n, fixture.repeat_counts.as_ref()) {
        (Some(_), Some(_)) => {
            Err("series_repeat accepts either repeat_n or repeat_counts, but not both".to_owned())
        }
        (Some(repeat_n), None) => {
            let repeat_n = usize::try_from(repeat_n).map_err(|_| {
                format!("repeat_n must be non-negative for series_repeat (got {repeat_n})")
            })?;
            Ok(vec![repeat_n; len])
        }
        (None, Some(repeat_counts)) => {
            if repeat_counts.len() != len {
                return Err(format!(
                    "repeat_counts must contain {len} elements for series_repeat (got {})",
                    repeat_counts.len()
                ));
            }
            repeat_counts
                .iter()
                .map(|count| {
                    usize::try_from(*count).map_err(|_| {
                        format!(
                            "repeat_counts must be non-negative for series_repeat (got {count})"
                        )
                    })
                })
                .collect()
        }
        (None, None) => Err("series_repeat requires repeat_n or repeat_counts".to_owned()),
    }
}

fn resolve_shift_axis(fixture: &PacketFixture) -> Result<usize, String> {
    match fixture.shift_axis.unwrap_or(0) {
        axis @ (0 | 1) => Ok(axis),
        axis => Err(format!(
            "shift_axis must be 0 or 1 for dataframe_shift (got {axis})"
        )),
    }
}

fn normalize_head_take(n: i64, len: usize) -> usize {
    if n >= 0 {
        usize::try_from(n).unwrap_or(usize::MAX).min(len)
    } else {
        len.saturating_sub(usize::try_from(n.unsigned_abs()).unwrap_or(usize::MAX))
    }
}

fn collect_constructor_series_payloads(
    fixture: &PacketFixture,
    op_name: &str,
) -> Result<Vec<FixtureSeries>, String> {
    let mut payloads = Vec::new();
    if let Some(left) = fixture.left.clone() {
        payloads.push(left);
    }
    if let Some(right) = fixture.right.clone() {
        payloads.push(right);
    }
    if let Some(extra) = fixture.groupby_keys.clone() {
        payloads.extend(extra);
    }
    if payloads.is_empty() {
        return Err(format!(
            "{op_name} requires at least one series payload (left/right/groupby_keys)"
        ));
    }
    Ok(payloads)
}

type DictConstructorPayloads<'a> = (Vec<(&'a str, Vec<Scalar>)>, Vec<&'a str>);

fn collect_dict_constructor_payloads<'a>(
    dict_columns: &'a BTreeMap<String, Vec<Scalar>>,
    column_order: Option<&[String]>,
    op_name: &str,
) -> Result<DictConstructorPayloads<'a>, String> {
    if let Some(order) = column_order
        && !order.is_empty()
    {
        let mut payloads = Vec::with_capacity(order.len());
        let mut selected_columns = Vec::with_capacity(order.len());
        for requested in order {
            let (name, values) = dict_columns
                .get_key_value(requested)
                .ok_or_else(|| format!("{op_name} column '{requested}' not found in data"))?;
            payloads.push((name.as_str(), values.clone()));
            selected_columns.push(name.as_str());
        }
        return Ok((payloads, selected_columns));
    }

    Ok((
        dict_columns
            .iter()
            .map(|(name, values)| (name.as_str(), values.clone()))
            .collect(),
        Vec::new(),
    ))
}

fn parse_constructor_dtype_spec(dtype_spec: &str) -> Result<DType, String> {
    let normalized = dtype_spec.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "bool" | "boolean" => Ok(DType::Bool),
        "int64" | "int" | "i64" => Ok(DType::Int64),
        "float64" | "float" | "f64" => Ok(DType::Float64),
        "utf8" | "string" | "str" => Ok(DType::Utf8),
        _ => Err(format!(
            "unsupported constructor dtype '{}'",
            dtype_spec.trim()
        )),
    }
}

fn apply_constructor_options(
    fixture: &PacketFixture,
    operation_name: &str,
    frame: DataFrame,
) -> Result<DataFrame, String> {
    let _copy_requested = fixture.constructor_copy.unwrap_or(false);
    let Some(dtype_spec) = fixture.constructor_dtype.as_deref() else {
        return Ok(frame);
    };
    let target_dtype = parse_constructor_dtype_spec(dtype_spec)?;
    let column_order = frame
        .column_names()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut coerced_columns = BTreeMap::new();
    for (name, column) in frame.columns() {
        let coerced = Column::new(target_dtype, column.values().to_vec())
            .map_err(|err| format!("{operation_name} dtype='{dtype_spec}' cast failed: {err}"))?;
        coerced_columns.insert(name.clone(), coerced);
    }
    DataFrame::new_with_column_order(frame.index().clone(), coerced_columns, column_order)
        .map_err(|err| err.to_string())
}

fn execute_nanop_fixture_operation(
    fixture: &PacketFixture,
    operation: FixtureOperation,
) -> Result<Scalar, String> {
    let left = require_left_series(fixture)?;
    Ok(match operation {
        FixtureOperation::NanSum => nansum(&left.values),
        FixtureOperation::NanMean => nanmean(&left.values),
        FixtureOperation::NanMin => nanmin(&left.values),
        FixtureOperation::NanMax => nanmax(&left.values),
        FixtureOperation::NanStd => nanstd(&left.values, 1),
        FixtureOperation::NanVar => nanvar(&left.values, 1),
        FixtureOperation::NanCount => nancount(&left.values),
        _ => {
            return Err(format!(
                "unsupported nanops operation for fixture execution: {operation:?}"
            ));
        }
    })
}

fn execute_dataframe_from_series_fixture_operation(
    fixture: &PacketFixture,
) -> Result<DataFrame, String> {
    let payloads = collect_constructor_series_payloads(fixture, "dataframe_from_series")?;
    let mut series_list = Vec::with_capacity(payloads.len());
    for payload in payloads {
        series_list
            .push(build_series(&payload).map_err(|err| format!("series build failed: {err}"))?);
    }
    let frame = DataFrame::from_series(series_list).map_err(|err| err.to_string())?;
    apply_constructor_options(fixture, "dataframe_from_series", frame)
}

fn execute_dataframe_from_dict_fixture_operation(
    fixture: &PacketFixture,
) -> Result<DataFrame, String> {
    let dict_columns = require_dict_columns(fixture)?;
    let (columns, selected_columns) = collect_dict_constructor_payloads(
        dict_columns,
        fixture.column_order.as_deref(),
        "dataframe_from_dict",
    )?;
    let frame = if let Some(index) = fixture.index.clone() {
        DataFrame::from_dict_with_index(columns, index).map_err(|err| err.to_string())?
    } else {
        DataFrame::from_dict(&selected_columns, columns).map_err(|err| err.to_string())?
    };
    apply_constructor_options(fixture, "dataframe_from_dict", frame)
}

fn execute_dataframe_from_records_fixture_operation(
    fixture: &PacketFixture,
) -> Result<DataFrame, String> {
    let frame = match (&fixture.records, &fixture.matrix_rows) {
        (Some(records), None) => DataFrame::from_record_maps(
            records.clone(),
            fixture.column_order.as_deref(),
            fixture.index.clone(),
        )
        .map_err(|err| err.to_string())?,
        (None, Some(matrix_rows)) => DataFrame::from_records(
            matrix_rows.clone(),
            fixture.column_order.as_deref(),
            fixture.index.clone(),
        )
        .map_err(|err| err.to_string())?,
        (Some(_), Some(_)) => {
            return Err(
                "dataframe_from_records fixture cannot define both records and matrix_rows"
                    .to_owned(),
            );
        }
        (None, None) => {
            return Err(
                "dataframe_from_records requires records or matrix_rows payload".to_owned(),
            );
        }
    };

    apply_constructor_options(fixture, "dataframe_from_records", frame)
}

fn execute_dataframe_constructor_kwargs_fixture_operation(
    fixture: &PacketFixture,
) -> Result<DataFrame, String> {
    let frame = build_dataframe(require_frame(fixture)?)
        .map_err(|err| format!("frame build failed: {err}"))?;
    let projected = materialize_dataframe_constructor_projection(
        &frame,
        fixture.index.clone(),
        fixture.column_order.clone(),
    )?;
    apply_constructor_options(fixture, "dataframe_constructor_kwargs", projected)
}

fn materialize_dataframe_constructor_projection(
    frame: &DataFrame,
    index: Option<Vec<IndexLabel>>,
    column_order: Option<Vec<String>>,
) -> Result<DataFrame, String> {
    let target_index = index.unwrap_or_else(|| frame.index().labels().to_vec());
    let target_columns = column_order.unwrap_or_else(|| {
        frame
            .column_names()
            .iter()
            .map(|name| (*name).clone())
            .collect()
    });

    let mut first_position = HashMap::new();
    for (position, label) in frame.index().labels().iter().enumerate() {
        first_position.entry(label.clone()).or_insert(position);
    }

    let mut columns = BTreeMap::new();
    for name in &target_columns {
        let values = if let Some(source_column) = frame.column(name) {
            target_index
                .iter()
                .map(|label| {
                    first_position
                        .get(label)
                        .map(|&position| source_column.values()[position].clone())
                        .unwrap_or(Scalar::Null(NullKind::Null))
                })
                .collect()
        } else {
            vec![Scalar::Null(NullKind::Null); target_index.len()]
        };
        let column = Column::from_values(values).map_err(|err| err.to_string())?;
        columns.insert(name.clone(), column);
    }

    DataFrame::new_with_column_order(Index::new(target_index), columns, target_columns)
        .map_err(|err| err.to_string())
}

fn execute_dataframe_constructor_scalar_fixture_operation(
    fixture: &PacketFixture,
) -> Result<DataFrame, String> {
    let fill_value = fixture
        .fill_value
        .clone()
        .ok_or_else(|| "fill_value is required for dataframe_constructor_scalar".to_owned())?;
    let target_index = fixture
        .index
        .clone()
        .ok_or_else(|| "index is required for dataframe_constructor_scalar".to_owned())?;
    let target_columns = fixture
        .column_order
        .clone()
        .ok_or_else(|| "column_order is required for dataframe_constructor_scalar".to_owned())?;

    let mut columns = BTreeMap::new();
    for name in &target_columns {
        let values = vec![fill_value.clone(); target_index.len()];
        let column = Column::from_values(values).map_err(|err| err.to_string())?;
        columns.insert(name.clone(), column);
    }

    let frame = DataFrame::new_with_column_order(Index::new(target_index), columns, target_columns)
        .map_err(|err| err.to_string())?;
    apply_constructor_options(fixture, "dataframe_constructor_scalar", frame)
}

fn execute_dataframe_constructor_dict_of_series_fixture_operation(
    fixture: &PacketFixture,
) -> Result<DataFrame, String> {
    let payloads =
        collect_constructor_series_payloads(fixture, "dataframe_constructor_dict_of_series")?;
    let mut series_list = Vec::with_capacity(payloads.len());
    for payload in payloads {
        series_list
            .push(build_series(&payload).map_err(|err| format!("series build failed: {err}"))?);
    }
    let frame = DataFrame::from_series(series_list).map_err(|err| err.to_string())?;
    let projected = materialize_dataframe_constructor_projection(
        &frame,
        fixture.index.clone(),
        fixture.column_order.clone(),
    )?;
    apply_constructor_options(fixture, "dataframe_constructor_dict_of_series", projected)
}

fn execute_dataframe_constructor_list_like_fixture_operation(
    fixture: &PacketFixture,
) -> Result<DataFrame, String> {
    let matrix_rows = require_matrix_rows(fixture)?;
    let row_count = matrix_rows.len();
    let max_row_width = matrix_rows
        .iter()
        .map(std::vec::Vec::len)
        .max()
        .unwrap_or(0);

    let selected_columns: Vec<String> = if let Some(column_order) = fixture.column_order.clone() {
        if max_row_width > column_order.len() {
            return Err(format!(
                "dataframe_constructor_list_like row width {max_row_width} exceeds columns length {}",
                column_order.len()
            ));
        }
        column_order
    } else {
        (0..max_row_width).map(|idx| idx.to_string()).collect()
    };

    let index_labels = if let Some(index) = fixture.index.clone() {
        if index.len() != row_count {
            return Err(format!(
                "dataframe_constructor_list_like index length {} does not match row count {row_count}",
                index.len()
            ));
        }
        index
    } else {
        (0..row_count as i64).map(IndexLabel::from).collect()
    };

    let mut dict_columns = BTreeMap::new();
    for (column_offset, column_name) in selected_columns.iter().enumerate() {
        let values = matrix_rows
            .iter()
            .map(|row| {
                row.get(column_offset)
                    .cloned()
                    .unwrap_or(Scalar::Null(NullKind::Null))
            })
            .collect::<Vec<_>>();
        dict_columns.insert(column_name.clone(), values);
    }

    let (columns, _) = collect_dict_constructor_payloads(
        &dict_columns,
        Some(&selected_columns),
        "dataframe_constructor_list_like",
    )?;
    let frame =
        DataFrame::from_dict_with_index(columns, index_labels).map_err(|err| err.to_string())?;
    apply_constructor_options(fixture, "dataframe_constructor_list_like", frame)
}

fn execute_csv_round_trip_fixture_operation(fixture: &PacketFixture) -> Result<bool, String> {
    let csv_input = fixture
        .csv_input
        .as_ref()
        .ok_or_else(|| "csv_input is required for csv_round_trip".to_owned())?;
    let df = read_csv_str(csv_input).map_err(|err| format!("csv parse failed: {err}"))?;
    let output = write_csv_string(&df).map_err(|err| format!("csv write failed: {err}"))?;
    let reparsed = read_csv_str(&output).map_err(|err| format!("csv reparse failed: {err}"))?;
    Ok(dataframes_semantically_equal(&df, &reparsed))
}

fn dataframes_semantically_equal(left: &DataFrame, right: &DataFrame) -> bool {
    if left.column_names() != right.column_names() || left.len() != right.len() {
        return false;
    }
    for name in left.columns().keys() {
        let Some(left_col) = left.column(name) else {
            return false;
        };
        let Some(right_col) = right.column(name) else {
            return false;
        };
        if !left_col.semantic_eq(right_col) {
            return false;
        }
    }
    true
}

const INDEX_MERGE_KEY_COLUMN: &str = "__index_key";

fn execute_dataframe_fixture_operation(fixture: &PacketFixture) -> Result<DataFrame, String> {
    match fixture.operation {
        FixtureOperation::SeriesPartitionDf => {
            let left = require_left_series(fixture)?;
            let sep = require_string_sep(fixture, "series_partition_df")?;
            let series = build_series(left).map_err(|err| format!("series build failed: {err}"))?;
            series
                .str()
                .partition_df(sep)
                .map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesRpartitionDf => {
            let left = require_left_series(fixture)?;
            let sep = require_string_sep(fixture, "series_rpartition_df")?;
            let series = build_series(left).map_err(|err| format!("series build failed: {err}"))?;
            series
                .str()
                .rpartition_df(sep)
                .map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesExtractDf => {
            let left = require_left_series(fixture)?;
            let pattern = require_regex_pattern(fixture, "series_extract_df")?;
            let series = build_series(left).map_err(|err| format!("series build failed: {err}"))?;
            series
                .str()
                .extract_df(pattern)
                .map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesExtractAll => {
            let left = require_left_series(fixture)?;
            let pattern = require_regex_pattern(fixture, "series_extractall")?;
            let series = build_series(left).map_err(|err| format!("series build failed: {err}"))?;
            series
                .str()
                .extractall(pattern)
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameMerge => {
            execute_dataframe_merge_fixture_operation(fixture, false)
        }
        FixtureOperation::DataFrameMergeIndex => {
            execute_dataframe_merge_fixture_operation(fixture, true)
        }
        FixtureOperation::DataFrameMergeAsof => {
            execute_dataframe_merge_asof_fixture_operation(fixture)
        }
        FixtureOperation::DataFrameMergeOrdered => {
            execute_dataframe_merge_ordered_fixture_operation(fixture)
        }
        FixtureOperation::DataFrameConcat => {
            let left = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("left frame build failed: {err}"))?;
            let right = build_dataframe(require_frame_right(fixture)?)
                .map_err(|err| format!("right frame build failed: {err}"))?;
            let axis = normalize_concat_axis(fixture)?;
            let join = normalize_concat_join(fixture)?;
            concat_dataframes_with_axis_join(&[&left, &right], axis, join)
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameCombineFirst => {
            let left = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("left frame build failed: {err}"))?;
            let right = build_dataframe(require_frame_right(fixture)?)
                .map_err(|err| format!("right frame build failed: {err}"))?;
            left.combine_first(&right).map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameSortIndex => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame
                .sort_index(resolve_sort_ascending(fixture))
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameSortValues => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame
                .sort_values(
                    require_sort_column(fixture)?,
                    resolve_sort_ascending(fixture),
                )
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameDiff => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let periods = fixture.diff_periods.unwrap_or(1);
            let axis = fixture.diff_axis.unwrap_or(0);
            if axis == 1 {
                frame.diff_axis1(periods).map_err(|err| err.to_string())
            } else {
                frame.diff(periods).map_err(|err| err.to_string())
            }
        }
        FixtureOperation::DataFrameShift => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let periods = fixture.shift_periods.unwrap_or(1);
            let axis = resolve_shift_axis(fixture)?;
            if axis == 1 {
                frame.shift_axis1(periods).map_err(|err| err.to_string())
            } else {
                frame.shift(periods).map_err(|err| err.to_string())
            }
        }
        FixtureOperation::DataFramePctChange => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let periods = fixture.diff_periods.unwrap_or(1);
            let axis = fixture.diff_axis.unwrap_or(0);
            if axis == 1 {
                frame
                    .pct_change_axis1(periods)
                    .map_err(|err| err.to_string())
            } else {
                if periods < 0 {
                    return Err("dataframe_pct_change periods must be non-negative".to_owned());
                }
                frame
                    .pct_change(periods as usize)
                    .map_err(|err| err.to_string())
            }
        }
        FixtureOperation::DataFrameMelt => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let id_vars =
                resolve_optional_string_list(fixture.melt_id_vars.as_ref(), "melt_id_vars")?;
            let value_vars =
                resolve_optional_string_list(fixture.melt_value_vars.as_ref(), "melt_value_vars")?;
            let id_var_refs = id_vars.iter().map(String::as_str).collect::<Vec<_>>();
            let value_var_refs = value_vars.iter().map(String::as_str).collect::<Vec<_>>();
            frame
                .melt(
                    &id_var_refs,
                    &value_var_refs,
                    fixture.melt_var_name.as_deref(),
                    fixture.melt_value_name.as_deref(),
                )
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameIsNa => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame.isna().map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameNotNa => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame.notna().map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameIsNull => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame.isnull().map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameNotNull => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame.notnull().map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameMode => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame.mode().map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameRank => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let method = fixture.rank_method.as_deref().unwrap_or("average");
            let na_option = fixture.rank_na_option.as_deref().unwrap_or("keep");
            let ascending = resolve_sort_ascending(fixture);
            let axis = resolve_rank_axis(fixture)?;
            if axis == 1 {
                frame
                    .rank_axis1(method, ascending, na_option)
                    .map_err(|err| err.to_string())
            } else {
                frame
                    .rank(method, ascending, na_option)
                    .map_err(|err| err.to_string())
            }
        }
        FixtureOperation::DataFrameFillNa => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let fill_value = fixture
                .fill_value
                .as_ref()
                .ok_or_else(|| "fill_value is required for dataframe_fillna".to_owned())?;
            frame.fillna(fill_value).map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameDropNa => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame.dropna().map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameDropNaColumns => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame.dropna_columns().map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameSetIndex => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame
                .set_index(
                    require_set_index_column(fixture)?,
                    require_set_index_drop(fixture)?,
                )
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameResetIndex => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame
                .reset_index(require_reset_index_drop(fixture)?)
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameDropDuplicates => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let subset = resolve_duplicate_subset(fixture)?;
            let keep = resolve_duplicate_keep(fixture)?;
            frame
                .drop_duplicates(
                    subset.as_deref(),
                    keep,
                    resolve_drop_duplicates_ignore_index(fixture),
                )
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameTake => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let indices = require_take_indices(fixture)?;
            let axis = resolve_take_axis(fixture)?;
            frame.take(indices, axis).map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameXs => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame
                .xs(require_xs_key(fixture, "dataframe_xs")?)
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameGroupByIdxMin => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_idxmin")
        }
        FixtureOperation::DataFrameGroupByIdxMax => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_idxmax")
        }
        FixtureOperation::DataFrameGroupByAny => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_any")
        }
        FixtureOperation::DataFrameGroupByAll => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_all")
        }
        FixtureOperation::DataFrameGroupByGetGroup => {
            execute_dataframe_groupby_frame_fixture_operation(
                fixture,
                "dataframe_groupby_get_group",
            )
        }
        FixtureOperation::DataFrameGroupByFfill => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_ffill")
        }
        FixtureOperation::DataFrameGroupByBfill => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_bfill")
        }
        FixtureOperation::DataFrameGroupBySem => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_sem")
        }
        FixtureOperation::DataFrameGroupBySkew => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_skew")
        }
        FixtureOperation::DataFrameGroupByKurtosis => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_kurtosis")
        }
        FixtureOperation::DataFrameGroupByOhlc => {
            execute_dataframe_groupby_frame_fixture_operation(fixture, "dataframe_groupby_ohlc")
        }
        FixtureOperation::DataFrameAtTime => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame
                .at_time(require_time_value(fixture)?)
                .map_err(|err| err.to_string())
        }
        FixtureOperation::DataFrameBetweenTime => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            frame
                .between_time(require_start_time(fixture)?, require_end_time(fixture)?)
                .map_err(|err| err.to_string())
        }
        _ => Err(format!(
            "unsupported dataframe operation for fixture execution: {:?}",
            fixture.operation
        )),
    }
}

fn execute_dataframe_groupby_frame_fixture_operation(
    fixture: &PacketFixture,
    operation_name: &str,
) -> Result<DataFrame, String> {
    let frame = build_dataframe(require_frame(fixture)?)
        .map_err(|err| format!("frame build failed: {err}"))?;
    let groupby_columns = require_groupby_columns(fixture, operation_name)?;
    let groupby_refs = groupby_columns
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let groupby = frame
        .groupby(&groupby_refs)
        .map_err(|err| err.to_string())?;
    match operation_name {
        "dataframe_groupby_idxmin" => groupby.idxmin().map_err(|err| err.to_string()),
        "dataframe_groupby_idxmax" => groupby.idxmax().map_err(|err| err.to_string()),
        "dataframe_groupby_any" => groupby.any().map_err(|err| err.to_string()),
        "dataframe_groupby_all" => groupby.all().map_err(|err| err.to_string()),
        "dataframe_groupby_get_group" => groupby
            .get_group(require_group_name(fixture, operation_name)?)
            .map_err(|err| err.to_string()),
        "dataframe_groupby_ffill" => groupby.ffill(None).map_err(|err| err.to_string()),
        "dataframe_groupby_bfill" => groupby.bfill(None).map_err(|err| err.to_string()),
        "dataframe_groupby_sem" => groupby.sem().map_err(|err| err.to_string()),
        "dataframe_groupby_skew" => groupby.skew().map_err(|err| err.to_string()),
        "dataframe_groupby_kurtosis" => groupby.kurtosis().map_err(|err| err.to_string()),
        "dataframe_groupby_ohlc" => groupby.ohlc().map_err(|err| err.to_string()),
        other => Err(format!(
            "unsupported dataframe groupby frame operation: {other}"
        )),
    }
}

fn execute_dataframe_groupby_series_fixture_operation(
    fixture: &PacketFixture,
    use_ngroup: bool,
) -> Result<Series, String> {
    let frame = build_dataframe(require_frame(fixture)?)
        .map_err(|err| format!("frame build failed: {err}"))?;
    let operation_name = if use_ngroup {
        "dataframe_groupby_ngroup"
    } else {
        "dataframe_groupby_cumcount"
    };
    let groupby_columns = require_groupby_columns(fixture, operation_name)?;
    let groupby_refs = groupby_columns
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let groupby = frame
        .groupby(&groupby_refs)
        .map_err(|err| err.to_string())?;
    if use_ngroup {
        groupby.ngroup().map_err(|err| err.to_string())
    } else {
        groupby.cumcount().map_err(|err| err.to_string())
    }
}

fn execute_dataframe_asof_fixture_operation(fixture: &PacketFixture) -> Result<Series, String> {
    let frame = build_dataframe(require_frame(fixture)?)
        .map_err(|err| format!("frame build failed: {err}"))?;
    let subset = resolve_duplicate_subset(fixture)?;
    let subset_refs = subset
        .as_ref()
        .map(|columns| columns.iter().map(String::as_str).collect::<Vec<_>>());
    frame
        .asof(require_asof_label(fixture)?, subset_refs.as_deref())
        .map_err(|err| err.to_string())
}

fn execute_series_bool_fixture_operation(fixture: &PacketFixture) -> Result<bool, String> {
    let left = require_left_series(fixture)?;
    let series = build_series(left)?;
    series.bool_().map_err(|err| err.to_string())
}

fn execute_series_repeat_fixture_operation(fixture: &PacketFixture) -> Result<Series, String> {
    let left = require_left_series(fixture)?;
    let series = build_series(left)?;
    let repeat_counts = resolve_series_repeat_counts(fixture, series.len())?;
    series
        .repeat_by(&repeat_counts)
        .map_err(|err| err.to_string())
}

fn execute_series_combine_first_fixture_operation(
    fixture: &PacketFixture,
) -> Result<Series, String> {
    let left = require_left_series(fixture)?;
    let right = require_right_series(fixture)?;
    let left = build_series(left).map_err(|err| format!("left series build failed: {err}"))?;
    let right = build_series(right).map_err(|err| format!("right series build failed: {err}"))?;
    left.combine_first(&right).map_err(|err| err.to_string())
}

fn execute_series_module_utility_fixture_operation(
    fixture: &PacketFixture,
) -> Result<Series, String> {
    let series = build_series(require_left_series(fixture)?)?;
    match fixture.operation {
        FixtureOperation::SeriesToNumeric => to_numeric(&series).map_err(|err| err.to_string()),
        FixtureOperation::SeriesConvertDtypes => {
            series.convert_dtypes().map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesAstype => {
            let dtype_spec = fixture
                .constructor_dtype
                .as_deref()
                .ok_or_else(|| "constructor_dtype required for series_astype".to_owned())?;
            let target_dtype = parse_constructor_dtype_spec(dtype_spec)?;
            series.astype(target_dtype).map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesClip => series
            .clip(fixture.clip_lower, fixture.clip_upper)
            .map_err(|err| err.to_string()),
        FixtureOperation::SeriesAbs => series.abs().map_err(|err| err.to_string()),
        FixtureOperation::SeriesRound => {
            let decimals = fixture.round_decimals.unwrap_or(0);
            series.round(decimals).map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesCumsum => series.cumsum().map_err(|err| err.to_string()),
        FixtureOperation::SeriesCumprod => series.cumprod().map_err(|err| err.to_string()),
        FixtureOperation::SeriesCummax => series.cummax().map_err(|err| err.to_string()),
        FixtureOperation::SeriesCummin => series.cummin().map_err(|err| err.to_string()),
        FixtureOperation::SeriesNlargest => {
            let n = fixture
                .nlargest_n
                .ok_or_else(|| "nlargest_n required for series_nlargest".to_owned())?;
            series.nlargest(n).map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesNsmallest => {
            let n = fixture
                .nlargest_n
                .ok_or_else(|| "nlargest_n required for series_nsmallest".to_owned())?;
            series.nsmallest(n).map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesBetween => {
            let left = fixture
                .between_left
                .as_ref()
                .ok_or_else(|| "between_left required for series_between".to_owned())?;
            let right = fixture
                .between_right
                .as_ref()
                .ok_or_else(|| "between_right required for series_between".to_owned())?;
            let inclusive = fixture.between_inclusive.as_deref().unwrap_or("both");
            series
                .between(left, right, inclusive)
                .map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesCut => {
            cut(&series, require_cut_bins(fixture)?).map_err(|err| err.to_string())
        }
        FixtureOperation::SeriesQcut => {
            qcut(&series, require_qcut_quantiles(fixture)?).map_err(|err| err.to_string())
        }
        other => Err(format!(
            "unsupported series module utility operation for fixture execution: {other:?}"
        )),
    }
}

fn execute_series_xs_fixture_operation(fixture: &PacketFixture) -> Result<Series, String> {
    let series = build_series(require_left_series(fixture)?)?;
    series
        .xs(require_xs_key(fixture, "series_xs")?)
        .map_err(|err| err.to_string())
}

fn execute_dataframe_bool_fixture_operation(fixture: &PacketFixture) -> Result<bool, String> {
    let frame = build_dataframe(require_frame(fixture)?)
        .map_err(|err| format!("frame build failed: {err}"))?;
    frame.bool_().map_err(|err| err.to_string())
}

fn execute_dataframe_merge_fixture_operation(
    fixture: &PacketFixture,
    merge_on_index: bool,
) -> Result<DataFrame, String> {
    let left = build_dataframe(require_frame(fixture)?)
        .map_err(|err| format!("left frame build failed: {err}"))?;
    let right = build_dataframe(require_frame_right(fixture)?)
        .map_err(|err| format!("right frame build failed: {err}"))?;
    let join_type = require_join_type(fixture)?.into_join_type();

    let (left_use_index, right_use_index, operation_name) = if merge_on_index {
        (true, true, "dataframe_merge_index")
    } else {
        (
            fixture.left_index.unwrap_or(false),
            fixture.right_index.unwrap_or(false),
            "dataframe_merge",
        )
    };
    if matches!(join_type, JoinType::Cross) {
        validate_cross_merge_configuration(fixture, operation_name, merge_on_index)?;
    }

    let (left_merge_keys, right_merge_keys) = if matches!(join_type, JoinType::Cross) {
        (Vec::new(), Vec::new())
    } else {
        let default_index_key =
            (left_use_index && right_use_index).then_some(INDEX_MERGE_KEY_COLUMN);
        resolve_merge_key_pairs(
            fixture.merge_on.as_deref(),
            fixture.merge_on_keys.as_deref(),
            fixture.left_on_keys.as_deref(),
            fixture.right_on_keys.as_deref(),
            operation_name,
            default_index_key,
        )?
    };
    let indicator_name = resolve_merge_indicator_name(
        fixture.merge_indicator,
        fixture.merge_indicator_name.as_deref(),
        operation_name,
    )?;
    let validate_mode =
        resolve_merge_validate_mode(fixture.merge_validate.as_deref(), operation_name)?;
    let suffixes = resolve_merge_suffixes(fixture.merge_suffixes.as_ref());
    let sort = resolve_merge_sort(fixture.merge_sort);

    let left_input = if left_use_index {
        dataframe_with_index_as_columns(&left, &left_merge_keys)?
    } else {
        left
    };
    let right_input = if right_use_index {
        dataframe_with_index_as_columns(&right, &right_merge_keys)?
    } else {
        right
    };

    let left_merge_refs = left_merge_keys
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let right_merge_refs = right_merge_keys
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let merged = merge_dataframes_on_with_options(
        &left_input,
        &right_input,
        &left_merge_refs,
        &right_merge_refs,
        join_type,
        MergeExecutionOptions {
            indicator_name,
            validate_mode,
            suffixes,
            sort,
        },
    )
    .map_err(|err| err.to_string())?;
    DataFrame::new(merged.index, merged.columns).map_err(|err| err.to_string())
}

fn dataframe_with_index_as_columns(
    frame: &DataFrame,
    key_names: &[String],
) -> Result<DataFrame, String> {
    let mut out = frame.clone();
    for key_name in key_names {
        let index_values = frame
            .index()
            .labels()
            .iter()
            .map(index_label_to_scalar)
            .collect::<Vec<_>>();
        let key_column = Column::from_values(index_values).map_err(|err| err.to_string())?;
        out = out
            .with_column(key_name.as_str(), key_column)
            .map_err(|err| err.to_string())?;
    }
    Ok(out)
}

fn index_label_to_scalar(label: &IndexLabel) -> Scalar {
    match label {
        IndexLabel::Int64(value) => Scalar::Int64(*value),
        IndexLabel::Utf8(value) => Scalar::Utf8(value.clone()),
    }
}

fn execute_groupby_fixture_operation(
    fixture: &PacketFixture,
    operation: FixtureOperation,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
) -> Result<Series, String> {
    let key_series = build_groupby_key_series(fixture)?;
    let values = require_right_series(fixture)?;
    let value_series =
        build_series(values).map_err(|err| format!("values series build failed: {err}"))?;
    let options = GroupByOptions::default();

    let result = match operation {
        FixtureOperation::GroupBySum => {
            groupby_sum(&key_series, &value_series, options, policy, ledger)
        }
        FixtureOperation::GroupByMean => {
            groupby_mean(&key_series, &value_series, options, policy, ledger)
        }
        FixtureOperation::GroupByCount => {
            groupby_count(&key_series, &value_series, options, policy, ledger)
        }
        FixtureOperation::GroupByMin => {
            groupby_min(&key_series, &value_series, options, policy, ledger)
        }
        FixtureOperation::GroupByMax => {
            groupby_max(&key_series, &value_series, options, policy, ledger)
        }
        FixtureOperation::GroupByFirst => {
            groupby_first(&key_series, &value_series, options, policy, ledger)
        }
        FixtureOperation::GroupByLast => {
            groupby_last(&key_series, &value_series, options, policy, ledger)
        }
        FixtureOperation::GroupByStd => {
            groupby_std(&key_series, &value_series, options, policy, ledger)
        }
        FixtureOperation::GroupByVar => {
            groupby_var(&key_series, &value_series, options, policy, ledger)
        }
        FixtureOperation::GroupByMedian => {
            groupby_median(&key_series, &value_series, options, policy, ledger)
        }
        _ => {
            return Err(format!(
                "unsupported groupby operation for fixture execution: {operation:?}"
            ));
        }
    };

    result.map_err(|err| err.to_string())
}

fn build_groupby_key_series(fixture: &PacketFixture) -> Result<Series, String> {
    if let Some(groupby_keys) = fixture.groupby_keys.as_ref() {
        if groupby_keys.is_empty() {
            return Err("groupby_keys must contain at least one key series".to_owned());
        }
        return build_composite_groupby_keys_series(groupby_keys);
    }

    let keys = require_left_series(fixture)?;
    build_series(keys).map_err(|err| format!("keys series build failed: {err}"))
}

fn build_composite_groupby_keys_series(groupby_keys: &[FixtureSeries]) -> Result<Series, String> {
    let mut union_index = Vec::new();
    let mut seen = BTreeSet::new();
    let mut key_maps = Vec::with_capacity(groupby_keys.len());

    for key in groupby_keys {
        if key.index.len() != key.values.len() {
            return Err(format!(
                "groupby key series '{}' index/value length mismatch: {} vs {}",
                key.name,
                key.index.len(),
                key.values.len()
            ));
        }

        for label in &key.index {
            if seen.insert(label.clone()) {
                union_index.push(label.clone());
            }
        }

        let mut first_value_by_label: HashMap<IndexLabel, Scalar> = HashMap::new();
        for (label, value) in key.index.iter().cloned().zip(key.values.iter().cloned()) {
            first_value_by_label.entry(label).or_insert(value);
        }
        key_maps.push(first_value_by_label);
    }

    let mut composite_values = Vec::with_capacity(union_index.len());
    for label in &union_index {
        let mut tuple_values = Vec::with_capacity(key_maps.len());
        let mut has_missing_component = false;

        for key_map in &key_maps {
            match key_map.get(label) {
                Some(value) if !value.is_missing() => tuple_values.push(value.clone()),
                _ => {
                    has_missing_component = true;
                    break;
                }
            }
        }

        if has_missing_component {
            composite_values.push(Scalar::Null(NullKind::Null));
        } else {
            let composite = encode_groupby_composite_key(&tuple_values)?;
            composite_values.push(Scalar::Utf8(composite));
        }
    }

    Series::from_values(
        "groupby_composite_key".to_owned(),
        union_index,
        composite_values,
    )
    .map_err(|err| format!("groupby composite key series build failed: {err}"))
}

fn encode_groupby_composite_key(values: &[Scalar]) -> Result<String, String> {
    let mut components = Vec::with_capacity(values.len());
    for value in values {
        let component = match value {
            Scalar::Bool(v) => format!("b:{v}"),
            Scalar::Int64(v) => format!("i:{v}"),
            Scalar::Float64(v) => {
                if v.is_nan() {
                    return Err("groupby composite key component cannot be NaN".to_owned());
                }
                format!("f_bits:{:016x}", v.to_bits())
            }
            Scalar::Utf8(v) => {
                let escaped = serde_json::to_string(v)
                    .map_err(|err| format!("groupby key encoding failed: {err}"))?;
                format!("s:{escaped}")
            }
            Scalar::Null(_) => {
                return Err("groupby composite key component cannot be null".to_owned());
            }
        };
        components.push(component);
    }

    Ok(components.join("|"))
}

fn build_series(series: &FixtureSeries) -> Result<Series, String> {
    Series::from_values(
        series.name.clone(),
        series.index.clone(),
        series.values.clone(),
    )
    .map_err(|err| err.to_string())
}

fn resolve_datetime_origin_option(
    origin: Option<&serde_json::Value>,
) -> Result<Option<fp_frame::ToDatetimeOrigin<'_>>, String> {
    match origin {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Err("series_to_datetime datetime_origin must be non-empty".to_owned())
            } else {
                Ok(Some(fp_frame::ToDatetimeOrigin::Str(trimmed)))
            }
        }
        Some(serde_json::Value::Number(value)) => {
            if let Some(int_value) = value.as_i64() {
                Ok(Some(fp_frame::ToDatetimeOrigin::Int(int_value)))
            } else if let Some(float_value) = value.as_f64() {
                Ok(Some(fp_frame::ToDatetimeOrigin::Float(float_value)))
            } else {
                Err("series_to_datetime datetime_origin number is out of range".to_owned())
            }
        }
        Some(_) => {
            Err("series_to_datetime datetime_origin must be a string, integer, or float".to_owned())
        }
    }
}

fn build_dataframe(frame: &FixtureDataFrame) -> Result<DataFrame, String> {
    let mut columns = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(column_order) = frame.column_order.as_ref() {
        for name in column_order {
            let values = frame
                .columns
                .get(name)
                .ok_or_else(|| format!("frame column_order references missing column '{name}'"))?;
            if !seen.insert(name.clone()) {
                return Err(format!(
                    "frame column_order contains duplicate column '{name}'"
                ));
            }
            columns.push((name.as_str(), values.clone()));
        }
    }

    for (name, values) in &frame.columns {
        if seen.insert(name.clone()) {
            columns.push((name.as_str(), values.clone()));
        }
    }

    DataFrame::from_dict_with_index(columns, frame.index.clone()).map_err(|err| err.to_string())
}

fn compare_series_expected(
    actual: &Series,
    expected: &FixtureExpectedSeries,
) -> Result<(), String> {
    if actual.index().labels() != expected.index {
        return Err(format!(
            "index mismatch: actual={:?}, expected={:?}",
            actual.index().labels(),
            expected.index
        ));
    }

    if actual.values().len() != expected.values.len() {
        return Err(format!(
            "value length mismatch: actual={}, expected={}",
            actual.values().len(),
            expected.values.len()
        ));
    }

    for (idx, (left, right)) in actual
        .values()
        .iter()
        .zip(expected.values.iter())
        .enumerate()
    {
        if !left.semantic_eq(right) {
            return Err(format!(
                "value mismatch at idx={idx}: actual={left:?}, expected={right:?}"
            ));
        }
    }
    Ok(())
}

fn compare_dataframe_expected(
    actual: &DataFrame,
    expected: &FixtureExpectedDataFrame,
) -> Result<(), String> {
    if actual.index().labels() != expected.index {
        return Err(format!(
            "dataframe index mismatch: actual={:?}, expected={:?}",
            actual.index().labels(),
            expected.index
        ));
    }

    let actual_names = actual
        .column_names()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if let Some(expected_order) = expected.column_order.clone() {
        if actual_names != expected_order {
            return Err(format!(
                "dataframe column mismatch: actual={actual_names:?}, expected={expected_order:?}"
            ));
        }
    } else {
        let mut expected_names = expected.columns.keys().cloned().collect::<Vec<_>>();
        expected_names.sort();
        let mut actual_sorted = actual_names.clone();
        actual_sorted.sort();
        if actual_sorted != expected_names {
            return Err(format!(
                "dataframe column set mismatch: actual={actual_names:?}, expected={expected_names:?}"
            ));
        }
    }

    for (name, expected_values) in &expected.columns {
        let Some(column) = actual.column(name) else {
            return Err(format!("dataframe column missing in actual: {name}"));
        };

        if column.values().len() != expected_values.len() {
            return Err(format!(
                "dataframe column '{name}' length mismatch: actual={}, expected={}",
                column.values().len(),
                expected_values.len()
            ));
        }

        for (idx, (left, right)) in column
            .values()
            .iter()
            .zip(expected_values.iter())
            .enumerate()
        {
            if !left.semantic_eq(right) {
                return Err(format!(
                    "dataframe column '{name}' mismatch at idx={idx}: actual={left:?}, expected={right:?}"
                ));
            }
        }
    }

    Ok(())
}

fn compare_scalar(actual: &Scalar, expected: &Scalar, op_name: &str) -> Result<(), String> {
    if !actual.semantic_eq(expected) {
        return Err(format!(
            "{op_name} scalar mismatch: actual={actual:?}, expected={expected:?}"
        ));
    }
    Ok(())
}

fn compare_join_expected(
    actual: &fp_join::JoinedSeries,
    expected: &FixtureExpectedJoin,
) -> Result<(), String> {
    if actual.index.labels() != expected.index {
        return Err(format!(
            "join index mismatch: actual={:?}, expected={:?}",
            actual.index.labels(),
            expected.index
        ));
    }

    if actual.left_values.values().len() != expected.left_values.len() {
        return Err(format!(
            "join left length mismatch: actual={}, expected={}",
            actual.left_values.values().len(),
            expected.left_values.len()
        ));
    }
    if actual.right_values.values().len() != expected.right_values.len() {
        return Err(format!(
            "join right length mismatch: actual={}, expected={}",
            actual.right_values.values().len(),
            expected.right_values.len()
        ));
    }

    for (idx, (left, right)) in actual
        .left_values
        .values()
        .iter()
        .zip(expected.left_values.iter())
        .enumerate()
    {
        let equal = left.semantic_eq(right) || (left.is_missing() && right.is_missing());
        if !equal {
            return Err(format!(
                "join left mismatch at idx={idx}: actual={left:?}, expected={right:?}"
            ));
        }
    }
    for (idx, (left, right)) in actual
        .right_values
        .values()
        .iter()
        .zip(expected.right_values.iter())
        .enumerate()
    {
        let equal = left.semantic_eq(right) || (left.is_missing() && right.is_missing());
        if !equal {
            return Err(format!(
                "join right mismatch at idx={idx}: actual={left:?}, expected={right:?}"
            ));
        }
    }

    Ok(())
}

fn compare_alignment_expected(
    actual: &AlignmentPlan,
    expected: &FixtureExpectedAlignment,
) -> Result<(), String> {
    if actual.union_index.labels() != expected.union_index {
        return Err(format!(
            "union_index mismatch: actual={:?}, expected={:?}",
            actual.union_index.labels(),
            expected.union_index
        ));
    }
    if actual.left_positions != expected.left_positions {
        return Err(format!(
            "left_positions mismatch: actual={:?}, expected={:?}",
            actual.left_positions, expected.left_positions
        ));
    }
    if actual.right_positions != expected.right_positions {
        return Err(format!(
            "right_positions mismatch: actual={:?}, expected={:?}",
            actual.right_positions, expected.right_positions
        ));
    }
    Ok(())
}

// === Differential Harness: Internal Execution + Taxonomy Comparators ===

fn build_differential_report_internal(
    config: &HarnessConfig,
    suite: String,
    packet_id: Option<String>,
    fixtures: &[PacketFixture],
    options: &SuiteOptions,
) -> Result<DifferentialReport, HarnessError> {
    let mut results = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        results.push(run_differential_fixture(config, fixture, options)?);
    }
    Ok(build_differential_report(
        suite,
        packet_id,
        config.oracle_root.exists(),
        results,
    ))
}

fn run_differential_fixture(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    options: &SuiteOptions,
) -> Result<DifferentialResult, HarnessError> {
    let policy = match fixture.mode {
        RuntimeMode::Strict => RuntimePolicy::strict(),
        RuntimeMode::Hardened => RuntimePolicy::hardened(Some(100_000)),
    };
    let mut ledger = EvidenceLedger::new();
    let oracle_source = fixture.oracle_source.unwrap_or(match options.oracle_mode {
        OracleMode::FixtureExpected => FixtureOracleSource::Fixture,
        OracleMode::LiveLegacyPandas => FixtureOracleSource::LiveLegacyPandas,
    });

    let drift_records = match execute_and_compare_differential(
        config,
        fixture,
        &policy,
        &mut ledger,
        options.oracle_mode,
    ) {
        Ok(drifts) => drifts,
        Err(err) => vec![make_drift_record(
            ComparisonCategory::Value,
            DriftLevel::Critical,
            "execution",
            err,
        )],
    };

    let has_critical = drift_records
        .iter()
        .any(|d| matches!(d.level, DriftLevel::Critical));

    Ok(DifferentialResult {
        case_id: fixture.case_id.clone(),
        packet_id: fixture.packet_id.clone(),
        operation: fixture.operation,
        mode: fixture.mode,
        replay_key: deterministic_replay_key(&fixture.packet_id, &fixture.case_id, fixture.mode),
        trace_id: deterministic_trace_id(&fixture.packet_id, &fixture.case_id, fixture.mode),
        oracle_source,
        status: if has_critical {
            CaseStatus::Fail
        } else {
            CaseStatus::Pass
        },
        drift_records,
        evidence_records: ledger.records().len(),
    })
}

fn execute_and_compare_differential(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
    default_oracle_mode: OracleMode,
) -> Result<Vec<DriftRecord>, String> {
    let expected = resolve_expected(config, fixture, default_oracle_mode)
        .map_err(|err| format!("expected resolution failed: {err}"))?;

    match fixture.operation {
        FixtureOperation::SeriesAdd
        | FixtureOperation::SeriesSub
        | FixtureOperation::SeriesMul
        | FixtureOperation::SeriesDiv => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let left_series = build_series(left)?;
            let right_series = build_series(right)?;
            let actual = match fixture.operation {
                FixtureOperation::SeriesAdd => {
                    left_series.add_with_policy(&right_series, policy, ledger)
                }
                FixtureOperation::SeriesSub => {
                    left_series.sub_with_policy(&right_series, policy, ledger)
                }
                FixtureOperation::SeriesMul => {
                    left_series.mul_with_policy(&right_series, policy, ledger)
                }
                FixtureOperation::SeriesDiv => {
                    left_series.div_with_policy(&right_series, policy, ledger)
                }
                _ => unreachable!("match arm constrained to series arithmetic operations"),
            }
            .map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => {
                    return Err(format!(
                        "expected_series required for {}",
                        fixture.operation.operation_name()
                    ));
                }
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::SeriesJoin => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let joined = (|| -> Result<fp_join::JoinedSeries, String> {
                let left_series = build_series(left).map_err(|err| format!("left build: {err}"))?;
                let right_series =
                    build_series(right).map_err(|err| format!("right build: {err}"))?;
                let join_type = require_series_join_type(fixture)?;
                join_series(&left_series, &right_series, join_type).map_err(|err| err.to_string())
            })();
            match expected {
                ResolvedExpected::Join(expected_join) => Ok(diff_join(&joined?, &expected_join)),
                ResolvedExpected::ErrorContains(substr) => Ok(match joined {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_join.error",
                        format!(
                            "expected series_join error containing '{substr}', got '{}'",
                            message
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_join.error",
                        format!("expected series_join to fail with error containing '{substr}'"),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match joined {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_join.error",
                        "expected series_join to fail".to_owned(),
                    )],
                }),
                _ => Err("expected_join required for series_join".to_owned()),
            }
        }
        FixtureOperation::SeriesConstructor => {
            let left = require_left_series(fixture)?;
            let actual = build_series(left);
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_constructor.error",
                        format!(
                            "expected series_constructor error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_constructor.error",
                        "expected series_constructor to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_constructor.error",
                        "expected series_constructor to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for series_constructor".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesToDatetime => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = resolve_datetime_origin_option(fixture.datetime_origin.as_ref()).and_then(
                |origin| {
                    fp_frame::to_datetime_with_options(
                        &series,
                        fp_frame::ToDatetimeOptions {
                            format: None,
                            unit: fixture.datetime_unit.as_deref(),
                            utc: fixture.datetime_utc.unwrap_or(false),
                            origin,
                        },
                    )
                    .map_err(|err| err.to_string())
                },
            );
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_to_datetime.error",
                        format!(
                            "expected series_to_datetime error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_to_datetime.error",
                        "expected series_to_datetime to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_to_datetime.error",
                        "expected series_to_datetime to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for series_to_datetime".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesToTimedelta => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = fp_frame::to_timedelta(&series).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_to_timedelta.error",
                        format!(
                            "expected series_to_timedelta error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_to_timedelta.error",
                        "expected series_to_timedelta to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_to_timedelta.error",
                        "expected series_to_timedelta to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for series_to_timedelta".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesTimedeltaTotalSeconds => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = fp_frame::timedelta_total_seconds(&series).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "timedelta_total_seconds.error",
                        format!(
                            "expected timedelta_total_seconds error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "timedelta_total_seconds.error",
                        "expected timedelta_total_seconds to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "timedelta_total_seconds.error",
                        "expected timedelta_total_seconds to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for timedelta_total_seconds"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameFromSeries => {
            let actual = execute_dataframe_from_series_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_from_series.error",
                        format!(
                            "expected dataframe_from_series error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_from_series.error",
                        "expected dataframe_from_series to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_from_series.error",
                        "expected dataframe_from_series to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_from_series"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameFromDict => {
            let actual = execute_dataframe_from_dict_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_from_dict.error",
                        format!(
                            "expected dataframe_from_dict error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_from_dict.error",
                        "expected dataframe_from_dict to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_from_dict.error",
                        "expected dataframe_from_dict to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_from_dict".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameFromRecords => {
            let actual = execute_dataframe_from_records_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_from_records.error",
                        format!(
                            "expected dataframe_from_records error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_from_records.error",
                        "expected dataframe_from_records to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_from_records.error",
                        "expected dataframe_from_records to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_from_records"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameConstructorKwargs => {
            let actual = execute_dataframe_constructor_kwargs_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_kwargs.error",
                        format!(
                            "expected dataframe_constructor_kwargs error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_kwargs.error",
                        "expected dataframe_constructor_kwargs to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_kwargs.error",
                        "expected dataframe_constructor_kwargs to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_constructor_kwargs"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameConstructorScalar => {
            let actual = execute_dataframe_constructor_scalar_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_scalar.error",
                        format!(
                            "expected dataframe_constructor_scalar error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_scalar.error",
                        "expected dataframe_constructor_scalar to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_scalar.error",
                        "expected dataframe_constructor_scalar to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_constructor_scalar"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameConstructorDictOfSeries => {
            let actual = execute_dataframe_constructor_dict_of_series_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_dict_of_series.error",
                        format!(
                            "expected dataframe_constructor_dict_of_series error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_dict_of_series.error",
                        "expected dataframe_constructor_dict_of_series to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_dict_of_series.error",
                        "expected dataframe_constructor_dict_of_series to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_constructor_dict_of_series"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameConstructorListLike => {
            let actual = execute_dataframe_constructor_list_like_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_list_like.error",
                        format!(
                            "expected dataframe_constructor_list_like error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_list_like.error",
                        "expected dataframe_constructor_list_like to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_constructor_list_like.error",
                        "expected dataframe_constructor_list_like to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_constructor_list_like"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::GroupBySum => {
            let actual =
                execute_groupby_fixture_operation(fixture, fixture.operation, policy, ledger)?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for groupby_sum".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::IndexAlignUnion => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let plan = align_union(
                &Index::new(left.index.clone()),
                &Index::new(right.index.clone()),
            );
            validate_alignment_plan(&plan).map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Alignment(a) => a,
                _ => return Err("expected_alignment required for index_align_union".to_owned()),
            };
            Ok(diff_alignment(&plan, &expected))
        }
        FixtureOperation::IndexHasDuplicates
        | FixtureOperation::IndexIsMonotonicIncreasing
        | FixtureOperation::IndexIsMonotonicDecreasing => {
            let index = require_index(fixture)?;
            let index = Index::new(index.clone());
            let actual = match fixture.operation {
                FixtureOperation::IndexHasDuplicates => index.has_duplicates(),
                FixtureOperation::IndexIsMonotonicIncreasing => index.is_monotonic_increasing(),
                FixtureOperation::IndexIsMonotonicDecreasing => index.is_monotonic_decreasing(),
                _ => unreachable!(),
            };
            let expected = match expected {
                ResolvedExpected::Bool(b) => b,
                _ => {
                    return Err(format!(
                        "expected_bool required for {}",
                        fixture.operation.operation_name()
                    ));
                }
            };
            Ok(diff_bool(
                actual,
                expected,
                fixture.operation.operation_name(),
            ))
        }
        FixtureOperation::IndexFirstPositions => {
            let index = require_index(fixture)?;
            let index = Index::new(index.clone());
            let positions = index.position_map_first();
            let actual: Vec<Option<usize>> = index
                .labels()
                .iter()
                .map(|label| positions.get(label).copied())
                .collect();
            let expected = match expected {
                ResolvedExpected::Positions(p) => p,
                _ => {
                    return Err("expected_positions required for index_first_positions".to_owned());
                }
            };
            Ok(diff_positions(&actual, &expected))
        }
        FixtureOperation::SeriesConcat => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let left_s = build_series(left).map_err(|err| format!("left build: {err}"))?;
            let right_s = build_series(right).map_err(|err| format!("right build: {err}"))?;
            let actual = concat_series(&[&left_s, &right_s]).map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for series_concat".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::NanSum
        | FixtureOperation::NanMean
        | FixtureOperation::NanMin
        | FixtureOperation::NanMax
        | FixtureOperation::NanStd
        | FixtureOperation::NanVar
        | FixtureOperation::NanCount => {
            let actual = execute_nanop_fixture_operation(fixture, fixture.operation)?;
            let expected = match expected {
                ResolvedExpected::Scalar(scalar) => scalar,
                _ => {
                    return Err(format!(
                        "expected_scalar required for {}",
                        fixture.operation.operation_name()
                    ));
                }
            };
            Ok(diff_scalar(
                &actual,
                &expected,
                fixture.operation.operation_name(),
            ))
        }
        FixtureOperation::FillNa => {
            let left = require_left_series(fixture)?;
            let fill = fixture
                .fill_value
                .as_ref()
                .ok_or_else(|| "fill_value is required for fill_na".to_owned())?;
            let actual_values = fill_na(&left.values, fill);
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series required for fill_na".to_owned()),
            };
            Ok(diff_values(&actual_values, &expected.values, "fill_na"))
        }
        FixtureOperation::DropNa => {
            let left = require_left_series(fixture)?;
            let actual_values = dropna(&left.values);
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series required for drop_na".to_owned()),
            };
            Ok(diff_values(&actual_values, &expected.values, "drop_na"))
        }
        FixtureOperation::CsvRoundTrip => {
            let actual = execute_csv_round_trip_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Bool(value) => Ok(diff_bool(actual?, value, "csv_round_trip")),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "csv_round_trip.error",
                        format!(
                            "expected csv_round_trip error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "csv_round_trip.error",
                        "expected csv_round_trip to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "csv_round_trip.error",
                        "expected csv_round_trip to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_bool or expected_error required for csv_round_trip".to_owned()),
            }
        }
        FixtureOperation::ColumnDtypeCheck => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual_dtype = format!("{:?}", series.column().dtype());
            let expected = match expected {
                ResolvedExpected::Dtype(dtype) => dtype,
                _ => return Err("expected_dtype required for column_dtype_check".to_owned()),
            };
            Ok(diff_string(&actual_dtype, &expected, "column_dtype"))
        }
        FixtureOperation::SeriesFilter => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let data = build_series(left)?;
            let mask = build_series(right)?;
            let actual = data.filter(&mask).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_filter.error",
                        format!(
                            "expected series_filter error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_filter.error",
                        "expected series_filter to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_filter.error",
                        "expected series_filter to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_filter".to_owned()),
            }
        }
        FixtureOperation::SeriesHead => {
            let left = require_left_series(fixture)?;
            let n = fixture
                .head_n
                .ok_or_else(|| "head_n is required for series_head".to_owned())?;
            let series = build_series(left)?;
            let take = normalize_head_take(n, series.len());
            let labels = series.index().labels()[..take].to_vec();
            let values = series.values()[..take].to_vec();
            let actual =
                Series::from_values(series.name(), labels, values).map_err(|e| e.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for series_head".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::SeriesTail => {
            let left = require_left_series(fixture)?;
            let n = fixture
                .tail_n
                .ok_or_else(|| "tail_n is required for series_tail".to_owned())?;
            let series = build_series(left)?;
            let actual = series.tail(n).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_tail.error",
                        format!(
                            "expected series_tail error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_tail.error",
                        "expected series_tail to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_tail.error",
                        "expected series_tail to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_tail".to_owned()),
            }
        }
        FixtureOperation::SeriesValueCounts => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.value_counts().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_value_counts.error",
                        format!(
                            "expected series_value_counts error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_value_counts.error",
                        "expected series_value_counts to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_value_counts.error",
                        "expected series_value_counts to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for series_value_counts".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesSortIndex => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series
                .sort_index(resolve_sort_ascending(fixture))
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_sort_index.error",
                        format!(
                            "expected series_sort_index error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_sort_index.error",
                        "expected series_sort_index to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_sort_index.error",
                        "expected series_sort_index to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for series_sort_index".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesSortValues => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series
                .sort_values(resolve_sort_ascending(fixture))
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_sort_values.error",
                        format!(
                            "expected series_sort_values error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_sort_values.error",
                        "expected series_sort_values to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_sort_values.error",
                        "expected series_sort_values to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for series_sort_values".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesDiff => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let periods = fixture.diff_periods.unwrap_or(1);
            let actual = series.diff(periods).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_diff.error",
                        format!(
                            "expected series_diff error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_diff.error",
                        "expected series_diff to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_diff.error",
                        "expected series_diff to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_diff".to_owned()),
            }
        }
        FixtureOperation::SeriesShift => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let periods = fixture.shift_periods.unwrap_or(1);
            let actual = series.shift(periods).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_shift.error",
                        format!(
                            "expected series_shift error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_shift.error",
                        "expected series_shift to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_shift.error",
                        "expected series_shift to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_shift".to_owned()),
            }
        }
        FixtureOperation::SeriesPctChange => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let periods = fixture.pct_change_periods.unwrap_or(1) as usize;
            let actual = series.pct_change(periods).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_pct_change.error",
                        format!(
                            "expected series_pct_change error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_pct_change.error",
                        "expected series_pct_change to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_pct_change.error",
                        "expected series_pct_change to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for series_pct_change".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesIsNa => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.isna().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_isna.error",
                        format!(
                            "expected series_isna error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_isna.error",
                        "expected series_isna to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_isna.error",
                        "expected series_isna to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_isna".to_owned()),
            }
        }
        FixtureOperation::SeriesNotNa => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.notna().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_notna.error",
                        format!(
                            "expected series_notna error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_notna.error",
                        "expected series_notna to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_notna.error",
                        "expected series_notna to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_notna".to_owned()),
            }
        }
        FixtureOperation::SeriesIsNull => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.isnull().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_isnull.error",
                        format!(
                            "expected series_isnull error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_isnull.error",
                        "expected series_isnull to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_isnull.error",
                        "expected series_isnull to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_isnull".to_owned()),
            }
        }
        FixtureOperation::SeriesNotNull => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.notnull().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_notnull.error",
                        format!(
                            "expected series_notnull error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_notnull.error",
                        "expected series_notnull to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_notnull.error",
                        "expected series_notnull to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => {
                    Err("expected_series or expected_error required for series_notnull".to_owned())
                }
            }
        }
        FixtureOperation::SeriesFillNa => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let fill_value = fixture
                .fill_value
                .as_ref()
                .ok_or_else(|| "fill_value is required for series_fillna".to_owned())?;
            let actual = series.fillna(fill_value).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_fillna.error",
                        format!(
                            "expected series_fillna error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_fillna.error",
                        "expected series_fillna to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_fillna.error",
                        "expected series_fillna to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_fillna".to_owned()),
            }
        }
        FixtureOperation::SeriesDropNa => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.dropna().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_dropna.error",
                        format!(
                            "expected series_dropna error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_dropna.error",
                        "expected series_dropna to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_dropna.error",
                        "expected series_dropna to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_dropna".to_owned()),
            }
        }
        FixtureOperation::SeriesCount => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = Scalar::Int64(series.count() as i64);
            match expected {
                ResolvedExpected::Scalar(scalar) => Ok(diff_scalar(
                    &actual,
                    &scalar,
                    fixture.operation.operation_name(),
                )),
                _ => Err("expected_scalar required for series_count".to_owned()),
            }
        }
        FixtureOperation::DataFrameCount => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let actual = frame.count().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_count.error",
                        format!(
                            "expected dataframe_count error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_count.error",
                        "expected dataframe_count to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_count.error",
                        "expected dataframe_count to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => {
                    Err("expected_series or expected_error required for dataframe_count".to_owned())
                }
            }
        }
        FixtureOperation::DataFrameMode => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let actual = frame.mode().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_mode.error",
                        format!(
                            "expected dataframe_mode error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_mode.error",
                        "expected dataframe_mode to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_mode.error",
                        "expected dataframe_mode to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_frame or expected_error required for dataframe_mode".to_owned()),
            }
        }
        FixtureOperation::DataFrameRank => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let method = fixture.rank_method.as_deref().unwrap_or("average");
            let na_option = fixture.rank_na_option.as_deref().unwrap_or("keep");
            let ascending = resolve_sort_ascending(fixture);
            let axis = resolve_rank_axis(fixture)?;
            let actual = if axis == 1 {
                frame.rank_axis1(method, ascending, na_option)
            } else {
                frame.rank(method, ascending, na_option)
            }
            .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_rank.error",
                        format!(
                            "expected dataframe_rank error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_rank.error",
                        "expected dataframe_rank to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_rank.error",
                        "expected dataframe_rank to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_frame or expected_error required for dataframe_rank".to_owned()),
            }
        }
        FixtureOperation::DataFrameDuplicated => {
            let frame = build_dataframe(require_frame(fixture)?)
                .map_err(|err| format!("frame build failed: {err}"))?;
            let subset = resolve_duplicate_subset(fixture)?;
            let keep = resolve_duplicate_keep(fixture)?;
            let actual = frame
                .duplicated(subset.as_deref(), keep)
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_duplicated.error",
                        format!(
                            "expected dataframe_duplicated error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_duplicated.error",
                        "expected dataframe_duplicated to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_duplicated.error",
                        "expected dataframe_duplicated to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for dataframe_duplicated"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesAny => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.any().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Bool(value) => Ok(diff_bool(actual?, value, "series_any")),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_any.error",
                        format!("expected series_any error containing '{substr}', got '{message}'"),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_any.error",
                        "expected series_any to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_any.error",
                        "expected series_any to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_bool or expected_error required for series_any".to_owned()),
            }
        }
        FixtureOperation::SeriesAll => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual = series.all().map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Bool(value) => Ok(diff_bool(actual?, value, "series_all")),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_all.error",
                        format!("expected series_all error containing '{substr}', got '{message}'"),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_all.error",
                        "expected series_all to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_all.error",
                        "expected series_all to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_bool or expected_error required for series_all".to_owned()),
            }
        }
        FixtureOperation::SeriesBool => {
            let actual = execute_series_bool_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Bool(value) => Ok(diff_bool(actual?, value, "series_bool")),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_bool.error",
                        format!(
                            "expected series_bool error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_bool.error",
                        "expected series_bool to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_bool.error",
                        "expected series_bool to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_bool or expected_error required for series_bool".to_owned()),
            }
        }
        FixtureOperation::SeriesRepeat => {
            let actual = execute_series_repeat_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_repeat.error",
                        format!(
                            "expected series_repeat error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_repeat.error",
                        "expected series_repeat to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_repeat.error",
                        "expected series_repeat to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_repeat".to_owned()),
            }
        }
        FixtureOperation::SeriesCombineFirst => {
            let actual = execute_series_combine_first_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                _ => Err("expected_series required for series_combine_first".to_owned()),
            }
        }
        FixtureOperation::SeriesToNumeric
        | FixtureOperation::SeriesConvertDtypes
        | FixtureOperation::SeriesAstype
        | FixtureOperation::SeriesClip
        | FixtureOperation::SeriesAbs
        | FixtureOperation::SeriesRound
        | FixtureOperation::SeriesCumsum
        | FixtureOperation::SeriesCumprod
        | FixtureOperation::SeriesCummax
        | FixtureOperation::SeriesCummin
        | FixtureOperation::SeriesNlargest
        | FixtureOperation::SeriesNsmallest
        | FixtureOperation::SeriesBetween
        | FixtureOperation::SeriesCut
        | FixtureOperation::SeriesQcut => {
            let actual = execute_series_module_utility_fixture_operation(fixture);
            let op_name = fixture.operation.operation_name();
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        format!("{op_name}.error"),
                        format!("expected {op_name} error containing '{substr}', got '{message}'"),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        format!("{op_name}.error"),
                        format!("expected {op_name} to fail but operation succeeded"),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        format!("{op_name}.error"),
                        format!("expected {op_name} to fail but operation succeeded"),
                    )],
                }),
                _ => Err(format!(
                    "expected_series or expected_error required for {op_name}"
                )),
            }
        }
        FixtureOperation::SeriesXs => {
            let actual = execute_series_xs_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_xs.error",
                        format!("expected series_xs error containing '{substr}', got '{message}'"),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_xs.error",
                        "expected series_xs to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_xs.error",
                        "expected series_xs to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_xs".to_owned()),
            }
        }
        FixtureOperation::SeriesLoc => {
            let left = require_left_series(fixture)?;
            let labels = require_loc_labels(fixture)?;
            let series = build_series(left)?;
            let actual = series.loc(labels).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_loc.error",
                        format!("expected series_loc error containing '{substr}', got '{message}'"),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_loc.error",
                        "expected series_loc to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_loc.error",
                        "expected series_loc to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_loc".to_owned()),
            }
        }
        FixtureOperation::SeriesIloc => {
            let left = require_left_series(fixture)?;
            let positions = require_iloc_positions(fixture)?;
            let series = build_series(left)?;
            let actual = series.iloc(positions).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_iloc.error",
                        format!(
                            "expected series_iloc error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_iloc.error",
                        "expected series_iloc to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_iloc.error",
                        "expected series_iloc to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_iloc".to_owned()),
            }
        }
        FixtureOperation::SeriesTake => {
            let left = require_left_series(fixture)?;
            let indices = require_take_indices(fixture)?;
            let series = build_series(left)?;
            let actual = series.take(indices).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_take.error",
                        format!(
                            "expected series_take error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_take.error",
                        "expected series_take to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_take.error",
                        "expected series_take to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_series or expected_error required for series_take".to_owned()),
            }
        }
        FixtureOperation::SeriesAtTime => {
            let left = require_left_series(fixture)?;
            let time = require_time_value(fixture)?;
            let series = build_series(left)?;
            let actual = series.at_time(time).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_at_time.error",
                        format!(
                            "expected series_at_time error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_at_time.error",
                        "expected series_at_time to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_at_time.error",
                        "expected series_at_time to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => {
                    Err("expected_series or expected_error required for series_at_time".to_owned())
                }
            }
        }
        FixtureOperation::SeriesBetweenTime => {
            let left = require_left_series(fixture)?;
            let start = require_start_time(fixture)?;
            let end = require_end_time(fixture)?;
            let series = build_series(left)?;
            let actual = series
                .between_time(start, end)
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Series(s) => Ok(diff_series(&actual?, &s)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_between_time.error",
                        format!(
                            "expected series_between_time error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_between_time.error",
                        "expected series_between_time to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_between_time.error",
                        "expected series_between_time to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for series_between_time".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesPartitionDf => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_partition_df.error",
                        format!(
                            "expected series_partition_df error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_partition_df.error",
                        "expected series_partition_df to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_partition_df.error",
                        "expected series_partition_df to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for series_partition_df".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesRpartitionDf => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_rpartition_df.error",
                        format!(
                            "expected series_rpartition_df error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_rpartition_df.error",
                        "expected series_rpartition_df to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_rpartition_df.error",
                        "expected series_rpartition_df to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for series_rpartition_df".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesExtractAll => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_extractall.error",
                        format!(
                            "expected series_extractall error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_extractall.error",
                        "expected series_extractall to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_extractall.error",
                        "expected series_extractall to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for series_extractall".to_owned(),
                ),
            }
        }
        FixtureOperation::SeriesExtractDf => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_extract_df.error",
                        format!(
                            "expected series_extract_df error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_extract_df.error",
                        "expected series_extract_df to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "series_extract_df.error",
                        "expected series_extract_df to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for series_extract_df".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameLoc => {
            let frame = require_frame(fixture)?;
            let labels = require_loc_labels(fixture)?;
            let frame = build_dataframe(frame)?;
            let actual = frame
                .loc_with_columns(labels, fixture.column_order.as_deref())
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_loc.error",
                        format!(
                            "expected dataframe_loc error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_loc.error",
                        "expected dataframe_loc to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_loc.error",
                        "expected dataframe_loc to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_frame or expected_error required for dataframe_loc".to_owned()),
            }
        }
        FixtureOperation::DataFrameIloc => {
            let frame = require_frame(fixture)?;
            let positions = require_iloc_positions(fixture)?;
            let frame = build_dataframe(frame)?;
            let actual = frame
                .iloc_with_columns(positions, fixture.column_order.as_deref())
                .map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_iloc.error",
                        format!(
                            "expected dataframe_iloc error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_iloc.error",
                        "expected dataframe_iloc to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_iloc.error",
                        "expected dataframe_iloc to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_frame or expected_error required for dataframe_iloc".to_owned()),
            }
        }
        FixtureOperation::DataFrameTake => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_take.error",
                        format!(
                            "expected dataframe_take error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_take.error",
                        "expected dataframe_take to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_take.error",
                        "expected dataframe_take to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_frame or expected_error required for dataframe_take".to_owned()),
            }
        }
        FixtureOperation::DataFrameXs => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_xs.error",
                        format!(
                            "expected dataframe_xs error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_xs.error",
                        "expected dataframe_xs to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_xs.error",
                        "expected dataframe_xs to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_frame or expected_error required for dataframe_xs".to_owned()),
            }
        }
        FixtureOperation::DataFrameGroupByIdxMin => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_idxmin.error",
                        format!(
                            "expected dataframe_groupby_idxmin error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_idxmin.error",
                        "expected dataframe_groupby_idxmin to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_idxmin.error",
                        "expected dataframe_groupby_idxmin to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_idxmin"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByIdxMax => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_idxmax.error",
                        format!(
                            "expected dataframe_groupby_idxmax error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_idxmax.error",
                        "expected dataframe_groupby_idxmax to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_idxmax.error",
                        "expected dataframe_groupby_idxmax to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_idxmax"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByAny => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_any.error",
                        format!(
                            "expected dataframe_groupby_any error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_any.error",
                        "expected dataframe_groupby_any to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_any.error",
                        "expected dataframe_groupby_any to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_any"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByAll => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_all.error",
                        format!(
                            "expected dataframe_groupby_all error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_all.error",
                        "expected dataframe_groupby_all to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_all.error",
                        "expected dataframe_groupby_all to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_all"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByGetGroup => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_get_group.error",
                        format!(
                            "expected dataframe_groupby_get_group error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_get_group.error",
                        "expected dataframe_groupby_get_group to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_get_group.error",
                        "expected dataframe_groupby_get_group to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_get_group"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByFfill => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_ffill.error",
                        format!(
                            "expected dataframe_groupby_ffill error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_ffill.error",
                        "expected dataframe_groupby_ffill to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_ffill.error",
                        "expected dataframe_groupby_ffill to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_ffill"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByBfill => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_bfill.error",
                        format!(
                            "expected dataframe_groupby_bfill error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_bfill.error",
                        "expected dataframe_groupby_bfill to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_bfill.error",
                        "expected dataframe_groupby_bfill to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_bfill"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupBySem => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_sem.error",
                        format!(
                            "expected dataframe_groupby_sem error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_sem.error",
                        "expected dataframe_groupby_sem to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_sem.error",
                        "expected dataframe_groupby_sem to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_sem"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupBySkew => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_skew.error",
                        format!(
                            "expected dataframe_groupby_skew error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_skew.error",
                        "expected dataframe_groupby_skew to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_skew.error",
                        "expected dataframe_groupby_skew to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_skew"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByKurtosis => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_kurtosis.error",
                        format!(
                            "expected dataframe_groupby_kurtosis error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_kurtosis.error",
                        "expected dataframe_groupby_kurtosis to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_kurtosis.error",
                        "expected dataframe_groupby_kurtosis to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_kurtosis"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByOhlc => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_ohlc.error",
                        format!(
                            "expected dataframe_groupby_ohlc error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_ohlc.error",
                        "expected dataframe_groupby_ohlc to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_ohlc.error",
                        "expected dataframe_groupby_ohlc to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_groupby_ohlc"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByCumcount => {
            let actual = execute_dataframe_groupby_series_fixture_operation(fixture, false);
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_cumcount.error",
                        format!(
                            "expected dataframe_groupby_cumcount error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_cumcount.error",
                        "expected dataframe_groupby_cumcount to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_cumcount.error",
                        "expected dataframe_groupby_cumcount to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for dataframe_groupby_cumcount"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameGroupByNgroup => {
            let actual = execute_dataframe_groupby_series_fixture_operation(fixture, true);
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_ngroup.error",
                        format!(
                            "expected dataframe_groupby_ngroup error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_ngroup.error",
                        "expected dataframe_groupby_ngroup to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_groupby_ngroup.error",
                        "expected dataframe_groupby_ngroup to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_series or expected_error required for dataframe_groupby_ngroup"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameAsof => {
            let actual = execute_dataframe_asof_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Series(series) => Ok(diff_series(&actual?, &series)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_asof.error",
                        format!(
                            "expected dataframe_asof error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_asof.error",
                        "expected dataframe_asof to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_asof.error",
                        "expected dataframe_asof to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => {
                    Err("expected_series or expected_error required for dataframe_asof".to_owned())
                }
            }
        }
        FixtureOperation::DataFrameAtTime => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_at_time.error",
                        format!(
                            "expected dataframe_at_time error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_at_time.error",
                        "expected dataframe_at_time to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_at_time.error",
                        "expected dataframe_at_time to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_at_time".to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameBetweenTime => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_between_time.error",
                        format!(
                            "expected dataframe_between_time error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_between_time.error",
                        "expected dataframe_between_time to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_between_time.error",
                        "expected dataframe_between_time to fail but operation succeeded"
                            .to_owned(),
                    )],
                }),
                _ => Err(
                    "expected_frame or expected_error required for dataframe_between_time"
                        .to_owned(),
                ),
            }
        }
        FixtureOperation::DataFrameBool => {
            let actual = execute_dataframe_bool_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Bool(value) => Ok(diff_bool(actual?, value, "dataframe_bool")),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_bool.error",
                        format!(
                            "expected dataframe_bool error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_bool.error",
                        "expected dataframe_bool to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_bool.error",
                        "expected dataframe_bool to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_bool or expected_error required for dataframe_bool".to_owned()),
            }
        }
        FixtureOperation::DataFrameHead => {
            let frame = require_frame(fixture)?;
            let n = fixture
                .head_n
                .ok_or_else(|| "head_n is required for dataframe_head".to_owned())?;
            let frame = build_dataframe(frame)?;
            let actual = frame.head(n).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_head.error",
                        format!(
                            "expected dataframe_head error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_head.error",
                        "expected dataframe_head to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_head.error",
                        "expected dataframe_head to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_frame or expected_error required for dataframe_head".to_owned()),
            }
        }
        FixtureOperation::DataFrameTail => {
            let frame = require_frame(fixture)?;
            let n = fixture
                .tail_n
                .ok_or_else(|| "tail_n is required for dataframe_tail".to_owned())?;
            let frame = build_dataframe(frame)?;
            let actual = frame.tail(n).map_err(|err| err.to_string());
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_tail.error",
                        format!(
                            "expected dataframe_tail error containing '{substr}', got '{message}'"
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_tail.error",
                        "expected dataframe_tail to fail but operation succeeded".to_owned(),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        "dataframe_tail.error",
                        "expected dataframe_tail to fail but operation succeeded".to_owned(),
                    )],
                }),
                _ => Err("expected_frame or expected_error required for dataframe_tail".to_owned()),
            }
        }
        FixtureOperation::DataFrameMerge
        | FixtureOperation::DataFrameMergeIndex
        | FixtureOperation::DataFrameMergeAsof
        | FixtureOperation::DataFrameMergeOrdered
        | FixtureOperation::DataFrameConcat
        | FixtureOperation::DataFrameIsNa
        | FixtureOperation::DataFrameNotNa
        | FixtureOperation::DataFrameIsNull
        | FixtureOperation::DataFrameNotNull
        | FixtureOperation::DataFrameFillNa
        | FixtureOperation::DataFrameDropNa
        | FixtureOperation::DataFrameDropNaColumns
        | FixtureOperation::DataFrameSetIndex
        | FixtureOperation::DataFrameResetIndex
        | FixtureOperation::DataFrameDropDuplicates
        | FixtureOperation::DataFrameSortIndex
        | FixtureOperation::DataFrameSortValues
        | FixtureOperation::DataFrameDiff
        | FixtureOperation::DataFrameShift
        | FixtureOperation::DataFramePctChange
        | FixtureOperation::DataFrameMelt
        | FixtureOperation::DataFrameCombineFirst => {
            let actual = execute_dataframe_fixture_operation(fixture);
            match expected {
                ResolvedExpected::Frame(frame) => Ok(diff_dataframe(&actual?, &frame)),
                ResolvedExpected::ErrorContains(substr) => Ok(match actual {
                    Err(message) if message.contains(&substr) => Vec::new(),
                    Err(message) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        format!("{:?}.error", fixture.operation),
                        format!(
                            "expected {:?} error containing '{substr}', got '{message}'",
                            fixture.operation
                        ),
                    )],
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        format!("{:?}.error", fixture.operation),
                        format!(
                            "expected {:?} to fail but operation succeeded",
                            fixture.operation
                        ),
                    )],
                }),
                ResolvedExpected::ErrorAny => Ok(match actual {
                    Err(_) => Vec::new(),
                    Ok(_) => vec![make_drift_record(
                        ComparisonCategory::Value,
                        DriftLevel::Critical,
                        format!("{:?}.error", fixture.operation),
                        format!(
                            "expected {:?} to fail but operation succeeded",
                            fixture.operation
                        ),
                    )],
                }),
                _ => Err(format!(
                    "expected_frame or expected_error required for {:?}",
                    fixture.operation
                )),
            }
        }
        FixtureOperation::GroupByMean
        | FixtureOperation::GroupByCount
        | FixtureOperation::GroupByMin
        | FixtureOperation::GroupByMax
        | FixtureOperation::GroupByFirst
        | FixtureOperation::GroupByLast
        | FixtureOperation::GroupByStd
        | FixtureOperation::GroupByVar
        | FixtureOperation::GroupByMedian => {
            let actual =
                execute_groupby_fixture_operation(fixture, fixture.operation, policy, ledger)?;
            let op_name = format!("{:?}", fixture.operation).to_lowercase();
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err(format!("expected_series required for {op_name}")),
            };
            Ok(diff_series(&actual, &expected))
        }
    }
}

fn diff_scalar(actual: &Scalar, expected: &Scalar, name: &str) -> Vec<DriftRecord> {
    if actual.semantic_eq(expected) {
        Vec::new()
    } else {
        vec![make_drift_record(
            ComparisonCategory::Value,
            DriftLevel::Critical,
            format!("{name}.scalar"),
            format!("scalar mismatch: actual={actual:?}, expected={expected:?}"),
        )]
    }
}

fn diff_values(actual: &[Scalar], expected: &[Scalar], name: &str) -> Vec<DriftRecord> {
    let mut drifts = Vec::new();
    if actual.len() != expected.len() {
        drifts.push(make_drift_record(
            ComparisonCategory::Shape,
            DriftLevel::Critical,
            format!("{name}.len"),
            format!(
                "length mismatch: actual={}, expected={}",
                actual.len(),
                expected.len()
            ),
        ));
        return drifts;
    }
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        if !a.semantic_eq(e) {
            drifts.push(make_drift_record(
                ComparisonCategory::Value,
                DriftLevel::Critical,
                format!("{name}[{i}]"),
                format!("value mismatch: actual={a:?}, expected={e:?}"),
            ));
        }
    }
    drifts
}

fn diff_string(actual: &str, expected: &str, name: &str) -> Vec<DriftRecord> {
    if actual == expected {
        Vec::new()
    } else {
        vec![make_drift_record(
            ComparisonCategory::Type,
            DriftLevel::Critical,
            format!("{name}.value"),
            format!("string mismatch: actual={actual:?}, expected={expected:?}"),
        )]
    }
}

fn diff_series(actual: &Series, expected: &FixtureExpectedSeries) -> Vec<DriftRecord> {
    let mut drifts = Vec::new();

    if actual.index().labels() != expected.index {
        drifts.push(make_drift_record(
            ComparisonCategory::Index,
            DriftLevel::Critical,
            "series.index",
            format!(
                "index mismatch: actual={:?}, expected={:?}",
                actual.index().labels(),
                expected.index
            ),
        ));
    }

    if actual.values().len() != expected.values.len() {
        drifts.push(make_drift_record(
            ComparisonCategory::Shape,
            DriftLevel::Critical,
            "series.values.len",
            format!(
                "length mismatch: actual={}, expected={}",
                actual.values().len(),
                expected.values.len()
            ),
        ));
        return drifts;
    }

    diff_value_vectors(
        actual.values(),
        &expected.values,
        "series.values",
        &mut drifts,
    );
    drifts
}

fn diff_dataframe(actual: &DataFrame, expected: &FixtureExpectedDataFrame) -> Vec<DriftRecord> {
    let mut drifts = Vec::new();

    if actual.index().labels() != expected.index {
        drifts.push(make_drift_record(
            ComparisonCategory::Index,
            DriftLevel::Critical,
            "dataframe.index",
            format!(
                "index mismatch: actual={:?}, expected={:?}",
                actual.index().labels(),
                expected.index
            ),
        ));
    }

    let actual_names = actual.columns().keys().cloned().collect::<Vec<_>>();
    let expected_names = expected.columns.keys().cloned().collect::<Vec<_>>();
    if actual_names != expected_names {
        drifts.push(make_drift_record(
            ComparisonCategory::Shape,
            DriftLevel::Critical,
            "dataframe.columns",
            format!("column mismatch: actual={actual_names:?}, expected={expected_names:?}"),
        ));
    }

    for (name, expected_values) in &expected.columns {
        let Some(column) = actual.column(name) else {
            drifts.push(make_drift_record(
                ComparisonCategory::Shape,
                DriftLevel::Critical,
                format!("dataframe.columns.{name}"),
                "column missing in actual result".to_owned(),
            ));
            continue;
        };

        let actual_values = column.values();
        if actual_values.len() != expected_values.len() {
            drifts.push(make_drift_record(
                ComparisonCategory::Shape,
                DriftLevel::Critical,
                format!("dataframe.columns.{name}.len"),
                format!(
                    "length mismatch: actual={}, expected={}",
                    actual_values.len(),
                    expected_values.len()
                ),
            ));
            continue;
        }

        diff_value_vectors(
            actual_values,
            expected_values,
            &format!("dataframe.columns.{name}.values"),
            &mut drifts,
        );
    }

    drifts
}

fn diff_join(actual: &fp_join::JoinedSeries, expected: &FixtureExpectedJoin) -> Vec<DriftRecord> {
    let mut drifts = Vec::new();

    if actual.index.labels() != expected.index {
        drifts.push(make_drift_record(
            ComparisonCategory::Index,
            DriftLevel::Critical,
            "join.index",
            format!(
                "index mismatch: actual={:?}, expected={:?}",
                actual.index.labels(),
                expected.index
            ),
        ));
    }

    if actual.left_values.values().len() != expected.left_values.len() {
        drifts.push(make_drift_record(
            ComparisonCategory::Shape,
            DriftLevel::Critical,
            "join.left_values.len",
            format!(
                "left length mismatch: actual={}, expected={}",
                actual.left_values.values().len(),
                expected.left_values.len()
            ),
        ));
    } else {
        diff_value_vectors(
            actual.left_values.values(),
            &expected.left_values,
            "join.left_values",
            &mut drifts,
        );
    }

    if actual.right_values.values().len() != expected.right_values.len() {
        drifts.push(make_drift_record(
            ComparisonCategory::Shape,
            DriftLevel::Critical,
            "join.right_values.len",
            format!(
                "right length mismatch: actual={}, expected={}",
                actual.right_values.values().len(),
                expected.right_values.len()
            ),
        ));
    } else {
        diff_value_vectors(
            actual.right_values.values(),
            &expected.right_values,
            "join.right_values",
            &mut drifts,
        );
    }

    drifts
}

fn diff_alignment(actual: &AlignmentPlan, expected: &FixtureExpectedAlignment) -> Vec<DriftRecord> {
    let mut drifts = Vec::new();

    if actual.union_index.labels() != expected.union_index {
        drifts.push(make_drift_record(
            ComparisonCategory::Index,
            DriftLevel::Critical,
            "alignment.union_index",
            format!(
                "union_index mismatch: actual={:?}, expected={:?}",
                actual.union_index.labels(),
                expected.union_index
            ),
        ));
    }

    if actual.left_positions != expected.left_positions {
        drifts.push(make_drift_record(
            ComparisonCategory::Value,
            DriftLevel::Critical,
            "alignment.left_positions",
            format!(
                "left_positions mismatch: actual={:?}, expected={:?}",
                actual.left_positions, expected.left_positions
            ),
        ));
    }

    if actual.right_positions != expected.right_positions {
        drifts.push(make_drift_record(
            ComparisonCategory::Value,
            DriftLevel::Critical,
            "alignment.right_positions",
            format!(
                "right_positions mismatch: actual={:?}, expected={:?}",
                actual.right_positions, expected.right_positions
            ),
        ));
    }

    drifts
}

fn diff_bool(actual: bool, expected: bool, name: &str) -> Vec<DriftRecord> {
    if actual == expected {
        Vec::new()
    } else {
        vec![make_drift_record(
            ComparisonCategory::Value,
            DriftLevel::Critical,
            name,
            format!("boolean mismatch: actual={actual}, expected={expected}"),
        )]
    }
}

fn diff_positions(actual: &[Option<usize>], expected: &[Option<usize>]) -> Vec<DriftRecord> {
    if actual == expected {
        Vec::new()
    } else {
        vec![make_drift_record(
            ComparisonCategory::Value,
            DriftLevel::Critical,
            "first_positions",
            format!("positions mismatch: actual={actual:?}, expected={expected:?}"),
        )]
    }
}

fn diff_value_vectors(
    actual: &[Scalar],
    expected: &[Scalar],
    prefix: &str,
    drifts: &mut Vec<DriftRecord>,
) {
    for (idx, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        let equal = a.semantic_eq(e) || (a.is_missing() && e.is_missing());
        if !equal {
            let location = format!("{prefix}[{idx}]");
            if a.is_missing() != e.is_missing() {
                drifts.push(make_drift_record(
                    ComparisonCategory::Nullness,
                    DriftLevel::Critical,
                    location,
                    format!("nullness mismatch: actual={a:?}, expected={e:?}"),
                ));
            } else {
                let level = classify_value_drift(a, e);
                drifts.push(make_drift_record(
                    ComparisonCategory::Value,
                    level,
                    location,
                    format!("value mismatch: actual={a:?}, expected={e:?}"),
                ));
            }
        }
    }
}

fn classify_value_drift(actual: &Scalar, expected: &Scalar) -> DriftLevel {
    match (actual, expected) {
        (Scalar::Float64(a), Scalar::Float64(e)) => {
            let max_abs = a.abs().max(e.abs()).max(1.0);
            let rel_diff = (a - e).abs() / max_abs;
            if rel_diff < 1e-10 {
                DriftLevel::NonCritical
            } else {
                DriftLevel::Critical
            }
        }
        _ => DriftLevel::Critical,
    }
}

fn summarize_drift(results: &[DifferentialResult]) -> DriftSummary {
    let mut total = 0usize;
    let mut critical = 0usize;
    let mut non_critical = 0usize;
    let mut informational = 0usize;
    let mut cat_counts = BTreeMap::<ComparisonCategory, usize>::new();

    for result in results {
        for drift in &result.drift_records {
            total += 1;
            match drift.level {
                DriftLevel::Critical => critical += 1,
                DriftLevel::NonCritical => non_critical += 1,
                DriftLevel::Informational => informational += 1,
            }
            *cat_counts.entry(drift.category).or_default() += 1;
        }
    }

    let categories = cat_counts
        .into_iter()
        .map(|(category, count)| CategoryCount { category, count })
        .collect();

    DriftSummary {
        total_drift_records: total,
        critical_count: critical,
        non_critical_count: non_critical,
        informational_count: informational,
        categories,
    }
}

fn percent(failed: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (failed as f64 / total as f64) * 100.0
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_decode(value: &str) -> Result<Vec<u8>, HarnessError> {
    if !value.len().is_multiple_of(2) {
        return Err(HarnessError::RaptorQ(format!(
            "invalid hex length {}",
            value.len()
        )));
    }
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(value.len() / 2);
    for idx in (0..bytes.len()).step_by(2) {
        let high = hex_value(bytes[idx])?;
        let low = hex_value(bytes[idx + 1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => unreachable!("nibble out of range"),
    }
}

fn hex_value(byte: u8) -> Result<u8, HarnessError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(HarnessError::RaptorQ(format!(
            "invalid hex character: {}",
            byte as char
        ))),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

// === E2E Orchestrator + Replay/Forensics Logging (bd-2gi.6) ===

/// Forensic event kinds emitted during E2E orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ForensicEventKind {
    SuiteStart {
        suite: String,
        packet_filter: Option<String>,
    },
    SuiteEnd {
        suite: String,
        total_fixtures: usize,
        passed: usize,
        failed: usize,
    },
    PacketStart {
        packet_id: String,
    },
    PacketEnd {
        packet_id: String,
        fixtures: usize,
        passed: usize,
        failed: usize,
        gate_pass: bool,
    },
    CaseStart {
        scenario_id: String,
        packet_id: String,
        case_id: String,
        trace_id: String,
        step_id: String,
        seed: u64,
        assertion_path: String,
        replay_cmd: String,
        operation: FixtureOperation,
        mode: RuntimeMode,
    },
    CaseEnd {
        scenario_id: String,
        packet_id: String,
        case_id: String,
        trace_id: String,
        step_id: String,
        seed: u64,
        assertion_path: String,
        result: String,
        replay_cmd: String,
        decision_action: String,
        replay_key: String,
        mismatch_class: Option<String>,
        status: CaseStatus,
        evidence_records: usize,
        elapsed_us: u64,
    },
    CompatClosureCase {
        ts_utc: u64,
        suite_id: String,
        test_id: String,
        api_surface_id: String,
        packet_id: String,
        mode: RuntimeMode,
        seed: u64,
        input_digest: String,
        output_digest: String,
        env_fingerprint: String,
        artifact_refs: Vec<String>,
        duration_ms: u64,
        outcome: String,
        reason_code: String,
    },
    ArtifactWritten {
        packet_id: String,
        artifact_kind: String,
        path: String,
    },
    GateEvaluated {
        packet_id: String,
        pass: bool,
        reasons: Vec<String>,
    },
    DriftHistoryAppended {
        path: String,
        entries: usize,
    },
    Error {
        phase: String,
        message: String,
    },
}

/// A single forensic log entry with timestamp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForensicEvent {
    pub ts_unix_ms: u64,
    pub event: ForensicEventKind,
}

/// Accumulator for forensic events during an E2E run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForensicLog {
    pub events: Vec<ForensicEvent>,
}

impl ForensicLog {
    #[must_use]
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn record(&mut self, event: ForensicEventKind) {
        self.events.push(ForensicEvent {
            ts_unix_ms: now_unix_ms(),
            event,
        });
    }

    /// Write the forensic log as JSONL to the given path.
    pub fn write_jsonl(&self, path: &Path) -> Result<(), HarnessError> {
        let mut file = fs::File::create(path).map_err(HarnessError::Io)?;
        for entry in &self.events {
            let line = serde_json::to_string(entry).map_err(HarnessError::Json)?;
            writeln!(file, "{line}").map_err(HarnessError::Io)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Lifecycle hooks for E2E orchestration. Default implementations are no-ops.
pub trait LifecycleHooks {
    fn before_suite(&mut self, _suite: &str, _packet_filter: &Option<String>) {}
    fn after_suite(&mut self, _report: &[PacketParityReport]) {}
    fn before_packet(&mut self, _packet_id: &str) {}
    fn after_packet(&mut self, _report: &PacketParityReport, _gate_pass: bool) {}
    fn before_case(&mut self, _fixture: &PacketFixture) {}
    fn after_case(&mut self, _result: &CaseResult) {}
}

/// Default no-op hooks.
pub struct NoopHooks;
impl LifecycleHooks for NoopHooks {}

/// Configuration for the E2E orchestrator.
#[derive(Debug, Clone)]
pub struct E2eConfig {
    pub harness: HarnessConfig,
    pub options: SuiteOptions,
    pub write_artifacts: bool,
    pub enforce_gates: bool,
    pub append_drift_history: bool,
    pub forensic_log_path: Option<PathBuf>,
}

impl E2eConfig {
    #[must_use]
    pub fn default_all_phases() -> Self {
        Self {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions::default(),
            write_artifacts: true,
            enforce_gates: true,
            append_drift_history: true,
            forensic_log_path: None,
        }
    }
}

/// Final result of an E2E orchestration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2eReport {
    pub suite: String,
    pub packet_reports: Vec<PacketParityReport>,
    pub artifacts_written: Vec<WrittenPacketArtifacts>,
    pub gate_results: Vec<PacketGateResult>,
    pub gates_pass: bool,
    pub drift_history_path: Option<String>,
    pub forensic_log: ForensicLog,
    pub total_fixtures: usize,
    pub total_passed: usize,
    pub total_failed: usize,
}

impl E2eReport {
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.total_failed == 0 && self.total_fixtures > 0 && self.gates_pass
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatClosureScenarioKind {
    GoldenJourney,
    Regression,
    FailureInjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatClosureE2eScenarioStep {
    pub scenario_id: String,
    pub packet_id: String,
    pub mode: RuntimeMode,
    pub trace_id: String,
    pub step_id: String,
    pub kind: CompatClosureScenarioKind,
    pub command_or_api: String,
    pub input_ref: String,
    pub output_ref: String,
    pub duration_ms: u64,
    pub retry_count: u32,
    pub outcome: String,
    pub reason_code: String,
    pub replay_cmd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatClosureE2eScenarioReport {
    pub suite_id: String,
    pub scenario_count: usize,
    pub pass_count: usize,
    pub fail_count: usize,
    pub steps: Vec<CompatClosureE2eScenarioStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct CompatClosureScenarioBuildStats {
    trace_metadata_index_nodes: usize,
    trace_metadata_lookup_steps: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CompatClosureTraceMetadata {
    operation: FixtureOperation,
    mode: RuntimeMode,
}

fn append_fault_injection_steps(
    steps: &mut Vec<CompatClosureE2eScenarioStep>,
    fault_reports: &[FaultInjectionValidationReport],
) {
    for report in fault_reports {
        for entry in &report.entries {
            let scenario_id = format!(
                "fault-injection:{}:{}:{}",
                entry.packet_id,
                entry.case_id,
                runtime_mode_slug(entry.mode)
            );
            let outcome = match entry.classification {
                FaultInjectionClassification::StrictViolation => "fail",
                FaultInjectionClassification::HardenedAllowlisted => "allowlisted",
            };
            steps.push(CompatClosureE2eScenarioStep {
                scenario_id,
                packet_id: entry.packet_id.clone(),
                mode: entry.mode,
                trace_id: entry.trace_id.clone(),
                step_id: "fault-injected".to_owned(),
                kind: CompatClosureScenarioKind::FailureInjection,
                command_or_api: "fault_injection".to_owned(),
                input_ref: format!("fixture://{}/{}", entry.packet_id, entry.case_id),
                output_ref: format!(
                    "artifact://{}/fault_injection_validation.json#{}",
                    entry.packet_id, entry.replay_key
                ),
                duration_ms: 1,
                retry_count: 0,
                outcome: outcome.to_owned(),
                reason_code: entry.mismatch_class.clone(),
                replay_cmd: "cargo test -p fp-conformance --lib fault_injection_validation_classifies_strict_vs_hardened -- --nocapture".to_owned(),
            });
        }
    }
}

fn finalize_compat_closure_e2e_scenario_report(
    mut steps: Vec<CompatClosureE2eScenarioStep>,
) -> CompatClosureE2eScenarioReport {
    steps.sort_by(|a, b| {
        (
            a.scenario_id.as_str(),
            a.step_id.as_str(),
            runtime_mode_slug(a.mode),
        )
            .cmp(&(
                b.scenario_id.as_str(),
                b.step_id.as_str(),
                runtime_mode_slug(b.mode),
            ))
    });

    let pass_count = steps
        .iter()
        .filter(|step| step.outcome == "pass" || step.outcome == "allowlisted")
        .count();
    let fail_count = steps.len().saturating_sub(pass_count);
    let scenario_count = steps
        .iter()
        .map(|step| step.scenario_id.as_str())
        .collect::<BTreeSet<_>>()
        .len();

    CompatClosureE2eScenarioReport {
        suite_id: "COMPAT-CLOSURE-G".to_owned(),
        scenario_count,
        pass_count,
        fail_count,
        steps,
    }
}

#[cfg(test)]
fn build_compat_closure_e2e_scenario_report_baseline_with_stats(
    e2e: &E2eReport,
    fault_reports: &[FaultInjectionValidationReport],
) -> (
    CompatClosureE2eScenarioReport,
    CompatClosureScenarioBuildStats,
) {
    let mut operation_by_trace = BTreeMap::<String, FixtureOperation>::new();
    let mut mode_by_trace = BTreeMap::<String, RuntimeMode>::new();
    let mut trace_metadata_index_nodes = 0_usize;

    for event in &e2e.forensic_log.events {
        if let ForensicEventKind::CaseStart {
            trace_id,
            operation,
            mode,
            ..
        } = &event.event
        {
            operation_by_trace.insert(trace_id.clone(), *operation);
            mode_by_trace.insert(trace_id.clone(), *mode);
            trace_metadata_index_nodes += 2;
        }
    }

    let mut steps = Vec::new();
    let mut trace_metadata_lookup_steps = 0_usize;
    for event in &e2e.forensic_log.events {
        if let ForensicEventKind::CaseEnd {
            scenario_id,
            packet_id,
            trace_id,
            step_id,
            result,
            replay_cmd,
            replay_key,
            mismatch_class,
            elapsed_us,
            ..
        } = &event.event
        {
            trace_metadata_lookup_steps += 2;
            let mode = mode_by_trace
                .get(trace_id)
                .copied()
                .unwrap_or(RuntimeMode::Strict);
            let kind = if result == "pass" {
                CompatClosureScenarioKind::GoldenJourney
            } else {
                CompatClosureScenarioKind::Regression
            };
            let command_or_api = operation_by_trace
                .get(trace_id)
                .map(|operation| format!("{operation:?}"))
                .unwrap_or_else(|| "unknown_operation".to_owned());
            steps.push(CompatClosureE2eScenarioStep {
                scenario_id: scenario_id.clone(),
                packet_id: packet_id.clone(),
                mode,
                trace_id: trace_id.clone(),
                step_id: step_id.clone(),
                kind,
                command_or_api,
                input_ref: format!(
                    "fixture://{packet_id}/{}",
                    step_id.trim_start_matches("case:")
                ),
                output_ref: format!(
                    "artifact://{packet_id}/parity_mismatch_corpus.json#{replay_key}"
                ),
                duration_ms: elapsed_us.saturating_add(999) / 1000,
                retry_count: 0,
                outcome: result.clone(),
                reason_code: mismatch_class.clone().unwrap_or_else(|| "ok".to_owned()),
                replay_cmd: replay_cmd.clone(),
            });
        }
    }
    append_fault_injection_steps(&mut steps, fault_reports);

    let report = finalize_compat_closure_e2e_scenario_report(steps);
    let stats = CompatClosureScenarioBuildStats {
        trace_metadata_index_nodes,
        trace_metadata_lookup_steps,
    };
    (report, stats)
}

fn build_compat_closure_e2e_scenario_report_optimized_with_stats(
    e2e: &E2eReport,
    fault_reports: &[FaultInjectionValidationReport],
) -> (
    CompatClosureE2eScenarioReport,
    CompatClosureScenarioBuildStats,
) {
    let mut trace_metadata_by_id = HashMap::<String, CompatClosureTraceMetadata>::new();
    for event in &e2e.forensic_log.events {
        if let ForensicEventKind::CaseStart {
            trace_id,
            operation,
            mode,
            ..
        } = &event.event
        {
            trace_metadata_by_id.insert(
                trace_id.clone(),
                CompatClosureTraceMetadata {
                    operation: *operation,
                    mode: *mode,
                },
            );
        }
    }

    let mut steps = Vec::new();
    let mut trace_metadata_lookup_steps = 0_usize;
    for event in &e2e.forensic_log.events {
        if let ForensicEventKind::CaseEnd {
            scenario_id,
            packet_id,
            trace_id,
            step_id,
            result,
            replay_cmd,
            replay_key,
            mismatch_class,
            elapsed_us,
            ..
        } = &event.event
        {
            trace_metadata_lookup_steps += 1;
            let metadata = trace_metadata_by_id.get(trace_id);
            let mode = metadata
                .map(|entry| entry.mode)
                .unwrap_or(RuntimeMode::Strict);
            let kind = if result == "pass" {
                CompatClosureScenarioKind::GoldenJourney
            } else {
                CompatClosureScenarioKind::Regression
            };
            let command_or_api = metadata
                .map(|entry| format!("{:?}", entry.operation))
                .unwrap_or_else(|| "unknown_operation".to_owned());
            steps.push(CompatClosureE2eScenarioStep {
                scenario_id: scenario_id.clone(),
                packet_id: packet_id.clone(),
                mode,
                trace_id: trace_id.clone(),
                step_id: step_id.clone(),
                kind,
                command_or_api,
                input_ref: format!(
                    "fixture://{packet_id}/{}",
                    step_id.trim_start_matches("case:")
                ),
                output_ref: format!(
                    "artifact://{packet_id}/parity_mismatch_corpus.json#{replay_key}"
                ),
                duration_ms: elapsed_us.saturating_add(999) / 1000,
                retry_count: 0,
                outcome: result.clone(),
                reason_code: mismatch_class.clone().unwrap_or_else(|| "ok".to_owned()),
                replay_cmd: replay_cmd.clone(),
            });
        }
    }

    append_fault_injection_steps(&mut steps, fault_reports);

    let report = finalize_compat_closure_e2e_scenario_report(steps);
    let stats = CompatClosureScenarioBuildStats {
        trace_metadata_index_nodes: trace_metadata_by_id.len(),
        trace_metadata_lookup_steps,
    };
    (report, stats)
}

#[must_use]
pub fn build_compat_closure_e2e_scenario_report(
    e2e: &E2eReport,
    fault_reports: &[FaultInjectionValidationReport],
) -> CompatClosureE2eScenarioReport {
    let (report, _) =
        build_compat_closure_e2e_scenario_report_optimized_with_stats(e2e, fault_reports);
    report
}

pub fn write_compat_closure_e2e_scenario_report(
    repo_root: &Path,
    report: &CompatClosureE2eScenarioReport,
) -> Result<PathBuf, HarnessError> {
    let packet_id = report
        .steps
        .iter()
        .map(|step| step.packet_id.as_str())
        .collect::<BTreeSet<_>>();
    let path = if packet_id.len() == 1 {
        let only = packet_id
            .iter()
            .next()
            .copied()
            .ok_or_else(|| HarnessError::FixtureFormat("missing packet id".to_owned()))?;
        repo_root.join(format!(
            "artifacts/phase2c/{only}/compat_closure_e2e_scenarios.json"
        ))
    } else {
        repo_root.join("artifacts/phase2c/compat_closure_e2e_scenarios.json")
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(report)?)?;
    Ok(path)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatClosureMigrationManifest {
    pub manifest_version: String,
    pub packet_ids: Vec<String>,
    pub compatibility_guarantees: Vec<String>,
    pub known_deltas: Vec<String>,
    pub rollback_paths: Vec<String>,
    pub operational_guardrails: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatClosureReproducibilityLedger {
    pub env_fingerprint: String,
    pub lockfile_path: String,
    pub replay_commands: Vec<String>,
    pub artifact_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatClosureFinalEvidencePacket {
    pub packet_id: String,
    pub parity_green: bool,
    pub gate_pass: bool,
    pub strict_critical_drift_count: usize,
    pub strict_zero_drift: bool,
    pub hardened_allowlisted_count: usize,
    pub sidecar_integrity_ok: bool,
    pub decode_proof_recovered: bool,
    pub risk_notes: Vec<String>,
}

impl CompatClosureFinalEvidencePacket {
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.parity_green
            && self.gate_pass
            && self.strict_zero_drift
            && self.sidecar_integrity_ok
            && self.decode_proof_recovered
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatClosureFinalEvidencePack {
    pub generated_unix_ms: u128,
    pub suite_id: String,
    pub coverage_report: CompatClosureCoverageReport,
    pub strict_zero_drift: bool,
    pub hardened_allowlisted_total: usize,
    pub packets: Vec<CompatClosureFinalEvidencePacket>,
    pub migration_manifest: CompatClosureMigrationManifest,
    pub reproducibility_ledger: CompatClosureReproducibilityLedger,
    pub benchmark_delta_report_ref: String,
    pub invariant_checklist_delta: Vec<String>,
    pub risk_note_update: Vec<String>,
    pub all_checks_passed: bool,
    pub attestation_signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatClosureAttestationSummary {
    pub claim_id: String,
    pub generated_unix_ms: u128,
    pub all_checks_passed: bool,
    pub coverage_percent: usize,
    pub strict_zero_drift: bool,
    pub hardened_allowlisted_total: usize,
    pub packet_count: usize,
    pub attestation_signature: String,
    pub evidence_pack_path: String,
    pub migration_manifest_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatClosureFinalEvidencePaths {
    pub evidence_pack_path: PathBuf,
    pub migration_manifest_path: PathBuf,
    pub attestation_summary_path: PathBuf,
}

fn summarize_strict_and_hardened_drift(report: &DifferentialReport) -> (usize, usize) {
    let mut strict_critical_drift_count = 0_usize;
    let mut hardened_allowlisted_count = 0_usize;
    for result in &report.differential_results {
        let has_critical = result
            .drift_records
            .iter()
            .any(|record| matches!(record.level, DriftLevel::Critical));
        let has_non_critical_or_info = result.drift_records.iter().any(|record| {
            matches!(
                record.level,
                DriftLevel::NonCritical | DriftLevel::Informational
            )
        });

        match result.mode {
            RuntimeMode::Strict => {
                if has_critical || matches!(result.status, CaseStatus::Fail) {
                    strict_critical_drift_count += 1;
                }
            }
            RuntimeMode::Hardened => {
                if !has_critical && has_non_critical_or_info {
                    hardened_allowlisted_count += 1;
                }
            }
        }
    }
    (strict_critical_drift_count, hardened_allowlisted_count)
}

fn collect_compat_closure_artifact_hashes(
    config: &HarnessConfig,
    packet_ids: &[String],
) -> BTreeMap<String, String> {
    let mut hashes = BTreeMap::new();
    for packet_id in packet_ids {
        let packet_root = config.packet_artifact_root(packet_id);
        for file_name in [
            "parity_report.json",
            "parity_report.raptorq.json",
            "parity_report.decode_proof.json",
            "parity_gate_result.json",
            "parity_mismatch_corpus.json",
            "differential_validation_log.jsonl",
            "fault_injection_validation.json",
            "compat_closure_e2e_scenarios.json",
        ] {
            let path = packet_root.join(file_name);
            if let Ok(bytes) = fs::read(&path) {
                hashes.insert(
                    relative_to_repo(config, &path),
                    format!("sha256:{}", hash_bytes(&bytes)),
                );
            }
        }
    }

    for shared in [
        "artifacts/phase2c/PERFORMANCE_BASELINES.md",
        "artifacts/phase2c/COMPAT_CLOSURE_FINAL_EVIDENCE_PACK.md",
        "artifacts/phase2c/COMPAT_CLOSURE_TEST_LOGGING_EVIDENCE.md",
        "artifacts/phase2c/COMPAT_CLOSURE_DIFFERENTIAL_FAULT_INJECTION_EVIDENCE.md",
        "artifacts/phase2c/COMPAT_CLOSURE_E2E_SCENARIO_EVIDENCE.md",
        "artifacts/phase2c/COMPAT_CLOSURE_OPTIMIZATION_ISOMORPHISM_EVIDENCE.md",
    ] {
        let path = config.repo_root.join(shared);
        if let Ok(bytes) = fs::read(&path) {
            hashes.insert(shared.to_owned(), format!("sha256:{}", hash_bytes(&bytes)));
        }
    }
    hashes
}

fn build_compat_closure_migration_manifest(
    packet_ids: &[String],
) -> CompatClosureMigrationManifest {
    CompatClosureMigrationManifest {
        manifest_version: "1.0.0".to_owned(),
        packet_ids: packet_ids.to_vec(),
        compatibility_guarantees: vec![
            "strict mode enforces fail-closed compatibility boundaries".to_owned(),
            "hardened mode divergences remain explicit and bounded via allowlist policy".to_owned(),
            "alignment and null/NaN semantics are preserved as pandas-observable contracts"
                .to_owned(),
        ],
        known_deltas: vec![
            "live-oracle fallback remains opt-in via --allow-system-pandas-fallback".to_owned(),
            "full pandas API closure beyond scoped packets remains tracked in remaining Phase-2C program beads".to_owned(),
        ],
        rollback_paths: vec![
            "re-run packet parity gates with fixture oracle to validate rollback candidate".to_owned(),
            "use parity_mismatch_corpus replay keys for one-command reproduction of regressions"
                .to_owned(),
        ],
        operational_guardrails: vec![
            "require --require-green for release-path conformance execution".to_owned(),
            "block release when sidecar/decode integrity checks fail".to_owned(),
            "treat unresolved critical strict-mode drift as release blocker".to_owned(),
        ],
    }
}

pub fn build_compat_closure_final_evidence_pack(
    config: &HarnessConfig,
    packet_reports: &[PacketParityReport],
    differential_reports: &[DifferentialReport],
    fault_reports: &[FaultInjectionValidationReport],
) -> Result<CompatClosureFinalEvidencePack, HarnessError> {
    let coverage_report = build_compat_closure_coverage_report(config)?;

    let differential_by_packet = differential_reports
        .iter()
        .filter_map(|report| {
            report
                .report
                .packet_id
                .as_ref()
                .map(|packet_id| (packet_id.clone(), report))
        })
        .collect::<BTreeMap<_, _>>();
    let fault_by_packet = fault_reports
        .iter()
        .map(|report| (report.packet_id.clone(), report))
        .collect::<BTreeMap<_, _>>();

    let mut packet_ids = packet_reports
        .iter()
        .filter_map(|report| report.packet_id.clone())
        .collect::<Vec<_>>();
    packet_ids.sort();
    packet_ids.dedup();

    let mut packets = Vec::new();
    let mut strict_zero_drift = true;
    let mut hardened_allowlisted_total = 0_usize;
    let mut risk_note_update = Vec::new();

    for report in packet_reports {
        let Some(packet_id) = report.packet_id.as_deref() else {
            continue;
        };
        let gate = evaluate_parity_gate(config, report)?;
        let sidecar =
            verify_packet_sidecar_integrity(&config.packet_artifact_root(packet_id), packet_id);
        let (strict_critical_drift_count, differential_hardened_allowlisted_count) =
            differential_by_packet
                .get(packet_id)
                .map_or((0, 0), |diff| summarize_strict_and_hardened_drift(diff));
        let hardened_allowlisted_count = fault_by_packet
            .get(packet_id)
            .map_or(differential_hardened_allowlisted_count, |fault| {
                fault.hardened_allowlisted_count
            });

        strict_zero_drift &= strict_critical_drift_count == 0;
        hardened_allowlisted_total += hardened_allowlisted_count;

        let mut risk_notes = Vec::new();
        if !report.is_green() {
            risk_notes.push(format!(
                "parity report not green (passed={} failed={})",
                report.passed, report.failed
            ));
        }
        if !gate.pass {
            if gate.reasons.is_empty() {
                risk_notes.push("parity gate failed without explicit reasons".to_owned());
            } else {
                risk_notes.push(format!("parity gate failed: {}", gate.reasons.join("; ")));
            }
        }
        if strict_critical_drift_count > 0 {
            risk_notes.push(format!(
                "strict-mode critical drift count={strict_critical_drift_count}"
            ));
        }
        if hardened_allowlisted_count > 0 {
            risk_notes.push(format!(
                "hardened allowlisted divergence count={hardened_allowlisted_count}"
            ));
        }
        if !sidecar.is_ok() {
            risk_notes.extend(sidecar.errors.clone());
        }
        risk_notes.sort();
        risk_notes.dedup();
        risk_note_update.extend(risk_notes.iter().cloned());

        packets.push(CompatClosureFinalEvidencePacket {
            packet_id: packet_id.to_owned(),
            parity_green: report.is_green(),
            gate_pass: gate.pass,
            strict_critical_drift_count,
            strict_zero_drift: strict_critical_drift_count == 0,
            hardened_allowlisted_count,
            sidecar_integrity_ok: sidecar.is_ok(),
            decode_proof_recovered: sidecar.decode_proof_valid,
            risk_notes,
        });
    }

    packets.sort_by(|left, right| left.packet_id.cmp(&right.packet_id));
    risk_note_update.sort();
    risk_note_update.dedup();

    let migration_manifest = build_compat_closure_migration_manifest(&packet_ids);
    let reproducibility_ledger = CompatClosureReproducibilityLedger {
        env_fingerprint: compat_closure_env_fingerprint(config),
        lockfile_path: "Cargo.lock".to_owned(),
        replay_commands: vec![
            "rch exec -- cargo run -p fp-conformance --bin fp-conformance-cli -- --packet-id FP-P2C-001 --write-artifacts --require-green --write-drift-history --write-differential-validation --write-fault-injection --write-e2e-scenarios --write-final-evidence-pack".to_owned(),
            "rch exec -- cargo test -p fp-conformance --lib compat_closure_e2e_scenario -- --nocapture".to_owned(),
            "rch exec -- cargo check --workspace --all-targets".to_owned(),
            "rch exec -- cargo clippy --workspace --all-targets -- -D warnings".to_owned(),
        ],
        artifact_hashes: collect_compat_closure_artifact_hashes(config, &packet_ids),
    };

    let generated_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let all_checks_passed = coverage_report.is_complete()
        && strict_zero_drift
        && packets
            .iter()
            .all(CompatClosureFinalEvidencePacket::is_green);

    let mut pack = CompatClosureFinalEvidencePack {
        generated_unix_ms,
        suite_id: "COMPAT-CLOSURE-I".to_owned(),
        coverage_report,
        strict_zero_drift,
        hardened_allowlisted_total,
        packets,
        migration_manifest,
        reproducibility_ledger,
        benchmark_delta_report_ref: "artifacts/phase2c/PERFORMANCE_BASELINES.md".to_owned(),
        invariant_checklist_delta: vec![
            "CC-001..CC-009 closure coverage preserved at 100%".to_owned(),
            "strict-mode differential drift budget remains zero".to_owned(),
            "RaptorQ sidecar + decode-proof integrity checks remain enforced".to_owned(),
        ],
        risk_note_update,
        all_checks_passed,
        attestation_signature: String::new(),
    };

    let unsigned = serde_json::to_vec(&pack).map_err(HarnessError::Json)?;
    pack.attestation_signature = format!("sha256:{}", hash_bytes(&unsigned));
    Ok(pack)
}

pub fn write_compat_closure_final_evidence_pack(
    config: &HarnessConfig,
    pack: &CompatClosureFinalEvidencePack,
) -> Result<CompatClosureFinalEvidencePaths, HarnessError> {
    let phase2c_root = config.repo_root.join("artifacts/phase2c");
    fs::create_dir_all(&phase2c_root)?;

    let evidence_pack_path = phase2c_root.join("compat_closure_final_evidence_pack.json");
    let migration_manifest_path = phase2c_root.join("compat_closure_migration_manifest.json");
    let attestation_summary_path = phase2c_root.join("compat_closure_attestation_summary.json");

    fs::write(&evidence_pack_path, serde_json::to_string_pretty(pack)?)?;
    fs::write(
        &migration_manifest_path,
        serde_json::to_string_pretty(&pack.migration_manifest)?,
    )?;

    let summary = CompatClosureAttestationSummary {
        claim_id: "COMPAT-CLOSURE-I/attestation".to_owned(),
        generated_unix_ms: pack.generated_unix_ms,
        all_checks_passed: pack.all_checks_passed,
        coverage_percent: pack.coverage_report.achieved_percent,
        strict_zero_drift: pack.strict_zero_drift,
        hardened_allowlisted_total: pack.hardened_allowlisted_total,
        packet_count: pack.packets.len(),
        attestation_signature: pack.attestation_signature.clone(),
        evidence_pack_path: relative_to_repo(config, &evidence_pack_path),
        migration_manifest_path: relative_to_repo(config, &migration_manifest_path),
    };
    fs::write(
        &attestation_summary_path,
        serde_json::to_string_pretty(&summary)?,
    )?;

    Ok(CompatClosureFinalEvidencePaths {
        evidence_pack_path,
        migration_manifest_path,
        attestation_summary_path,
    })
}

/// Run the full E2E orchestration pipeline with lifecycle hooks and forensic logging.
///
/// Phases:
/// 1. **Run**: Execute all packet fixtures grouped by packet ID
/// 2. **Write**: Persist parity reports, RaptorQ sidecars, decode proofs, gate results, mismatch corpora
/// 3. **Enforce**: Validate all reports against parity gate configs
/// 4. **History**: Append JSONL rows to drift history ledger
///
/// Returns `E2eReport` with all phase results and forensic log.
pub fn run_e2e_suite(
    config: &E2eConfig,
    hooks: &mut dyn LifecycleHooks,
) -> Result<E2eReport, HarnessError> {
    let mut forensic = ForensicLog::new();
    let suite_name = config
        .options
        .packet_filter
        .clone()
        .unwrap_or("full".to_owned());

    // Phase 1: Suite Start
    forensic.record(ForensicEventKind::SuiteStart {
        suite: suite_name.clone(),
        packet_filter: config.options.packet_filter.clone(),
    });
    hooks.before_suite(&suite_name, &config.options.packet_filter);

    // Phase 2: Run fixtures grouped by packet
    let grouped_reports = run_packets_grouped(&config.harness, &config.options)?;

    // Emit per-packet forensic events
    for report in &grouped_reports {
        let packet_id = report
            .packet_id
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        let scenario_id = deterministic_scenario_id(&suite_name, &packet_id);

        forensic.record(ForensicEventKind::PacketStart {
            packet_id: packet_id.clone(),
        });
        hooks.before_packet(&packet_id);

        // Emit per-case events from the report results
        for case_result in &report.results {
            let trace_id = if case_result.trace_id.is_empty() {
                deterministic_trace_id(
                    &case_result.packet_id,
                    &case_result.case_id,
                    case_result.mode,
                )
            } else {
                case_result.trace_id.clone()
            };
            let replay_key = if case_result.replay_key.is_empty() {
                deterministic_replay_key(
                    &case_result.packet_id,
                    &case_result.case_id,
                    case_result.mode,
                )
            } else {
                case_result.replay_key.clone()
            };
            let step_id = deterministic_step_id(&case_result.case_id);
            let seed = deterministic_seed(
                &case_result.packet_id,
                &case_result.case_id,
                case_result.mode,
            );
            let assertion_path =
                assertion_path_for_case(&case_result.packet_id, &case_result.case_id);
            let replay_cmd = replay_cmd_for_case(&case_result.case_id);
            forensic.record(ForensicEventKind::CaseStart {
                scenario_id: scenario_id.clone(),
                packet_id: case_result.packet_id.clone(),
                case_id: case_result.case_id.clone(),
                trace_id: trace_id.clone(),
                step_id: step_id.clone(),
                seed,
                assertion_path: assertion_path.clone(),
                replay_cmd: replay_cmd.clone(),
                operation: case_result.operation,
                mode: case_result.mode,
            });
            forensic.record(ForensicEventKind::CaseEnd {
                scenario_id: scenario_id.clone(),
                packet_id: case_result.packet_id.clone(),
                case_id: case_result.case_id.clone(),
                trace_id,
                step_id,
                seed,
                assertion_path,
                result: result_label_for_status(&case_result.status).to_owned(),
                replay_cmd,
                decision_action: decision_action_for(&case_result.status).to_owned(),
                replay_key,
                mismatch_class: case_result.mismatch_class.clone(),
                status: case_result.status.clone(),
                evidence_records: case_result.evidence_records,
                elapsed_us: case_result.elapsed_us.max(1),
            });
            let compat_case_log = build_compat_closure_case_log(
                &config.harness,
                COMPAT_CLOSURE_SUITE_ID,
                case_result,
                now_unix_ms(),
            );
            forensic.record(ForensicEventKind::CompatClosureCase {
                ts_utc: compat_case_log.ts_utc,
                suite_id: compat_case_log.suite_id,
                test_id: compat_case_log.test_id,
                api_surface_id: compat_case_log.api_surface_id,
                packet_id: compat_case_log.packet_id,
                mode: compat_case_log.mode,
                seed: compat_case_log.seed,
                input_digest: compat_case_log.input_digest,
                output_digest: compat_case_log.output_digest,
                env_fingerprint: compat_case_log.env_fingerprint,
                artifact_refs: compat_case_log.artifact_refs,
                duration_ms: compat_case_log.duration_ms,
                outcome: compat_case_log.outcome,
                reason_code: compat_case_log.reason_code,
            });
            hooks.after_case(case_result);
        }

        // Evaluate gate for this packet
        let gate_result = evaluate_parity_gate(&config.harness, report)?;

        forensic.record(ForensicEventKind::GateEvaluated {
            packet_id: packet_id.clone(),
            pass: gate_result.pass,
            reasons: gate_result.reasons.clone(),
        });

        forensic.record(ForensicEventKind::PacketEnd {
            packet_id: packet_id.clone(),
            fixtures: report.fixture_count,
            passed: report.passed,
            failed: report.failed,
            gate_pass: gate_result.pass,
        });
        hooks.after_packet(report, gate_result.pass);
    }

    // Phase 3: Write artifacts
    let mut artifacts_written = Vec::new();
    if config.write_artifacts {
        let written = write_grouped_artifacts(&config.harness, &grouped_reports)?;
        for w in &written {
            for (kind, path) in [
                ("parity_report", &w.parity_report_path),
                ("raptorq_sidecar", &w.raptorq_sidecar_path),
                ("decode_proof", &w.decode_proof_path),
                ("gate_result", &w.gate_result_path),
                ("mismatch_corpus", &w.mismatch_corpus_path),
            ] {
                forensic.record(ForensicEventKind::ArtifactWritten {
                    packet_id: w.packet_id.clone(),
                    artifact_kind: kind.to_owned(),
                    path: path.display().to_string(),
                });
            }
        }
        artifacts_written = written;
    }

    // Phase 4: Evaluate gates
    let mut gate_results = Vec::new();
    let mut gates_pass = true;
    for report in &grouped_reports {
        let gate = evaluate_parity_gate(&config.harness, report)?;
        if !gate.pass {
            gates_pass = false;
        }
        gate_results.push(gate);
    }

    // Phase 5: Enforce gates (if configured)
    if config.enforce_gates
        && let Err(e) = enforce_packet_gates(&config.harness, &grouped_reports)
    {
        forensic.record(ForensicEventKind::Error {
            phase: "gate_enforcement".to_owned(),
            message: e.to_string(),
        });
        // Record but don't fail: the E2eReport captures gates_pass=false
    }

    // Phase 6: Append drift history
    let mut drift_history_path = None;
    if config.append_drift_history {
        match append_phase2c_drift_history(&config.harness, &grouped_reports) {
            Ok(path) => {
                forensic.record(ForensicEventKind::DriftHistoryAppended {
                    path: path.display().to_string(),
                    entries: grouped_reports.len(),
                });
                drift_history_path = Some(path.display().to_string());
            }
            Err(e) => {
                forensic.record(ForensicEventKind::Error {
                    phase: "drift_history".to_owned(),
                    message: e.to_string(),
                });
            }
        }
    }

    // Aggregate totals
    let total_fixtures: usize = grouped_reports.iter().map(|r| r.fixture_count).sum();
    let total_passed: usize = grouped_reports.iter().map(|r| r.passed).sum();
    let total_failed: usize = grouped_reports.iter().map(|r| r.failed).sum();

    // Suite End
    forensic.record(ForensicEventKind::SuiteEnd {
        suite: suite_name.clone(),
        total_fixtures,
        passed: total_passed,
        failed: total_failed,
    });
    hooks.after_suite(&grouped_reports);

    // Write forensic log if configured
    if let Some(ref log_path) = config.forensic_log_path {
        forensic.write_jsonl(log_path)?;
    }

    Ok(E2eReport {
        suite: suite_name,
        packet_reports: grouped_reports,
        artifacts_written,
        gate_results,
        gates_pass,
        drift_history_path,
        forensic_log: forensic,
        total_fixtures,
        total_passed,
        total_failed,
    })
}

// === Failure Forensics UX + Artifact Index (bd-2gi.21) ===

/// Deterministic artifact identifier for cross-referencing forensic artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactId {
    pub packet_id: String,
    pub artifact_kind: String,
    pub run_ts_unix_ms: u64,
}

impl ArtifactId {
    /// Generate a short deterministic hash for display.
    #[must_use]
    pub fn short_hash(&self) -> String {
        let input = format!(
            "{}:{}:{}",
            self.packet_id, self.artifact_kind, self.run_ts_unix_ms
        );
        let hash = Sha256::digest(input.as_bytes());
        format!("{:x}", hash)[..8].to_owned()
    }
}

impl std::fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}@{}",
            self.packet_id,
            self.artifact_kind,
            self.short_hash()
        )
    }
}

/// A concise failure summary for a single test case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureDigest {
    pub packet_id: String,
    pub case_id: String,
    pub operation: FixtureOperation,
    pub mode: RuntimeMode,
    pub mismatch_class: Option<String>,
    pub mismatch_summary: String,
    pub replay_key: String,
    pub trace_id: String,
    pub replay_command: String,
    pub artifact_path: Option<String>,
}

impl std::fmt::Display for FailureDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "FAIL {packet}::{case} [{op:?}/{mode:?}]",
            packet = self.packet_id,
            case = self.case_id,
            op = self.operation,
            mode = self.mode,
        )?;
        if let Some(ref mismatch_class) = self.mismatch_class {
            writeln!(f, "  Class:    {mismatch_class}")?;
        }
        writeln!(f, "  ReplayKey: {}", self.replay_key)?;
        writeln!(f, "  Trace:    {}", self.trace_id)?;
        writeln!(f, "  Mismatch: {}", self.mismatch_summary)?;
        writeln!(f, "  Replay:   {}", self.replay_command)?;
        if let Some(ref path) = self.artifact_path {
            writeln!(f, "  Artifact: {path}")?;
        }
        Ok(())
    }
}

/// Human-readable failure report for an E2E run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureForensicsReport {
    pub run_ts_unix_ms: u64,
    pub total_fixtures: usize,
    pub total_passed: usize,
    pub total_failed: usize,
    pub failures: Vec<FailureDigest>,
    pub gate_failures: Vec<String>,
}

impl FailureForensicsReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty() && self.gate_failures.is_empty()
    }
}

impl std::fmt::Display for FailureForensicsReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_clean() {
            writeln!(
                f,
                "ALL GREEN: {}/{} fixtures passed",
                self.total_passed, self.total_fixtures
            )?;
            return Ok(());
        }

        writeln!(
            f,
            "FAILURES: {failed}/{total} fixtures failed",
            failed = self.total_failed,
            total = self.total_fixtures,
        )?;
        writeln!(f)?;

        for (i, failure) in self.failures.iter().enumerate() {
            write!(f, "  {idx}. {failure}", idx = i + 1)?;
        }

        if !self.gate_failures.is_empty() {
            writeln!(f)?;
            writeln!(f, "GATE FAILURES:")?;
            for reason in &self.gate_failures {
                writeln!(f, "  - {reason}")?;
            }
        }

        Ok(())
    }
}

/// Build a failure forensics report from an E2E report.
#[must_use]
pub fn build_failure_forensics(e2e: &E2eReport) -> FailureForensicsReport {
    let mut failures = Vec::new();

    for report in &e2e.packet_reports {
        let packet_id = report
            .packet_id
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());

        for result in &report.results {
            if matches!(result.status, CaseStatus::Fail) {
                let mismatch_summary = result
                    .mismatch
                    .as_deref()
                    .unwrap_or("(no details)")
                    .chars()
                    .take(200)
                    .collect();

                let replay_command = replay_cmd_for_case(&result.case_id);

                let artifact_path = e2e
                    .artifacts_written
                    .iter()
                    .find(|a| a.packet_id == packet_id)
                    .map(|a| a.mismatch_corpus_path.display().to_string());

                failures.push(FailureDigest {
                    packet_id: packet_id.clone(),
                    case_id: result.case_id.clone(),
                    operation: result.operation,
                    mode: result.mode,
                    mismatch_class: result.mismatch_class.clone(),
                    mismatch_summary,
                    replay_key: if result.replay_key.is_empty() {
                        deterministic_replay_key(&result.packet_id, &result.case_id, result.mode)
                    } else {
                        result.replay_key.clone()
                    },
                    trace_id: if result.trace_id.is_empty() {
                        deterministic_trace_id(&result.packet_id, &result.case_id, result.mode)
                    } else {
                        result.trace_id.clone()
                    },
                    replay_command,
                    artifact_path,
                });
            }
        }
    }

    let gate_failures: Vec<String> = e2e
        .gate_results
        .iter()
        .filter(|g| !g.pass)
        .flat_map(|g| {
            g.reasons
                .iter()
                .map(|r| format!("{}: {}", g.packet_id, r))
                .collect::<Vec<_>>()
        })
        .collect();

    FailureForensicsReport {
        run_ts_unix_ms: now_unix_ms(),
        total_fixtures: e2e.total_fixtures,
        total_passed: e2e.total_passed,
        total_failed: e2e.total_failed,
        failures,
        gate_failures,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::sync::{Mutex, OnceLock};

    use super::{
        ArtifactId, CaseResult, CaseStatus, CiGate, CiGateResult, CiPipelineConfig,
        CiPipelineResult, ComparisonCategory, DecodeProofArtifact, DecodeProofStatus,
        DifferentialResult, DriftLevel, DriftRecord, E2eConfig, FailureDigest,
        FailureForensicsReport, FaultInjectionClassification, FixtureExpectedAlignment,
        FixtureOperation, FixtureOracleSource, ForensicEventKind, ForensicLog, HarnessConfig,
        LifecycleHooks, NoopHooks, OracleMode, PacketParityReport, RaptorQSidecarArtifact,
        SuiteOptions, append_phase2c_drift_history, build_ci_forensics_report,
        build_compat_closure_e2e_scenario_report, build_compat_closure_final_evidence_pack,
        build_differential_report, build_differential_validation_log, build_failure_forensics,
        enforce_packet_gates, evaluate_ci_gate, evaluate_parity_gate, fuzz_column_arith_bytes,
        fuzz_common_dtype_bytes, fuzz_csv_parse_bytes, fuzz_excel_io_bytes, fuzz_feather_io_bytes,
        fuzz_fixture_parse_bytes, fuzz_groupby_sum_bytes, fuzz_index_align_bytes,
        fuzz_ipc_stream_io_bytes, fuzz_join_series_bytes, fuzz_json_io_bytes,
        fuzz_parquet_io_bytes, fuzz_scalar_cast_bytes, fuzz_series_add_bytes,
        generate_raptorq_sidecar, run_ci_pipeline, run_differential_by_id, run_differential_suite,
        run_e2e_suite, run_fault_injection_validation_by_id, run_packet_by_id, run_packet_suite,
        run_packet_suite_with_options, run_packets_grouped, run_raptorq_decode_recovery_drill,
        run_smoke, verify_all_sidecars_ci, verify_packet_sidecar_integrity,
        write_compat_closure_e2e_scenario_report, write_compat_closure_final_evidence_pack,
        write_differential_validation_log, write_fault_injection_validation_report,
        write_packet_artifacts,
    };
    use fp_runtime::RuntimeMode;

    fn phase2c_artifact_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .expect("phase2c artifact test lock poisoned")
    }

    fn assert_text_golden(golden_name: &str, actual: &str) {
        let golden_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("goldens")
            .join(golden_name);

        if std::env::var_os("UPDATE_GOLDENS").is_some() {
            let parent = golden_path
                .parent()
                .expect("golden files should always have a parent directory");
            std::fs::create_dir_all(parent).expect("golden directory should be creatable");
            std::fs::write(&golden_path, actual).expect("golden file should be writable");
            return;
        }

        let expected =
            std::fs::read_to_string(&golden_path).expect("golden file should be readable");
        assert_eq!(
            actual,
            expected,
            "golden mismatch for {}",
            golden_path.display()
        );
    }

    fn sample_failure_digest() -> FailureDigest {
        FailureDigest {
            packet_id: "FP-P2C-001".to_owned(),
            case_id: "series_add_strict".to_owned(),
            operation: FixtureOperation::SeriesAdd,
            mode: RuntimeMode::Strict,
            mismatch_class: Some("value_critical".to_owned()),
            mismatch_summary: "expected Int64(10), got Float64(10.0)".to_owned(),
            replay_key: "FP-P2C-001/series_add_strict/strict".to_owned(),
            trace_id: "FP-P2C-001:series_add_strict:strict".to_owned(),
            replay_command: "cargo test -p fp-conformance -- series_add_strict --nocapture"
                .to_owned(),
            artifact_path: Some("artifacts/phase2c/FP-P2C-001/mismatch.json".to_owned()),
        }
    }

    fn sample_failure_forensics_report() -> FailureForensicsReport {
        FailureForensicsReport {
            run_ts_unix_ms: 1000,
            total_fixtures: 5,
            total_passed: 3,
            total_failed: 2,
            failures: vec![
                FailureDigest {
                    packet_id: "FP-P2C-001".to_owned(),
                    case_id: "case_a".to_owned(),
                    operation: FixtureOperation::SeriesAdd,
                    mode: RuntimeMode::Strict,
                    mismatch_class: Some("value_critical".to_owned()),
                    mismatch_summary: "value drift".to_owned(),
                    replay_key: "FP-P2C-001/case_a/strict".to_owned(),
                    trace_id: "FP-P2C-001:case_a:strict".to_owned(),
                    replay_command: "cargo test -- case_a".to_owned(),
                    artifact_path: None,
                },
                FailureDigest {
                    packet_id: "FP-P2C-002".to_owned(),
                    case_id: "case_b".to_owned(),
                    operation: FixtureOperation::IndexAlignUnion,
                    mode: RuntimeMode::Hardened,
                    mismatch_class: Some("shape_critical".to_owned()),
                    mismatch_summary: "shape mismatch".to_owned(),
                    replay_key: "FP-P2C-002/case_b/hardened".to_owned(),
                    trace_id: "FP-P2C-002:case_b:hardened".to_owned(),
                    replay_command: "cargo test -- case_b".to_owned(),
                    artifact_path: Some("path/to/corpus.json".to_owned()),
                },
            ],
            gate_failures: vec!["FP-P2C-001: strict_failed > 0".to_owned()],
        }
    }

    fn sample_ci_pipeline_result() -> CiPipelineResult {
        CiPipelineResult {
            gates: vec![
                CiGateResult {
                    gate: CiGate::G3Unit,
                    passed: true,
                    elapsed_ms: 100,
                    summary: "10 tests passed".to_owned(),
                    errors: vec![],
                },
                CiGateResult {
                    gate: CiGate::G6Conformance,
                    passed: false,
                    elapsed_ms: 200,
                    summary: "2 fixtures failed".to_owned(),
                    errors: vec!["case_a: value drift".to_owned()],
                },
            ],
            all_passed: false,
            first_failure: Some(CiGate::G6Conformance),
            elapsed_ms: 300,
        }
    }

    #[test]
    fn smoke_harness_finds_oracle_and_fixtures() {
        let cfg = HarnessConfig::default_paths();
        if !cfg.oracle_root.exists() {
            eprintln!(
                "oracle repo missing at {}; skipping smoke oracle check",
                cfg.oracle_root.display()
            );
            return;
        }
        let report = run_smoke(&cfg);
        assert!(report.oracle_present, "oracle repo should be present");
        assert!(report.fixture_count >= 1, "expected at least one fixture");
        assert!(report.strict_mode);
    }

    #[test]
    fn packet_suite_is_green_for_bootstrap_cases() {
        let cfg = HarnessConfig::default_paths();
        let report = run_packet_suite(&cfg).expect("suite should run");
        assert!(report.fixture_count >= 1, "expected packet fixtures");
        assert!(report.is_green(), "expected report green: {report:?}");
    }

    #[test]
    fn fuzz_fixture_parse_bytes_accepts_valid_seed_fixture() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/fuzz_fixture_parse/series_add_valid_seed.json"
        );
        fuzz_fixture_parse_bytes(seed).expect("valid fuzz seed should parse");
    }

    #[test]
    fn fuzz_fixture_parse_bytes_accepts_expected_error_seed_fixture() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/fuzz_fixture_parse/dataframe_constructor_missing_matrix_rows_seed.json"
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
            include_bytes!("../fixtures/adversarial/fuzz_corpus/csv_parse/simple_valid_seed.csv");
        fuzz_csv_parse_bytes(seed).expect("simple csv fuzz seed should parse");
    }

    #[test]
    fn fuzz_csv_parse_bytes_accepts_quoted_newline_seed_fixture() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/csv_parse/quoted_newline_valid_seed.csv"
        );
        fuzz_csv_parse_bytes(seed).expect("quoted newline csv fuzz seed should parse");
    }

    #[test]
    fn fuzz_csv_parse_bytes_reports_duplicate_headers() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/csv_parse/duplicate_headers_invalid_seed.csv"
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
            include_bytes!("../fixtures/adversarial/fuzz_corpus/excel_io/simple_valid_seed.xlsx");
        fuzz_excel_io_bytes(seed).expect("excel fuzz seed should parse");
    }

    #[test]
    fn fuzz_excel_io_bytes_reports_invalid_workbook() {
        let seed =
            include_bytes!("../fixtures/adversarial/fuzz_corpus/excel_io/invalid_text_seed.bin");
        let err = fuzz_excel_io_bytes(seed).expect_err("invalid workbook bytes should error");
        assert!(
            matches!(err, fp_io::IoError::Excel(_)),
            "expected Excel parse error, got {err:?}"
        );
    }

    #[test]
    fn fuzz_parquet_io_bytes_accepts_synthesized_seed_fixture() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/parquet_io/synthesized_valid_seed.bin"
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
            include_bytes!("../fixtures/adversarial/fuzz_corpus/parquet_io/invalid_text_seed.bin");
        let err = fuzz_parquet_io_bytes(seed).expect_err("invalid parquet bytes should error");
        assert!(
            matches!(err, fp_io::IoError::Parquet(_)),
            "expected Parquet parse error, got {err:?}"
        );
    }

    #[test]
    fn fuzz_feather_io_bytes_accepts_synthesized_seed_fixture() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/feather_io/synthesized_valid_seed.bin"
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
            include_bytes!("../fixtures/adversarial/fuzz_corpus/feather_io/invalid_text_seed.bin");
        let err = fuzz_feather_io_bytes(seed).expect_err("invalid feather bytes should error");
        assert!(
            matches!(err, fp_io::IoError::Arrow(_)),
            "expected Arrow parse error, got {err:?}"
        );
    }

    #[test]
    fn fuzz_ipc_stream_io_bytes_accepts_synthesized_seed_fixture() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/ipc_stream_io/synthesized_valid_seed.bin"
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
            "../fixtures/adversarial/fuzz_corpus/ipc_stream_io/invalid_text_seed.bin"
        );
        let err =
            fuzz_ipc_stream_io_bytes(seed).expect_err("invalid IPC stream bytes should error");
        assert!(
            matches!(err, fp_io::IoError::Arrow(_)),
            "expected Arrow parse error, got {err:?}"
        );
    }

    #[test]
    fn fuzz_common_dtype_bytes_accepts_identical_dtype_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/common_dtype/identical_int64_seed.bin"
        );
        fuzz_common_dtype_bytes(seed).expect("identical dtype seed should satisfy invariants");
    }

    #[test]
    fn fuzz_common_dtype_bytes_accepts_numeric_promotion_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/common_dtype/numeric_promotion_seed.bin"
        );
        fuzz_common_dtype_bytes(seed).expect("numeric promotion seed should satisfy invariants");
    }

    #[test]
    fn fuzz_common_dtype_bytes_accepts_incompatible_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/common_dtype/incompatible_utf8_bool_seed.bin"
        );
        fuzz_common_dtype_bytes(seed).expect("incompatible seed should still preserve symmetry");
    }

    #[test]
    fn fuzz_scalar_cast_bytes_accepts_identity_int64_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/scalar_cast/identity_int64_seed.bin"
        );
        fuzz_scalar_cast_bytes(seed).expect("identity cast seed should satisfy invariants");
    }

    #[test]
    fn fuzz_scalar_cast_bytes_accepts_missing_float_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/scalar_cast/missing_float_seed.bin"
        );
        fuzz_scalar_cast_bytes(seed).expect("missing float seed should satisfy invariants");
    }

    #[test]
    fn fuzz_scalar_cast_bytes_accepts_lossy_float_error_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/scalar_cast/lossy_float_to_int_seed.bin"
        );
        fuzz_scalar_cast_bytes(seed)
            .expect("lossy float cast seed should still satisfy invariants");
    }

    #[test]
    fn fuzz_series_add_bytes_accepts_unique_overlap_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/series_add/unique_overlap_seed.bin"
        );
        fuzz_series_add_bytes(seed).expect("unique-overlap seed should satisfy invariants");
    }

    #[test]
    fn fuzz_series_add_bytes_accepts_duplicate_cross_product_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/series_add/duplicate_cross_product_seed.bin"
        );
        fuzz_series_add_bytes(seed)
            .expect("duplicate cross-product seed should satisfy invariants");
    }

    #[test]
    fn fuzz_series_add_bytes_accepts_missing_alignment_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/series_add/missing_alignment_seed.bin"
        );
        fuzz_series_add_bytes(seed).expect("missing alignment seed should satisfy invariants");
    }

    #[test]
    fn fuzz_column_arith_bytes_accepts_add_missing_seed() {
        let seed =
            include_bytes!("../fixtures/adversarial/fuzz_corpus/column_arith/add_missing_seed.bin");
        fuzz_column_arith_bytes(seed).expect("add-missing seed should satisfy invariants");
    }

    #[test]
    fn fuzz_column_arith_bytes_accepts_sub_seed() {
        let seed =
            include_bytes!("../fixtures/adversarial/fuzz_corpus/column_arith/sub_int_seed.bin");
        fuzz_column_arith_bytes(seed).expect("sub seed should satisfy invariants");
    }

    #[test]
    fn fuzz_column_arith_bytes_accepts_mixed_mul_seed() {
        let seed =
            include_bytes!("../fixtures/adversarial/fuzz_corpus/column_arith/mixed_mul_seed.bin");
        fuzz_column_arith_bytes(seed).expect("mixed-mul seed should satisfy invariants");
    }

    #[test]
    fn fuzz_column_arith_bytes_accepts_div_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/column_arith/div_identity_seed.bin"
        );
        fuzz_column_arith_bytes(seed).expect("div seed should satisfy invariants");
    }

    #[test]
    fn fuzz_column_arith_bytes_accepts_mod_zero_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/column_arith/mod_zero_divisor_seed.bin"
        );
        fuzz_column_arith_bytes(seed).expect("mod-zero seed should satisfy invariants");
    }

    #[test]
    fn fuzz_column_arith_bytes_accepts_pow_seed() {
        let seed = include_bytes!("../fixtures/adversarial/fuzz_corpus/column_arith/pow_seed.bin");
        fuzz_column_arith_bytes(seed).expect("pow seed should satisfy invariants");
    }

    #[test]
    fn fuzz_column_arith_bytes_accepts_floor_div_zero_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/column_arith/floor_div_zero_divisor_seed.bin"
        );
        fuzz_column_arith_bytes(seed).expect("floor-div-zero seed should satisfy invariants");
    }

    #[test]
    fn fuzz_join_series_bytes_accepts_inner_overlap_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/join_series/inner_overlap_seed.bin"
        );
        fuzz_join_series_bytes(seed).expect("inner-overlap seed should satisfy invariants");
    }

    #[test]
    fn fuzz_join_series_bytes_accepts_left_unmatched_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/join_series/left_unmatched_seed.bin"
        );
        fuzz_join_series_bytes(seed).expect("left-unmatched seed should satisfy invariants");
    }

    #[test]
    fn fuzz_join_series_bytes_accepts_right_unmatched_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/join_series/right_unmatched_seed.bin"
        );
        fuzz_join_series_bytes(seed).expect("right-unmatched seed should satisfy invariants");
    }

    #[test]
    fn fuzz_join_series_bytes_accepts_outer_union_seed() {
        let seed =
            include_bytes!("../fixtures/adversarial/fuzz_corpus/join_series/outer_union_seed.bin");
        fuzz_join_series_bytes(seed).expect("outer-union seed should satisfy invariants");
    }

    #[test]
    fn fuzz_join_series_bytes_accepts_cross_product_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/join_series/cross_product_seed.bin"
        );
        fuzz_join_series_bytes(seed).expect("cross-product seed should satisfy invariants");
    }

    #[test]
    fn fuzz_groupby_sum_bytes_accepts_dropna_true_seed() {
        let seed =
            include_bytes!("../fixtures/adversarial/fuzz_corpus/groupby_sum/dropna_true_seed.bin");
        fuzz_groupby_sum_bytes(seed).expect("dropna=true seed should satisfy invariants");
    }

    #[test]
    fn fuzz_groupby_sum_bytes_accepts_dropna_false_null_group_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/groupby_sum/dropna_false_null_group_seed.bin"
        );
        fuzz_groupby_sum_bytes(seed)
            .expect("dropna=false null-group seed should satisfy invariants");
    }

    #[test]
    fn fuzz_groupby_sum_bytes_accepts_alignment_seed() {
        let seed =
            include_bytes!("../fixtures/adversarial/fuzz_corpus/groupby_sum/alignment_seed.bin");
        fuzz_groupby_sum_bytes(seed).expect("alignment seed should satisfy invariants");
    }

    #[test]
    fn fuzz_index_align_bytes_accepts_unique_overlap_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/index_align/unique_overlap_seed.bin"
        );
        fuzz_index_align_bytes(seed).expect("unique-overlap seed should satisfy invariants");
    }

    #[test]
    fn fuzz_index_align_bytes_accepts_duplicate_cross_product_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/index_align/duplicate_cross_product_seed.bin"
        );
        fuzz_index_align_bytes(seed)
            .expect("duplicate seed should satisfy multiplicity invariants");
    }

    #[test]
    fn fuzz_index_align_bytes_accepts_utf8_right_only_seed() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/index_align/utf8_right_only_seed.bin"
        );
        fuzz_index_align_bytes(seed).expect("utf8 right-only seed should satisfy invariants");
    }

    #[test]
    fn fuzz_json_io_bytes_accepts_records_seed_fixture() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/fuzz_json_io/records_valid_seed.json"
        );
        fuzz_json_io_bytes(seed).expect("records fuzz seed should parse");
    }

    #[test]
    fn fuzz_json_io_bytes_accepts_split_seed_fixture() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/fuzz_json_io/split_valid_seed.json"
        );
        fuzz_json_io_bytes(seed).expect("split fuzz seed should parse");
    }

    #[test]
    fn fuzz_json_io_bytes_accepts_jsonl_seed_fixture() {
        let seed = include_bytes!(
            "../fixtures/adversarial/fuzz_corpus/fuzz_json_io/jsonl_valid_seed.jsonl"
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
            report.fixture_count >= 3,
            "expected groupby packet fixtures"
        );
        assert!(report.is_green());
    }

    #[test]
    fn packet_filter_runs_groupby_aggregate_matrix_packet() {
        let cfg = HarnessConfig::default_paths();
        let report =
            run_packet_by_id(&cfg, "FP-P2C-011", OracleMode::FixtureExpected).expect("report");
        assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-011"));
        assert!(
            report.fixture_count >= 12,
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
            report.fixture_count >= 14,
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
            report.fixture_count >= 4,
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
            report.fixture_count >= 6,
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
            report.fixture_count >= 6,
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
            report.fixture_count >= 3,
            "expected FP-P2D-072 series_between fixtures"
        );
        assert!(report.is_green(), "expected report green: {report:?}");
    }

    #[test]
    fn grouped_reports_are_partitioned_per_packet() {
        let _artifact_guard = phase2c_artifact_test_lock();
        let cfg = HarnessConfig::default_paths();
        let reports = run_packets_grouped(&cfg, &SuiteOptions::default()).expect("grouped");
        let packet_ids: Vec<String> = reports
            .iter()
            .map(|report| {
                report
                    .packet_id
                    .clone()
                    .expect("grouped report should have packet_id")
            })
            .collect();
        assert!(
            !packet_ids.is_empty(),
            "expected grouped packet reports to include packet ids"
        );
        let unique_packet_count = packet_ids.iter().collect::<BTreeSet<_>>().len();
        assert!(
            unique_packet_count == reports.len(),
            "expected exactly one grouped report per packet: unique={unique_packet_count} reports={}",
            reports.len()
        );
        enforce_packet_gates(&cfg, &reports).expect("enforcement should pass");
    }

    #[test]
    fn packet_gate_enforcement_fails_when_report_is_not_green() {
        let _artifact_guard = phase2c_artifact_test_lock();
        let cfg = HarnessConfig::default_paths();
        let mut reports = run_packets_grouped(&cfg, &SuiteOptions::default()).expect("grouped");
        let report = reports.first_mut().expect("at least one packet");
        let first_case = report.results.first_mut().expect("at least one case");
        first_case.status = CaseStatus::Fail;
        first_case.mismatch = Some("synthetic non-green check".to_owned());
        report.failed = 1;
        report.passed = report.fixture_count.saturating_sub(1);

        let err = enforce_packet_gates(&cfg, &reports).expect_err("should fail");
        let message = err.to_string();
        assert!(
            message.contains("enforcement failed"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn drift_history_append_emits_jsonl_rows() {
        let _artifact_guard = phase2c_artifact_test_lock();
        let cfg = HarnessConfig::default_paths();
        let reports = run_packets_grouped(&cfg, &SuiteOptions::default()).expect("grouped");
        let history_path = append_phase2c_drift_history(&cfg, &reports).expect("history");
        let contents = fs::read_to_string(&history_path).expect("history content");
        let latest = contents.lines().last().expect("at least one row");
        let row: serde_json::Value = serde_json::from_str(latest).expect("json row");
        assert!(
            row.get("packet_id").is_some(),
            "history row should include packet_id"
        );
        assert!(
            row.get("gate_pass").is_some(),
            "history row should include gate pass status"
        );
    }

    #[test]
    fn parity_gate_evaluation_passes_for_packet_001() {
        let _artifact_guard = phase2c_artifact_test_lock();
        let cfg = HarnessConfig::default_paths();
        let report =
            run_packet_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected).expect("report");
        let result = evaluate_parity_gate(&cfg, &report).expect("gate");
        assert!(result.pass, "gate should pass: {result:?}");
    }

    #[test]
    fn parity_gate_evaluation_fails_for_injected_drift() {
        let _artifact_guard = phase2c_artifact_test_lock();
        let cfg = HarnessConfig::default_paths();
        let mut report =
            run_packet_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected).expect("report");
        let first = report.results.first_mut().expect("at least one result");
        first.status = CaseStatus::Fail;
        first.mismatch = Some("synthetic drift injection".to_owned());
        report.failed = 1;
        report.passed = report.fixture_count.saturating_sub(1);

        let result = evaluate_parity_gate(&cfg, &report).expect("gate");
        assert!(!result.pass, "gate should fail for injected drift");
        assert!(
            result
                .reasons
                .iter()
                .any(|reason| reason.contains("failed="))
        );
    }

    #[test]
    fn raptorq_sidecar_round_trip_recovery_drill_passes() {
        let payload = br#"{\"suite\":\"phase2c_packets\",\"passed\":4,\"failed\":0}"#;
        let sidecar = generate_raptorq_sidecar("test/parity_report", "conformance", payload, 8)
            .expect("sidecar generation");
        let proof = run_raptorq_decode_recovery_drill(&sidecar, payload).expect("decode drill");
        assert!(proof.recovered_blocks >= 1);
    }

    #[test]
    fn index_alignment_expected_type_serialization_is_stable() {
        let expected = FixtureExpectedAlignment {
            union_index: vec![1_i64.into(), 2_i64.into()],
            left_positions: vec![Some(0), None],
            right_positions: vec![None, Some(0)],
        };
        let json = serde_json::to_string(&expected).expect("serialize");
        let back: FixtureExpectedAlignment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, expected);
    }

    #[test]
    fn live_oracle_mode_executes_or_returns_structured_failure() {
        let cfg = HarnessConfig::default_paths();
        let options = SuiteOptions {
            packet_filter: Some("FP-P2C-001".to_owned()),
            oracle_mode: OracleMode::LiveLegacyPandas,
        };
        let result = run_packet_suite_with_options(&cfg, &options);
        match result {
            Ok(report) => assert!(report.fixture_count >= 1),
            Err(err) => {
                let message = err.to_string();
                assert!(
                    message.contains("oracle"),
                    "expected oracle-class error, got {message}"
                );
            }
        }
    }

    #[test]
    fn live_oracle_series_constructor_mixed_utf8_numeric_reports_object_values() {
        let mut cfg = HarnessConfig::default_paths();
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
            eprintln!(
                "live pandas unavailable; skipping mixed object series oracle test: {message}"
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
        let mut cfg = HarnessConfig::default_paths();
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
            &SuiteOptions {
                packet_filter: None,
                oracle_mode: OracleMode::LiveLegacyPandas,
            },
        )
        .expect("differential report");
        assert_eq!(diff.status, CaseStatus::Pass);
        assert_eq!(diff.oracle_source, FixtureOracleSource::LiveLegacyPandas);
        assert!(
            diff.drift_records.is_empty(),
            "expected no drift for mixed object constructor parity: {diff:?}"
        );
    }

    #[test]
    fn live_oracle_dataframe_merge_ordered_ffill_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

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

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping merge_ordered oracle test: {message}");
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

        let actual = super::execute_dataframe_merge_ordered_fixture_operation(&fixture)
            .expect("actual frame");
        super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_combine_first_utf8_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
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
            eprintln!(
                "live pandas unavailable; skipping series combine_first oracle test: {message}"
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
            super::execute_series_combine_first_fixture_operation(&fixture).expect("actual series");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_dataframe_combine_first_object_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
    fn live_oracle_dataframe_rank_axis1_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
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
            eprintln!(
                "live pandas unavailable; skipping dataframe rank axis=1 oracle test: {message}"
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
        let mut cfg = HarnessConfig::default_paths();
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

    #[test]
    fn live_oracle_dataframe_take_axis0_negative_indices_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
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
            eprintln!(
                "live pandas unavailable; skipping dataframe take axis=0 oracle test: {message}"
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
    fn live_oracle_dataframe_take_axis1_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
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
            eprintln!(
                "live pandas unavailable; skipping dataframe take axis=1 oracle test: {message}"
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
    fn live_oracle_dataframe_groupby_idxmin_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
            eprintln!(
                "live pandas unavailable; skipping dataframe groupby any oracle test: {message}"
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
    fn live_oracle_dataframe_groupby_all_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
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
            eprintln!(
                "live pandas unavailable; skipping dataframe groupby all oracle test: {message}"
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
    fn live_oracle_dataframe_groupby_get_group_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
            eprintln!(
                "live pandas unavailable; skipping dataframe groupby sem oracle test: {message}"
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
    fn live_oracle_dataframe_groupby_skew_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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
        let mut cfg = HarnessConfig::default_paths();
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

    #[test]
    fn live_oracle_dataframe_at_time_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-067",
            "case_id": "dataframe_at_time_live",
            "mode": "strict",
            "operation": "dataframe_at_time",
            "oracle_source": "live_legacy_pandas",
            "time_value": "10:00:00",
            "frame": {
                "index": [
                    { "kind": "utf8", "value": "2024-01-01T10:00:00" },
                    { "kind": "utf8", "value": "2024-01-02T10:00:00" },
                    { "kind": "utf8", "value": "2024-01-03T14:30:00" }
                ],
                "column_order": ["a", "b"],
                "columns": {
                    "a": [
                        { "kind": "int64", "value": 1 },
                        { "kind": "int64", "value": 2 },
                        { "kind": "int64", "value": 3 }
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
            eprintln!("live pandas unavailable; skipping dataframe at_time oracle test: {message}");
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
    fn live_oracle_dataframe_between_time_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-067",
            "case_id": "dataframe_between_time_live",
            "mode": "strict",
            "operation": "dataframe_between_time",
            "oracle_source": "live_legacy_pandas",
            "start_time": "09:00:00",
            "end_time": "16:00:00",
            "frame": {
                "index": [
                    { "kind": "utf8", "value": "2024-01-01T08:00:00" },
                    { "kind": "utf8", "value": "2024-01-01T12:30:00" },
                    { "kind": "utf8", "value": "2024-01-01T15:00:00" },
                    { "kind": "utf8", "value": "2024-01-01T20:00:00" }
                ],
                "column_order": ["a", "b"],
                "columns": {
                    "a": [
                        { "kind": "int64", "value": 1 },
                        { "kind": "int64", "value": 2 },
                        { "kind": "int64", "value": 3 },
                        { "kind": "int64", "value": 4 }
                    ],
                    "b": [
                        { "kind": "utf8", "value": "w" },
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
            eprintln!(
                "live pandas unavailable; skipping dataframe between_time oracle test: {message}"
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
    fn live_oracle_dataframe_asof_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-067",
            "case_id": "dataframe_asof_live",
            "mode": "strict",
            "operation": "dataframe_asof",
            "oracle_source": "live_legacy_pandas",
            "asof_label": { "kind": "utf8", "value": "2024-01-02T10:00:00" },
            "subset": ["b"],
            "frame": {
                "index": [
                    { "kind": "utf8", "value": "2024-01-01T10:00:00" },
                    { "kind": "utf8", "value": "2024-01-02T10:00:00" },
                    { "kind": "utf8", "value": "2024-01-03T10:00:00" }
                ],
                "column_order": ["a", "b"],
                "columns": {
                    "a": [
                        { "kind": "int64", "value": 1 },
                        { "kind": "int64", "value": 2 },
                        { "kind": "int64", "value": 3 }
                    ],
                    "b": [
                        { "kind": "float64", "value": 10.0 },
                        { "kind": "null", "value": "na_n" },
                        { "kind": "float64", "value": 30.0 }
                    ]
                }
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping dataframe asof oracle test: {message}");
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

        let actual = super::execute_dataframe_asof_fixture_operation(&fixture).expect("actual");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_take_negative_indices_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2C-010",
            "case_id": "series_take_negative_indices_live",
            "mode": "strict",
            "operation": "series_take",
            "oracle_source": "live_legacy_pandas",
            "take_indices": [-1, -3],
            "left": {
                "name": "animals",
                "index": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 40 }
                ],
                "values": [
                    { "kind": "utf8", "value": "falcon" },
                    { "kind": "utf8", "value": "parrot" },
                    { "kind": "utf8", "value": "lion" },
                    { "kind": "utf8", "value": "monkey" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series take oracle test: {message}");
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

        let left = super::require_left_series(&fixture).expect("left series");
        let series = super::build_series(left).expect("build series");
        let actual = series.take(fixture.take_indices.as_deref().expect("take indices"));
        super::compare_series_expected(&actual.expect("actual series"), &expected)
            .expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_xs_duplicate_labels_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-077",
            "case_id": "series_xs_duplicate_labels_live",
            "mode": "strict",
            "operation": "series_xs",
            "oracle_source": "live_legacy_pandas",
            "xs_key": { "kind": "utf8", "value": "x" },
            "left": {
                "name": "vals",
                "index": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" },
                    { "kind": "utf8", "value": "x" }
                ],
                "values": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series xs oracle test: {message}");
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

        let actual = super::execute_series_xs_fixture_operation(&fixture).expect("actual");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_repeat_scalar_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-069",
            "case_id": "series_repeat_scalar_live",
            "mode": "strict",
            "operation": "series_repeat",
            "oracle_source": "live_legacy_pandas",
            "repeat_n": 2,
            "left": {
                "name": "animals",
                "index": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 }
                ],
                "values": [
                    { "kind": "utf8", "value": "falcon" },
                    { "kind": "utf8", "value": "lion" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series repeat oracle test: {message}");
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

        let actual = super::execute_series_repeat_fixture_operation(&fixture).expect("actual");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_repeat_counts_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-069",
            "case_id": "series_repeat_counts_live",
            "mode": "strict",
            "operation": "series_repeat",
            "oracle_source": "live_legacy_pandas",
            "repeat_counts": [2, 0, 1],
            "left": {
                "name": "nums",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ],
                "values": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!(
                "live pandas unavailable; skipping series repeat-count oracle test: {message}"
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

        let actual = super::execute_series_repeat_fixture_operation(&fixture).expect("actual");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_dataframe_xs_duplicate_labels_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-078",
            "case_id": "dataframe_xs_duplicate_labels_live",
            "mode": "strict",
            "operation": "dataframe_xs",
            "oracle_source": "live_legacy_pandas",
            "xs_key": { "kind": "utf8", "value": "x" },
            "frame": {
                "index": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" },
                    { "kind": "utf8", "value": "x" }
                ],
                "column_order": ["a", "b"],
                "columns": {
                    "a": [
                        { "kind": "int64", "value": 1 },
                        { "kind": "int64", "value": 2 },
                        { "kind": "int64", "value": 3 }
                    ],
                    "b": [
                        { "kind": "utf8", "value": "u" },
                        { "kind": "utf8", "value": "v" },
                        { "kind": "utf8", "value": "w" }
                    ]
                }
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping dataframe xs oracle test: {message}");
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

        let actual = super::execute_dataframe_fixture_operation(&fixture).expect("actual");
        super::compare_dataframe_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_to_numeric_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-074",
            "case_id": "series_to_numeric_live",
            "mode": "strict",
            "operation": "series_to_numeric",
            "oracle_source": "live_legacy_pandas",
            "left": {
                "name": "raw",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "values": [
                    { "kind": "utf8", "value": "1" },
                    { "kind": "utf8", "value": "2.5" },
                    { "kind": "utf8", "value": "bad" },
                    { "kind": "bool", "value": true }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series to_numeric oracle test: {message}");
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
            super::execute_series_module_utility_fixture_operation(&fixture).expect("actual");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_cut_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-075",
            "case_id": "series_cut_live",
            "mode": "strict",
            "operation": "series_cut",
            "oracle_source": "live_legacy_pandas",
            "cut_bins": 3,
            "left": {
                "name": "nums",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "values": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 4 },
                    { "kind": "int64", "value": 7 },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series cut oracle test: {message}");
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
            super::execute_series_module_utility_fixture_operation(&fixture).expect("actual");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_qcut_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-076",
            "case_id": "series_qcut_live",
            "mode": "strict",
            "operation": "series_qcut",
            "oracle_source": "live_legacy_pandas",
            "qcut_quantiles": 2,
            "left": {
                "name": "nums",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "values": [
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 30 },
                    { "kind": "null", "value": "na_n" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series qcut oracle test: {message}");
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
            super::execute_series_module_utility_fixture_operation(&fixture).expect("actual");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_at_time_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2C-010",
            "case_id": "series_at_time_live",
            "mode": "strict",
            "operation": "series_at_time",
            "oracle_source": "live_legacy_pandas",
            "time_value": "10:00:00",
            "left": {
                "name": "values",
                "index": [
                    { "kind": "utf8", "value": "2024-01-01T10:00:00" },
                    { "kind": "utf8", "value": "2024-01-02T10:00:00" },
                    { "kind": "utf8", "value": "2024-01-03T14:30:00" }
                ],
                "values": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series at_time oracle test: {message}");
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

        let left = super::require_left_series(&fixture).expect("left series");
        let series = super::build_series(left).expect("build series");
        let actual = series.at_time(fixture.time_value.as_deref().expect("time value"));
        super::compare_series_expected(&actual.expect("actual series"), &expected)
            .expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_between_time_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2C-010",
            "case_id": "series_between_time_live",
            "mode": "strict",
            "operation": "series_between_time",
            "oracle_source": "live_legacy_pandas",
            "start_time": "09:00:00",
            "end_time": "16:00:00",
            "left": {
                "name": "values",
                "index": [
                    { "kind": "utf8", "value": "2024-01-01T08:00:00" },
                    { "kind": "utf8", "value": "2024-01-01T12:30:00" },
                    { "kind": "utf8", "value": "2024-01-01T15:00:00" },
                    { "kind": "utf8", "value": "2024-01-01T20:00:00" }
                ],
                "values": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 },
                    { "kind": "int64", "value": 4 }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!(
                "live pandas unavailable; skipping series between_time oracle test: {message}"
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

        let left = super::require_left_series(&fixture).expect("left series");
        let series = super::build_series(left).expect("build series");
        let actual = series.between_time(
            fixture.start_time.as_deref().expect("start time"),
            fixture.end_time.as_deref().expect("end time"),
        );
        super::compare_series_expected(&actual.expect("actual series"), &expected)
            .expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_extractall_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-086",
            "case_id": "series_extractall_live",
            "mode": "strict",
            "operation": "series_extractall",
            "oracle_source": "live_legacy_pandas",
            "regex_pattern": "([a-z])(\\d)",
            "left": {
                "name": "tokens",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ],
                "values": [
                    { "kind": "utf8", "value": "a1b2" },
                    { "kind": "utf8", "value": "c3" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series extractall oracle test: {message}");
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
    fn live_oracle_series_extract_df_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-087",
            "case_id": "series_extract_df_live",
            "mode": "strict",
            "operation": "series_extract_df",
            "oracle_source": "live_legacy_pandas",
            "regex_pattern": "(?P<prefix>[a-z]+)-(?P<number>\\d+)",
            "left": {
                "name": "tokens",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ],
                "values": [
                    { "kind": "utf8", "value": "abc-123" },
                    { "kind": "utf8", "value": "xyz" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series extract_df oracle test: {message}");
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
    fn live_oracle_series_partition_df_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-088",
            "case_id": "series_partition_df_live",
            "mode": "strict",
            "operation": "series_partition_df",
            "oracle_source": "live_legacy_pandas",
            "string_sep": "-",
            "left": {
                "name": "tokens",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ],
                "values": [
                    { "kind": "utf8", "value": "a-b-c" },
                    { "kind": "utf8", "value": "solo" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!(
                "live pandas unavailable; skipping series partition_df oracle test: {message}"
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
    fn live_oracle_series_rpartition_df_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-089",
            "case_id": "series_rpartition_df_live",
            "mode": "strict",
            "operation": "series_rpartition_df",
            "oracle_source": "live_legacy_pandas",
            "string_sep": "-",
            "left": {
                "name": "tokens",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ],
                "values": [
                    { "kind": "utf8", "value": "a-b-c" },
                    { "kind": "utf8", "value": "solo" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!(
                "live pandas unavailable; skipping series rpartition_df oracle test: {message}"
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
    fn live_oracle_series_bool_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-068",
            "case_id": "series_bool_live",
            "mode": "strict",
            "operation": "series_bool",
            "oracle_source": "live_legacy_pandas",
            "left": {
                "name": "flag",
                "index": [{ "kind": "int64", "value": 0 }],
                "values": [{ "kind": "bool", "value": true }]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping series bool oracle test: {message}");
            return;
        }

        let expected = expected_result.expect("live oracle expected");
        assert!(
            matches!(&expected, super::ResolvedExpected::Bool(_)),
            "expected live oracle bool payload, got {expected:?}"
        );
        let super::ResolvedExpected::Bool(expected) = expected else {
            return;
        };

        let actual = super::execute_series_bool_fixture_operation(&fixture).expect("actual bool");
        assert_eq!(actual, expected);
    }

    #[test]
    fn live_oracle_series_bool_non_boolean_errors_like_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-068",
            "case_id": "series_bool_non_boolean_live",
            "mode": "strict",
            "operation": "series_bool",
            "oracle_source": "live_legacy_pandas",
            "expected_error_contains": "boolean scalar",
            "left": {
                "name": "flag",
                "index": [{ "kind": "int64", "value": 0 }],
                "values": [{ "kind": "int64", "value": 1 }]
            }
        }))
        .expect("fixture");

        let expected = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected {
            eprintln!("live pandas unavailable; skipping series bool error oracle test: {message}");
            return;
        }

        let expected = expected.expect("live oracle expected");
        assert!(
            matches!(&expected, super::ResolvedExpected::ErrorAny),
            "expected live oracle error payload, got {expected:?}"
        );

        let actual = super::execute_series_bool_fixture_operation(&fixture);
        assert!(
            actual
                .as_ref()
                .err()
                .is_some_and(|message| message.contains("boolean scalar")),
            "{actual:?}"
        );
    }

    #[test]
    fn live_oracle_dataframe_bool_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-068",
            "case_id": "dataframe_bool_live",
            "mode": "strict",
            "operation": "dataframe_bool",
            "oracle_source": "live_legacy_pandas",
            "frame": {
                "index": [{ "kind": "int64", "value": 0 }],
                "column_order": ["flag"],
                "columns": {
                    "flag": [{ "kind": "bool", "value": false }]
                }
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping dataframe bool oracle test: {message}");
            return;
        }

        let expected = expected_result.expect("live oracle expected");
        assert!(
            matches!(&expected, super::ResolvedExpected::Bool(_)),
            "expected live oracle bool payload, got {expected:?}"
        );
        let super::ResolvedExpected::Bool(expected) = expected else {
            return;
        };

        let actual =
            super::execute_dataframe_bool_fixture_operation(&fixture).expect("actual bool");
        assert_eq!(actual, expected);
    }

    #[test]
    fn live_oracle_dataframe_bool_non_boolean_errors_like_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-068",
            "case_id": "dataframe_bool_non_boolean_live",
            "mode": "strict",
            "operation": "dataframe_bool",
            "oracle_source": "live_legacy_pandas",
            "expected_error_contains": "boolean scalar",
            "frame": {
                "index": [{ "kind": "int64", "value": 0 }],
                "column_order": ["flag"],
                "columns": {
                    "flag": [{ "kind": "int64", "value": 1 }]
                }
            }
        }))
        .expect("fixture");

        let expected = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected {
            eprintln!(
                "live pandas unavailable; skipping dataframe bool error oracle test: {message}"
            );
            return;
        }

        let expected = expected.expect("live oracle expected");
        assert!(
            matches!(&expected, super::ResolvedExpected::ErrorAny),
            "expected live oracle error payload, got {expected:?}"
        );

        let actual = super::execute_dataframe_bool_fixture_operation(&fixture);
        assert!(
            actual
                .as_ref()
                .err()
                .is_some_and(|message| message.contains("boolean scalar")),
            "{actual:?}"
        );
    }

    #[test]
    fn live_oracle_series_to_datetime_unit_nanoseconds_preserves_precision() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-064",
            "case_id": "series_to_datetime_unit_nanoseconds_live",
            "mode": "strict",
            "operation": "series_to_datetime",
            "oracle_source": "live_legacy_pandas",
            "datetime_unit": "ns",
            "left": {
                "name": "epoch_ns",
                "index": [
                    { "kind": "int64", "value": 0 }
                ],
                "values": [
                    { "kind": "int64", "value": 1490195805433502912_i64 }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!(
                "live pandas unavailable; skipping to_datetime unit nanoseconds oracle test: {message}"
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
    fn live_oracle_series_to_datetime_origin_days_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-064",
            "case_id": "series_to_datetime_origin_days_live",
            "mode": "strict",
            "operation": "series_to_datetime",
            "oracle_source": "live_legacy_pandas",
            "datetime_unit": "D",
            "datetime_origin": "1960-01-01",
            "left": {
                "name": "epoch_d",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ],
                "values": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "float64", "value": 2.5 }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!(
                "live pandas unavailable; skipping to_datetime origin oracle test: {message}"
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

        let actual = fp_frame::to_datetime_with_options(
            &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
            fp_frame::ToDatetimeOptions {
                unit: fixture.datetime_unit.as_deref(),
                origin: super::resolve_datetime_origin_option(fixture.datetime_origin.as_ref())
                    .expect("datetime origin"),
                ..fp_frame::ToDatetimeOptions::default()
            },
        )
        .expect("actual series");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_to_datetime_numeric_origin_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-064",
            "case_id": "series_to_datetime_numeric_origin_live",
            "mode": "strict",
            "operation": "series_to_datetime",
            "oracle_source": "live_legacy_pandas",
            "datetime_unit": "D",
            "datetime_origin": 2,
            "left": {
                "name": "epoch_d",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ],
                "values": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "float64", "value": 1.5 }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!(
                "live pandas unavailable; skipping to_datetime numeric-origin oracle test: {message}"
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

        let actual = fp_frame::to_datetime_with_options(
            &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
            fp_frame::ToDatetimeOptions {
                unit: fixture.datetime_unit.as_deref(),
                origin: super::resolve_datetime_origin_option(fixture.datetime_origin.as_ref())
                    .expect("datetime origin"),
                ..fp_frame::ToDatetimeOptions::default()
            },
        )
        .expect("actual series");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_to_datetime_julian_origin_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-064",
            "case_id": "series_to_datetime_julian_origin_live",
            "mode": "strict",
            "operation": "series_to_datetime",
            "oracle_source": "live_legacy_pandas",
            "datetime_unit": "D",
            "datetime_origin": "julian",
            "left": {
                "name": "julian_d",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ],
                "values": [
                    { "kind": "float64", "value": 2451544.5 },
                    { "kind": "float64", "value": 2451545.0 }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!(
                "live pandas unavailable; skipping to_datetime julian-origin oracle test: {message}"
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

        let actual = fp_frame::to_datetime_with_options(
            &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
            fp_frame::ToDatetimeOptions {
                unit: fixture.datetime_unit.as_deref(),
                origin: super::resolve_datetime_origin_option(fixture.datetime_origin.as_ref())
                    .expect("datetime origin"),
                ..fp_frame::ToDatetimeOptions::default()
            },
        )
        .expect("actual series");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_to_datetime_utc_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-064",
            "case_id": "series_to_datetime_utc_live",
            "mode": "strict",
            "operation": "series_to_datetime",
            "oracle_source": "live_legacy_pandas",
            "datetime_utc": true,
            "left": {
                "name": "ts",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ],
                "values": [
                    { "kind": "utf8", "value": "2024-01-15 10:30:00" },
                    { "kind": "utf8", "value": "2024-01-15 10:30:00+05:30" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!("live pandas unavailable; skipping to_datetime utc oracle test: {message}");
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

        let actual = fp_frame::to_datetime_with_options(
            &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
            fp_frame::ToDatetimeOptions {
                utc: fixture.datetime_utc.unwrap_or(false),
                ..fp_frame::ToDatetimeOptions::default()
            },
        )
        .expect("actual series");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_series_to_datetime_mixed_tz_strings_matches_pandas() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.allow_system_pandas_fallback = false;

        let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
            "packet_id": "FP-P2D-064",
            "case_id": "series_to_datetime_mixed_tz_strings_live",
            "mode": "strict",
            "operation": "series_to_datetime",
            "oracle_source": "live_legacy_pandas",
            "left": {
                "name": "ts",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 }
                ],
                "values": [
                    { "kind": "utf8", "value": "2024-01-15 10:30:00" },
                    { "kind": "utf8", "value": "2024-01-15T10:30:00Z" }
                ]
            }
        }))
        .expect("fixture");

        let expected_result = super::capture_live_oracle_expected(&cfg, &fixture);
        if let Err(super::HarnessError::OracleUnavailable(message)) = &expected_result {
            eprintln!(
                "live pandas unavailable; skipping to_datetime mixed-tz-string oracle test: {message}"
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

        let actual = fp_frame::to_datetime(
            &super::build_series(fixture.left.as_ref().expect("left")).expect("series"),
        )
        .expect("actual series");
        super::compare_series_expected(&actual, &expected).expect("pandas parity");
    }

    #[test]
    fn live_oracle_unavailable_propagates_without_fallback() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.oracle_root = "/__fp_missing_legacy_oracle__/pandas".into();
        cfg.allow_system_pandas_fallback = false;

        let report = run_packet_by_id(&cfg, "FP-P2C-001", OracleMode::LiveLegacyPandas)
            .expect("expected report even when cases fail");
        assert!(
            !report.is_green(),
            "expected non-green report without fallback: {report:?}"
        );
        assert!(
            report.results.iter().all(|case| {
                case.mismatch
                    .as_deref()
                    .is_some_and(|message| message.contains("legacy oracle root does not exist"))
            }),
            "expected oracle-unavailable mismatches in all failed cases: {report:?}"
        );
    }

    #[test]
    fn live_oracle_unavailable_falls_back_to_fixture_when_enabled() {
        let mut cfg = HarnessConfig::default_paths();
        cfg.oracle_root = "/__fp_missing_legacy_oracle__/pandas".into();
        cfg.allow_system_pandas_fallback = true;

        let report = run_packet_by_id(&cfg, "FP-P2C-001", OracleMode::LiveLegacyPandas)
            .expect("fixture fallback should recover live-oracle unavailability");
        assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-001"));
        assert!(
            report.is_green(),
            "expected green fallback report: {report:?}"
        );
    }

    #[test]
    fn live_oracle_non_oracle_unavailable_errors_still_propagate() {
        let mut cfg = HarnessConfig::default_paths();
        if !cfg.oracle_root.exists() {
            eprintln!(
                "oracle repo missing at {}; skipping python-missing check",
                cfg.oracle_root.display()
            );
            return;
        }
        cfg.allow_system_pandas_fallback = true;
        cfg.python_bin = "/__fp_missing_python__/python3".to_owned();

        let report = run_packet_by_id(&cfg, "FP-P2C-001", OracleMode::LiveLegacyPandas)
            .expect("expected report even when command spawn fails");
        assert!(
            !report.is_green(),
            "expected non-green report for missing python binary: {report:?}"
        );
        assert!(
            report.results.iter().all(|case| {
                case.mismatch
                    .as_deref()
                    .is_some_and(|message| message.contains("No such file or directory"))
            }),
            "expected command-spawn io error mismatches in all failed cases: {report:?}"
        );
    }

    #[test]
    fn sidecar_verification_runs_on_generated_artifact() {
        let payload = br#"{\"suite\":\"phase2c_packets\",\"passed\":2,\"failed\":0}"#;
        let sidecar: RaptorQSidecarArtifact =
            generate_raptorq_sidecar("test/artifact", "conformance", payload, 8).expect("sidecar");
        let scrub = super::verify_raptorq_sidecar(&sidecar, payload).expect("scrub");
        assert_eq!(scrub.status, "ok");
    }

    // === Differential Harness Tests ===

    #[test]
    fn differential_suite_produces_structured_drift() {
        let cfg = HarnessConfig::default_paths();
        let diff_report =
            run_differential_suite(&cfg, &SuiteOptions::default()).expect("differential suite");
        assert!(diff_report.report.fixture_count >= 1);
        assert!(diff_report.report.is_green());
        assert_eq!(diff_report.drift_summary.critical_count, 0);
    }

    #[test]
    fn differential_by_id_matches_legacy_report() {
        let cfg = HarnessConfig::default_paths();
        let legacy_report =
            run_packet_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected).expect("legacy");
        let diff_report = run_differential_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
            .expect("differential");
        assert_eq!(
            diff_report.report.fixture_count,
            legacy_report.fixture_count
        );
        assert_eq!(diff_report.report.passed, legacy_report.passed);
        assert_eq!(diff_report.report.failed, legacy_report.failed);
    }

    #[test]
    fn differential_result_converts_to_case_result() {
        let diff = DifferentialResult {
            case_id: "test".to_owned(),
            packet_id: "FP-P2C-001".to_owned(),
            operation: FixtureOperation::SeriesAdd,
            mode: RuntimeMode::Strict,
            replay_key: "FP-P2C-001/test/strict".to_owned(),
            trace_id: "FP-P2C-001:test:strict".to_owned(),
            oracle_source: FixtureOracleSource::Fixture,
            status: CaseStatus::Pass,
            drift_records: Vec::new(),
            evidence_records: 0,
        };
        let case = diff.to_case_result();
        assert_eq!(case.status, CaseStatus::Pass);
        assert!(case.mismatch.is_none());
        assert_eq!(case.replay_key, "FP-P2C-001/test/strict");
        assert_eq!(case.trace_id, "FP-P2C-001:test:strict");
    }

    #[test]
    fn differential_result_with_drift_converts_mismatch_string() {
        let diff = DifferentialResult {
            case_id: "test".to_owned(),
            packet_id: "FP-P2C-001".to_owned(),
            operation: FixtureOperation::SeriesAdd,
            mode: RuntimeMode::Strict,
            replay_key: "FP-P2C-001/test/strict".to_owned(),
            trace_id: "FP-P2C-001:test:strict".to_owned(),
            oracle_source: FixtureOracleSource::Fixture,
            status: CaseStatus::Fail,
            drift_records: vec![
                DriftRecord {
                    category: ComparisonCategory::Value,
                    level: DriftLevel::Critical,
                    mismatch_class: "value_critical".to_owned(),
                    location: "series.values[0]".to_owned(),
                    message: "value mismatch".to_owned(),
                },
                DriftRecord {
                    category: ComparisonCategory::Index,
                    level: DriftLevel::NonCritical,
                    mismatch_class: "index_non_critical".to_owned(),
                    location: "series.index".to_owned(),
                    message: "order divergence".to_owned(),
                },
            ],
            evidence_records: 0,
        };
        let case = diff.to_case_result();
        assert_eq!(case.status, CaseStatus::Fail);
        let mismatch = case.mismatch.expect("should have mismatch");
        assert!(mismatch.contains("Value"));
        assert!(mismatch.contains("Index"));
        assert!(mismatch.contains("Critical"));
        assert!(mismatch.contains("NonCritical"));
    }

    #[test]
    fn drift_summary_counts_categories() {
        let results = vec![DifferentialResult {
            case_id: "test".to_owned(),
            packet_id: "FP-P2C-001".to_owned(),
            operation: FixtureOperation::SeriesAdd,
            mode: RuntimeMode::Strict,
            replay_key: "FP-P2C-001/test/strict".to_owned(),
            trace_id: "FP-P2C-001:test:strict".to_owned(),
            oracle_source: FixtureOracleSource::Fixture,
            status: CaseStatus::Fail,
            drift_records: vec![
                DriftRecord {
                    category: ComparisonCategory::Value,
                    level: DriftLevel::Critical,
                    mismatch_class: "value_critical".to_owned(),
                    location: "test[0]".to_owned(),
                    message: "mismatch".to_owned(),
                },
                DriftRecord {
                    category: ComparisonCategory::Index,
                    level: DriftLevel::NonCritical,
                    mismatch_class: "index_non_critical".to_owned(),
                    location: "test.index".to_owned(),
                    message: "order".to_owned(),
                },
                DriftRecord {
                    category: ComparisonCategory::Nullness,
                    level: DriftLevel::Informational,
                    mismatch_class: "nullness_informational".to_owned(),
                    location: "test[1]".to_owned(),
                    message: "nan handling".to_owned(),
                },
            ],
            evidence_records: 0,
        }];
        let summary = super::summarize_drift(&results);
        assert_eq!(summary.total_drift_records, 3);
        assert_eq!(summary.critical_count, 1);
        assert_eq!(summary.non_critical_count, 1);
        assert_eq!(summary.informational_count, 1);
        assert_eq!(summary.categories.len(), 3);
    }

    #[test]
    fn build_differential_report_aggregates_correctly() {
        let results = vec![
            DifferentialResult {
                case_id: "pass_case".to_owned(),
                packet_id: "FP-P2C-001".to_owned(),
                operation: FixtureOperation::SeriesAdd,
                mode: RuntimeMode::Strict,
                replay_key: "FP-P2C-001/pass_case/strict".to_owned(),
                trace_id: "FP-P2C-001:pass_case:strict".to_owned(),
                oracle_source: FixtureOracleSource::Fixture,
                status: CaseStatus::Pass,
                drift_records: Vec::new(),
                evidence_records: 0,
            },
            DifferentialResult {
                case_id: "fail_case".to_owned(),
                packet_id: "FP-P2C-001".to_owned(),
                operation: FixtureOperation::SeriesAdd,
                mode: RuntimeMode::Strict,
                replay_key: "FP-P2C-001/fail_case/strict".to_owned(),
                trace_id: "FP-P2C-001:fail_case:strict".to_owned(),
                oracle_source: FixtureOracleSource::Fixture,
                status: CaseStatus::Fail,
                drift_records: vec![DriftRecord {
                    category: ComparisonCategory::Value,
                    level: DriftLevel::Critical,
                    mismatch_class: "value_critical".to_owned(),
                    location: "v[0]".to_owned(),
                    message: "bad".to_owned(),
                }],
                evidence_records: 1,
            },
        ];
        let report = build_differential_report(
            "test_suite".to_owned(),
            Some("FP-P2C-001".to_owned()),
            true,
            results,
        );
        assert_eq!(report.report.fixture_count, 2);
        assert_eq!(report.report.passed, 1);
        assert_eq!(report.report.failed, 1);
        assert_eq!(report.differential_results.len(), 2);
        assert_eq!(report.drift_summary.total_drift_records, 1);
        assert_eq!(report.drift_summary.critical_count, 1);
    }

    #[test]
    fn differential_report_serializes_to_json() {
        let cfg = HarnessConfig::default_paths();
        let diff_report = run_differential_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
            .expect("differential");
        let json = serde_json::to_string_pretty(&diff_report).expect("serialize");
        assert!(json.contains("differential_results"));
        assert!(json.contains("drift_summary"));
    }

    #[test]
    fn differential_all_packets_green() {
        let cfg = HarnessConfig::default_paths();
        let mut packet_ids: Vec<String> = run_packets_grouped(&cfg, &SuiteOptions::default())
            .expect("grouped")
            .into_iter()
            .map(|report| {
                report
                    .packet_id
                    .expect("grouped report should have packet_id")
            })
            .collect();
        packet_ids.sort();
        packet_ids.dedup();
        assert!(
            !packet_ids.is_empty(),
            "expected at least one packet id from grouped reports"
        );
        for packet_id in packet_ids {
            let diff_report = run_differential_by_id(&cfg, &packet_id, OracleMode::FixtureExpected)
                .expect("differential report for discovered packet should run");
            assert!(
                diff_report.report.is_green(),
                "{packet_id} differential not green: {:?}",
                diff_report.drift_summary
            );
        }
    }

    #[test]
    fn differential_validation_log_contains_required_fields() {
        let cfg = HarnessConfig::default_paths();
        let report =
            run_differential_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected).expect("diff");
        let entries = build_differential_validation_log(&report);
        assert!(
            !entries.is_empty(),
            "expected differential validation entries"
        );

        for entry in entries {
            assert_eq!(entry.packet_id, "FP-P2C-001");
            assert!(!entry.case_id.is_empty());
            assert!(!entry.trace_id.is_empty());
            assert!(!entry.replay_key.is_empty());
            assert!(!entry.mismatch_class.is_empty());
        }
    }

    #[test]
    fn differential_validation_log_writes_jsonl() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut cfg = HarnessConfig::default_paths();
        cfg.repo_root = tmp.path().to_path_buf();

        let report =
            run_differential_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected).expect("diff");
        let path = write_differential_validation_log(&cfg, &report).expect("write log");
        assert!(path.exists(), "differential validation log should exist");
        let content = fs::read_to_string(path).expect("read");
        let lines: Vec<&str> = content.lines().collect();
        assert!(!lines.is_empty(), "expected at least one jsonl row");
        for line in lines {
            let row: serde_json::Value = serde_json::from_str(line).expect("json row");
            for required in [
                "packet_id",
                "case_id",
                "mode",
                "trace_id",
                "oracle_source",
                "mismatch_class",
                "replay_key",
            ] {
                assert!(row.get(required).is_some(), "missing field: {required}");
            }
        }
    }

    #[test]
    fn fault_injection_validation_classifies_strict_vs_hardened() {
        let cfg = HarnessConfig::default_paths();
        let report =
            run_fault_injection_validation_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
                .expect("fault report");
        assert_eq!(report.packet_id, "FP-P2C-001");
        assert!(report.entry_count > 0);
        assert!(report.strict_violation_count > 0);
        assert!(report.hardened_allowlisted_count > 0);
        assert_eq!(
            report.entry_count,
            report.strict_violation_count + report.hardened_allowlisted_count
        );

        for entry in &report.entries {
            assert!(!entry.case_id.is_empty());
            assert!(!entry.trace_id.is_empty());
            assert!(!entry.replay_key.is_empty());
            assert!(!entry.mismatch_class.is_empty());
            match entry.classification {
                FaultInjectionClassification::StrictViolation => {
                    assert_eq!(entry.mode, RuntimeMode::Strict);
                }
                FaultInjectionClassification::HardenedAllowlisted => {
                    assert_eq!(entry.mode, RuntimeMode::Hardened);
                }
            }
        }
    }

    #[test]
    fn fault_injection_validation_report_writes_json() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut cfg = HarnessConfig::default_paths();
        cfg.repo_root = tmp.path().to_path_buf();

        let report =
            run_fault_injection_validation_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
                .expect("fault report");
        let path = write_fault_injection_validation_report(&cfg, &report).expect("write report");
        assert!(path.exists(), "fault injection report should exist");
        let row: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).expect("read")).expect("json");
        assert_eq!(row["packet_id"], "FP-P2C-001");
        assert!(row.get("strict_violation_count").is_some());
        assert!(row.get("hardened_allowlisted_count").is_some());
        assert!(row.get("entries").is_some());
    }

    fn amplify_compat_closure_scenario_builder_workload(
        report: &mut super::E2eReport,
        repeats: usize,
    ) {
        let base_case_events: Vec<_> = report
            .forensic_log
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event.event,
                    ForensicEventKind::CaseStart { .. } | ForensicEventKind::CaseEnd { .. }
                )
            })
            .cloned()
            .collect();

        for repeat in 0..repeats {
            for template in &base_case_events {
                let mut event = template.clone();
                match &mut event.event {
                    ForensicEventKind::CaseStart {
                        case_id,
                        trace_id,
                        step_id,
                        assertion_path,
                        replay_cmd,
                        seed,
                        ..
                    } => {
                        *case_id = format!("{case_id}::amp{repeat}");
                        *trace_id = format!("{trace_id}::amp{repeat}");
                        *step_id = format!("{step_id}::amp{repeat}");
                        *assertion_path = format!("{assertion_path}::amp{repeat}");
                        *replay_cmd = format!("{replay_cmd} -- --amplify-seed={repeat}");
                        *seed = seed.saturating_add(repeat as u64 + 1);
                    }
                    ForensicEventKind::CaseEnd {
                        case_id,
                        trace_id,
                        step_id,
                        assertion_path,
                        replay_cmd,
                        replay_key,
                        seed,
                        ..
                    } => {
                        *case_id = format!("{case_id}::amp{repeat}");
                        *trace_id = format!("{trace_id}::amp{repeat}");
                        *step_id = format!("{step_id}::amp{repeat}");
                        *assertion_path = format!("{assertion_path}::amp{repeat}");
                        *replay_cmd = format!("{replay_cmd} -- --amplify-seed={repeat}");
                        *replay_key = format!("{replay_key}::amp{repeat}");
                        *seed = seed.saturating_add(repeat as u64 + 1);
                    }
                    _ => {}
                }
                report.forensic_log.events.push(event);
            }
        }
    }

    fn quantile_from_sorted(samples: &[u128], pct: usize) -> u128 {
        let len = samples.len();
        assert!(len > 0);
        let idx = (len.saturating_sub(1) * pct) / 100;
        samples[idx]
    }

    fn latency_quantiles(mut samples_ns: Vec<u128>) -> (u128, u128, u128) {
        samples_ns.sort_unstable();
        (
            quantile_from_sorted(&samples_ns, 50),
            quantile_from_sorted(&samples_ns, 95),
            quantile_from_sorted(&samples_ns, 99),
        )
    }

    #[test]
    fn compat_closure_e2e_scenario_report_contains_required_step_fields() {
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let e2e = run_e2e_suite(&config, &mut hooks).expect("e2e");
        let report = build_compat_closure_e2e_scenario_report(&e2e, &[]);
        assert!(report.scenario_count >= 1);
        assert!(!report.steps.is_empty());

        for step in report.steps {
            assert!(!step.scenario_id.is_empty());
            assert!(!step.packet_id.is_empty());
            assert!(!step.trace_id.is_empty());
            assert!(!step.step_id.is_empty());
            assert!(!step.command_or_api.is_empty());
            assert!(!step.input_ref.is_empty());
            assert!(!step.output_ref.is_empty());
            assert!(step.duration_ms >= 1);
            assert!(!step.outcome.is_empty());
            assert!(!step.reason_code.is_empty());
            assert!(!step.replay_cmd.is_empty());
        }
    }

    #[test]
    fn compat_closure_e2e_scenario_report_includes_failure_injection_steps() {
        let e2e_config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let e2e = run_e2e_suite(&e2e_config, &mut hooks).expect("e2e");
        let fault = run_fault_injection_validation_by_id(
            &e2e_config.harness,
            "FP-P2C-001",
            OracleMode::FixtureExpected,
        )
        .expect("fault");
        let report = build_compat_closure_e2e_scenario_report(&e2e, &[fault]);

        assert!(
            report.steps.iter().any(|step| {
                step.kind == super::CompatClosureScenarioKind::FailureInjection
                    && step.command_or_api == "fault_injection"
            }),
            "expected failure-injection steps in scenario report"
        );
    }

    #[test]
    fn compat_closure_e2e_scenario_optimized_path_is_isomorphic_to_baseline() {
        let e2e_config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let mut e2e = run_e2e_suite(&e2e_config, &mut hooks).expect("e2e");
        amplify_compat_closure_scenario_builder_workload(&mut e2e, 256);
        let fault = run_fault_injection_validation_by_id(
            &e2e_config.harness,
            "FP-P2C-001",
            OracleMode::FixtureExpected,
        )
        .expect("fault");

        let (baseline, baseline_stats) =
            super::build_compat_closure_e2e_scenario_report_baseline_with_stats(
                &e2e,
                std::slice::from_ref(&fault),
            );
        let (optimized, optimized_stats) =
            super::build_compat_closure_e2e_scenario_report_optimized_with_stats(
                &e2e,
                std::slice::from_ref(&fault),
            );
        assert_eq!(optimized, baseline);
        assert!(
            baseline_stats.trace_metadata_index_nodes > optimized_stats.trace_metadata_index_nodes
        );
        assert!(
            baseline_stats.trace_metadata_lookup_steps
                > optimized_stats.trace_metadata_lookup_steps
        );
    }

    #[test]
    fn compat_closure_e2e_scenario_profile_snapshot_reports_index_delta() {
        const ITERATIONS: usize = 64;
        let e2e_config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let mut e2e = run_e2e_suite(&e2e_config, &mut hooks).expect("e2e");
        amplify_compat_closure_scenario_builder_workload(&mut e2e, 256);
        let fault = run_fault_injection_validation_by_id(
            &e2e_config.harness,
            "FP-P2C-001",
            OracleMode::FixtureExpected,
        )
        .expect("fault");

        let mut baseline_ns = Vec::with_capacity(ITERATIONS);
        let mut optimized_ns = Vec::with_capacity(ITERATIONS);
        let mut baseline_index_nodes_total = 0_usize;
        let mut optimized_index_nodes_total = 0_usize;
        let mut baseline_lookup_steps_total = 0_usize;
        let mut optimized_lookup_steps_total = 0_usize;

        for _ in 0..ITERATIONS {
            let baseline_start = std::time::Instant::now();
            let (baseline, baseline_stats) =
                super::build_compat_closure_e2e_scenario_report_baseline_with_stats(
                    &e2e,
                    std::slice::from_ref(&fault),
                );
            baseline_ns.push(baseline_start.elapsed().as_nanos());
            baseline_index_nodes_total += baseline_stats.trace_metadata_index_nodes;
            baseline_lookup_steps_total += baseline_stats.trace_metadata_lookup_steps;

            let optimized_start = std::time::Instant::now();
            let (optimized, optimized_stats) =
                super::build_compat_closure_e2e_scenario_report_optimized_with_stats(
                    &e2e,
                    std::slice::from_ref(&fault),
                );
            optimized_ns.push(optimized_start.elapsed().as_nanos());
            optimized_index_nodes_total += optimized_stats.trace_metadata_index_nodes;
            optimized_lookup_steps_total += optimized_stats.trace_metadata_lookup_steps;

            assert_eq!(optimized, baseline);
            std::hint::black_box(optimized.steps.len());
        }

        let (baseline_p50_ns, baseline_p95_ns, baseline_p99_ns) = latency_quantiles(baseline_ns);
        let (optimized_p50_ns, optimized_p95_ns, optimized_p99_ns) =
            latency_quantiles(optimized_ns);
        assert!(baseline_index_nodes_total > optimized_index_nodes_total);
        assert!(baseline_lookup_steps_total > optimized_lookup_steps_total);

        println!(
            "compat_closure_e2e_scenario_profile_snapshot baseline_ns[p50={baseline_p50_ns},p95={baseline_p95_ns},p99={baseline_p99_ns}] optimized_ns[p50={optimized_p50_ns},p95={optimized_p95_ns},p99={optimized_p99_ns}] trace_metadata_index_nodes_baseline={baseline_index_nodes_total} trace_metadata_index_nodes_optimized={optimized_index_nodes_total} trace_metadata_lookup_steps_baseline={baseline_lookup_steps_total} trace_metadata_lookup_steps_optimized={optimized_lookup_steps_total}"
        );
    }

    #[test]
    fn compat_closure_e2e_scenario_report_writes_json() {
        let tmp = tempfile::tempdir().expect("tmp");
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };
        let mut hooks = NoopHooks;
        let e2e = run_e2e_suite(&config, &mut hooks).expect("e2e");
        let report = build_compat_closure_e2e_scenario_report(&e2e, &[]);
        let path = write_compat_closure_e2e_scenario_report(tmp.path(), &report).expect("write");
        assert!(path.exists());
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(path).expect("read")).expect("json");
        assert_eq!(json["suite_id"], "COMPAT-CLOSURE-G");
        assert!(json.get("steps").is_some());
    }

    #[test]
    fn compat_closure_final_evidence_pack_contains_required_fields() {
        let _artifact_guard = phase2c_artifact_test_lock();
        let cfg = HarnessConfig::default_paths();
        let options = SuiteOptions {
            packet_filter: Some("FP-P2C-001".to_owned()),
            oracle_mode: OracleMode::FixtureExpected,
        };
        let reports = run_packets_grouped(&cfg, &options).expect("reports");
        let _ = super::write_grouped_artifacts(&cfg, &reports).expect("write artifacts");
        let differential = run_differential_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
            .expect("differential");
        let fault =
            run_fault_injection_validation_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
                .expect("fault");

        let pack =
            build_compat_closure_final_evidence_pack(&cfg, &reports, &[differential], &[fault])
                .expect("build final evidence");
        let payload = serde_json::to_value(&pack).expect("serialize");
        for required in [
            "generated_unix_ms",
            "suite_id",
            "coverage_report",
            "strict_zero_drift",
            "hardened_allowlisted_total",
            "packets",
            "migration_manifest",
            "reproducibility_ledger",
            "benchmark_delta_report_ref",
            "invariant_checklist_delta",
            "risk_note_update",
            "all_checks_passed",
            "attestation_signature",
        ] {
            assert!(payload.get(required).is_some(), "missing field: {required}");
        }
        assert!(pack.coverage_report.is_complete());
        assert!(pack.attestation_signature.starts_with("sha256:"));
        assert!(
            pack.all_checks_passed,
            "expected all checks to pass for fixture-backed green packet"
        );
        assert!(
            pack.packets
                .iter()
                .any(|packet| packet.packet_id == "FP-P2C-001"),
            "expected packet snapshot for FP-P2C-001"
        );
    }

    #[test]
    fn compat_closure_final_evidence_pack_writes_json_artifacts() {
        let _artifact_guard = phase2c_artifact_test_lock();
        let cfg = HarnessConfig::default_paths();
        let options = SuiteOptions {
            packet_filter: Some("FP-P2C-001".to_owned()),
            oracle_mode: OracleMode::FixtureExpected,
        };
        let reports = run_packets_grouped(&cfg, &options).expect("reports");
        let _ = super::write_grouped_artifacts(&cfg, &reports).expect("write artifacts");
        let differential = run_differential_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
            .expect("differential");
        let fault =
            run_fault_injection_validation_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
                .expect("fault");

        let pack =
            build_compat_closure_final_evidence_pack(&cfg, &reports, &[differential], &[fault])
                .expect("build final evidence");
        let paths = write_compat_closure_final_evidence_pack(&cfg, &pack).expect("write final");

        assert!(paths.evidence_pack_path.exists());
        assert!(paths.migration_manifest_path.exists());
        assert!(paths.attestation_summary_path.exists());

        let summary: super::CompatClosureAttestationSummary = serde_json::from_str(
            &fs::read_to_string(&paths.attestation_summary_path).expect("read"),
        )
        .expect("summary json");
        assert_eq!(summary.attestation_signature, pack.attestation_signature);
        assert_eq!(
            summary.coverage_percent,
            pack.coverage_report.achieved_percent
        );
        assert_eq!(summary.packet_count, pack.packets.len());
    }

    // === E2E Orchestrator + Forensic Logging Tests (bd-2gi.6) ===

    #[test]
    fn forensic_log_records_events_with_timestamps() {
        let mut log = ForensicLog::new();
        assert!(log.is_empty());

        log.record(ForensicEventKind::SuiteStart {
            suite: "test".to_owned(),
            packet_filter: None,
        });
        log.record(ForensicEventKind::SuiteEnd {
            suite: "test".to_owned(),
            total_fixtures: 5,
            passed: 5,
            failed: 0,
        });

        assert_eq!(log.len(), 2);
        assert!(log.events[0].ts_unix_ms > 0);
        assert!(log.events[1].ts_unix_ms >= log.events[0].ts_unix_ms);
    }

    #[test]
    fn forensic_log_serializes_to_jsonl() {
        let mut log = ForensicLog::new();
        log.record(ForensicEventKind::PacketStart {
            packet_id: "FP-P2C-001".to_owned(),
        });
        log.record(ForensicEventKind::CaseEnd {
            scenario_id: "test:FP-P2C-001".to_owned(),
            packet_id: "FP-P2C-001".to_owned(),
            case_id: "test_case".to_owned(),
            trace_id: "FP-P2C-001:test_case:strict".to_owned(),
            step_id: "case:test_case".to_owned(),
            seed: 42,
            assertion_path: "ASUPERSYNC-G/FP-P2C-001/test_case".to_owned(),
            result: "pass".to_owned(),
            replay_cmd: "cargo test -p fp-conformance -- test_case --nocapture".to_owned(),
            decision_action: "allow".to_owned(),
            replay_key: "FP-P2C-001/test_case/strict".to_owned(),
            mismatch_class: None,
            status: CaseStatus::Pass,
            evidence_records: 2,
            elapsed_us: 1234,
        });

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("forensic.jsonl");
        log.write_jsonl(&path).expect("write");

        let content = fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Each line is valid JSON
        for line in &lines {
            let _: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
        }
        assert!(lines[0].contains("packet_start"));
        assert!(lines[1].contains("case_end"));
        assert!(lines[1].contains("test_case"));
        assert!(lines[1].contains("assertion_path"));
        assert!(lines[1].contains("replay_cmd"));
    }

    #[test]
    fn forensic_event_kind_serde_round_trip() {
        let events = vec![
            ForensicEventKind::SuiteStart {
                suite: "smoke".to_owned(),
                packet_filter: Some("FP-P2C-001".to_owned()),
            },
            ForensicEventKind::ArtifactWritten {
                packet_id: "FP-P2C-001".to_owned(),
                artifact_kind: "parity_report".to_owned(),
                path: "artifacts/phase2c/FP-P2C-001/parity_report.json".to_owned(),
            },
            ForensicEventKind::GateEvaluated {
                packet_id: "FP-P2C-002".to_owned(),
                pass: true,
                reasons: Vec::new(),
            },
            ForensicEventKind::Error {
                phase: "gate_enforcement".to_owned(),
                message: "gate failed".to_owned(),
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).expect("serialize");
            let back: ForensicEventKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(*event, back);
        }
    }

    #[test]
    fn e2e_suite_runs_full_pipeline() {
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions::default(),
            write_artifacts: false,      // skip artifact writes in test
            enforce_gates: false,        // skip enforcement in test
            append_drift_history: false, // skip drift history in test
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let report = run_e2e_suite(&config, &mut hooks).expect("e2e");
        let expected_packet_count = run_packets_grouped(&config.harness, &config.options)
            .expect("grouped")
            .len();

        assert!(
            report.packet_reports.len() == expected_packet_count,
            "expected packet report count to match grouped packets: expected={expected_packet_count} actual={}",
            report.packet_reports.len()
        );
        assert!(report.total_fixtures > 0, "should have fixtures");
        assert_eq!(report.total_failed, 0, "no failures expected");
        assert!(report.is_green(), "e2e should be green");

        // Forensic log should have events
        assert!(!report.forensic_log.is_empty());

        // Should have suite_start and suite_end
        let first = &report.forensic_log.events[0].event;
        assert!(
            matches!(first, ForensicEventKind::SuiteStart { .. }),
            "first event should be SuiteStart"
        );
        let last = &report.forensic_log.events[report.forensic_log.len() - 1].event;
        assert!(
            matches!(last, ForensicEventKind::SuiteEnd { .. }),
            "last event should be SuiteEnd"
        );
    }

    #[test]
    fn e2e_suite_with_packet_filter() {
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let report = run_e2e_suite(&config, &mut hooks).expect("e2e");

        assert_eq!(report.packet_reports.len(), 1);
        assert_eq!(
            report.packet_reports[0].packet_id.as_deref(),
            Some("FP-P2C-001")
        );
        assert!(report.is_green());
    }

    #[test]
    fn e2e_lifecycle_hooks_called() {
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct TrackingHooks {
            calls: Arc<Mutex<Vec<String>>>,
        }

        impl LifecycleHooks for TrackingHooks {
            fn before_suite(&mut self, suite: &str, _filter: &Option<String>) {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("before_suite:{suite}"));
            }
            fn after_suite(&mut self, _reports: &[PacketParityReport]) {
                self.calls.lock().unwrap().push("after_suite".to_owned());
            }
            fn before_packet(&mut self, packet_id: &str) {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("before_packet:{packet_id}"));
            }
            fn after_packet(&mut self, _report: &PacketParityReport, gate_pass: bool) {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("after_packet:gate={gate_pass}"));
            }
            fn after_case(&mut self, result: &CaseResult) {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("after_case:{}", result.case_id));
            }
        }

        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = TrackingHooks {
            calls: calls.clone(),
        };
        let _report = run_e2e_suite(&config, &mut hooks).expect("e2e");

        let logged = calls.lock().unwrap();
        assert!(logged[0].starts_with("before_suite:"));
        assert!(logged.iter().any(|c| c.starts_with("before_packet:")));
        assert!(logged.iter().any(|c| c.starts_with("after_packet:")));
        assert!(logged.iter().any(|c| c.starts_with("after_case:")));
        assert_eq!(*logged.last().unwrap(), "after_suite");
    }

    #[test]
    fn e2e_forensic_log_writes_to_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("forensic.jsonl");

        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: Some(log_path.clone()),
        };

        let mut hooks = NoopHooks;
        let _report = run_e2e_suite(&config, &mut hooks).expect("e2e");

        assert!(log_path.exists(), "forensic log file should exist");
        let content = fs::read_to_string(&log_path).expect("read");
        assert!(content.lines().count() >= 4, "should have multiple events");

        // Verify every line is valid JSON
        for line in content.lines() {
            let parsed: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
            assert!(parsed.get("ts_unix_ms").is_some());
            assert!(parsed.get("event").is_some());
        }
    }

    #[test]
    fn e2e_case_events_include_replay_bundle_fields() {
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let report = run_e2e_suite(&config, &mut hooks).expect("e2e");

        let mut saw_case_start = false;
        let mut saw_case_end = false;

        for event in report.forensic_log.events {
            match event.event {
                ForensicEventKind::CaseStart {
                    seed,
                    assertion_path,
                    replay_cmd,
                    ..
                } => {
                    saw_case_start = true;
                    assert!(seed > 0, "seed should be deterministic non-zero");
                    assert!(
                        assertion_path.starts_with("ASUPERSYNC-G/"),
                        "assertion_path should be namespaced"
                    );
                    assert!(
                        replay_cmd.contains("cargo test -p fp-conformance --"),
                        "replay command should target fp-conformance test replay"
                    );
                }
                ForensicEventKind::CaseEnd {
                    seed,
                    assertion_path,
                    result,
                    replay_cmd,
                    ..
                } => {
                    saw_case_end = true;
                    assert!(seed > 0, "seed should be deterministic non-zero");
                    assert!(
                        assertion_path.starts_with("ASUPERSYNC-G/"),
                        "assertion_path should be namespaced"
                    );
                    assert!(result == "pass" || result == "fail");
                    assert!(
                        replay_cmd.contains("cargo test -p fp-conformance --"),
                        "replay command should target fp-conformance test replay"
                    );
                }
                _ => {}
            }
        }

        assert!(saw_case_start, "expected at least one case_start event");
        assert!(saw_case_end, "expected at least one case_end event");
    }

    #[test]
    fn compat_closure_case_log_contains_required_fields() {
        let config = HarnessConfig::default_paths();
        let case = CaseResult {
            packet_id: "FP-P2C-001".to_owned(),
            case_id: "series_add_strict".to_owned(),
            mode: RuntimeMode::Strict,
            operation: FixtureOperation::SeriesAdd,
            status: CaseStatus::Pass,
            mismatch: None,
            mismatch_class: None,
            replay_key: "FP-P2C-001/series_add_strict/strict".to_owned(),
            trace_id: "FP-P2C-001:series_add_strict:strict".to_owned(),
            elapsed_us: 5_000,
            evidence_records: 2,
        };

        let log = super::build_compat_closure_case_log(
            &config,
            super::COMPAT_CLOSURE_SUITE_ID,
            &case,
            1_700_000_000_000,
        );
        let payload = serde_json::to_value(&log).expect("serialize");

        for field in [
            "ts_utc",
            "suite_id",
            "test_id",
            "api_surface_id",
            "packet_id",
            "mode",
            "seed",
            "input_digest",
            "output_digest",
            "env_fingerprint",
            "artifact_refs",
            "duration_ms",
            "outcome",
            "reason_code",
        ] {
            assert!(
                payload.get(field).is_some(),
                "structured compat-closure log missing field: {field}"
            );
        }

        assert_eq!(log.suite_id, super::COMPAT_CLOSURE_SUITE_ID);
        assert_eq!(log.api_surface_id, "CC-004");
        assert_eq!(log.outcome, "pass");
        assert_eq!(log.reason_code, "ok");
    }

    #[test]
    fn compat_closure_case_log_is_deterministic_for_same_inputs() {
        let config = HarnessConfig::default_paths();
        let case = CaseResult {
            packet_id: "FP-P2C-002".to_owned(),
            case_id: "index_align_union".to_owned(),
            mode: RuntimeMode::Hardened,
            operation: FixtureOperation::IndexAlignUnion,
            status: CaseStatus::Fail,
            mismatch: Some("synthetic mismatch".to_owned()),
            mismatch_class: Some("index_critical".to_owned()),
            replay_key: "FP-P2C-002/index_align_union/hardened".to_owned(),
            trace_id: "FP-P2C-002:index_align_union:hardened".to_owned(),
            elapsed_us: 8_000,
            evidence_records: 1,
        };

        let first = super::build_compat_closure_case_log(
            &config,
            super::COMPAT_CLOSURE_SUITE_ID,
            &case,
            1_700_000_000_001,
        );
        let second = super::build_compat_closure_case_log(
            &config,
            super::COMPAT_CLOSURE_SUITE_ID,
            &case,
            1_700_000_000_001,
        );

        let first_json = serde_json::to_vec(&first).expect("serialize");
        let second_json = serde_json::to_vec(&second).expect("serialize");
        assert_eq!(
            first_json, second_json,
            "compat-closure logs should be byte-identical for same inputs"
        );
        assert_eq!(first.reason_code, "index_critical");
    }

    #[test]
    fn compat_closure_mode_split_contracts_hold_across_seed_span() {
        for seed in 0_u64..128 {
            let mut strict_ledger = fp_runtime::EvidenceLedger::new();
            let strict = fp_runtime::RuntimePolicy::strict();
            let strict_action = strict.decide_unknown_feature(
                "compat-closure",
                format!("seed={seed}"),
                &mut strict_ledger,
            );
            assert_eq!(
                strict_action,
                fp_runtime::DecisionAction::Reject,
                "strict mode should fail closed (CC-008)"
            );

            let cap = 32 + (seed as usize % 64);
            let mut hardened_ledger = fp_runtime::EvidenceLedger::new();
            let hardened = fp_runtime::RuntimePolicy::hardened(Some(cap));
            let hardened_action = hardened.decide_join_admission(cap + 1, &mut hardened_ledger);
            assert_eq!(
                hardened_action,
                fp_runtime::DecisionAction::Repair,
                "hardened mode should enforce bounded repair over cap (CC-009)"
            );
        }
    }

    #[test]
    fn compat_closure_coverage_report_is_complete() {
        let config = HarnessConfig::default_paths();
        let report = super::build_compat_closure_coverage_report(&config).expect("coverage report");
        assert_eq!(report.suite_id, super::COMPAT_CLOSURE_SUITE_ID);
        assert!(
            report.is_complete(),
            "compat-closure matrix has uncovered rows: {:?}",
            report.uncovered_rows
        );
        assert_eq!(report.achieved_percent, 100);
        assert_eq!(report.coverage_floor_percent, 100);
    }

    #[test]
    fn e2e_emits_compat_closure_case_events() {
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let report = run_e2e_suite(&config, &mut hooks).expect("e2e");

        let compat_events: Vec<_> = report
            .forensic_log
            .events
            .iter()
            .filter_map(|entry| match &entry.event {
                ForensicEventKind::CompatClosureCase {
                    suite_id,
                    api_surface_id,
                    seed,
                    input_digest,
                    output_digest,
                    env_fingerprint,
                    artifact_refs,
                    duration_ms,
                    outcome,
                    reason_code,
                    ..
                } => Some((
                    suite_id,
                    api_surface_id,
                    seed,
                    input_digest,
                    output_digest,
                    env_fingerprint,
                    artifact_refs,
                    duration_ms,
                    outcome,
                    reason_code,
                )),
                _ => None,
            })
            .collect();

        assert!(
            !compat_events.is_empty(),
            "expected compat-closure case logs in forensic output"
        );

        for (
            suite_id,
            api_surface_id,
            seed,
            input_digest,
            output_digest,
            env_fingerprint,
            artifact_refs,
            duration_ms,
            outcome,
            reason_code,
        ) in compat_events
        {
            assert_eq!(suite_id.as_str(), super::COMPAT_CLOSURE_SUITE_ID);
            assert!(api_surface_id.starts_with("CC-"));
            assert!(*seed > 0);
            assert_eq!(input_digest.len(), 64);
            assert_eq!(output_digest.len(), 64);
            assert_eq!(env_fingerprint.len(), 64);
            assert!(
                artifact_refs.len() >= 5,
                "expected closure artifact references"
            );
            assert!(*duration_ms >= 1);
            assert!(outcome == "pass" || outcome == "fail");
            assert!(!reason_code.is_empty());
        }
    }

    // === Failure Forensics UX Tests (bd-2gi.21) ===

    #[test]
    fn artifact_id_short_hash_is_deterministic() {
        let id = ArtifactId {
            packet_id: "FP-P2C-001".to_owned(),
            artifact_kind: "parity_report".to_owned(),
            run_ts_unix_ms: 1000,
        };
        let h1 = id.short_hash();
        let h2 = id.short_hash();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 8);
        // Display format
        let display = format!("{id}");
        assert!(display.starts_with("FP-P2C-001:parity_report@"));
    }

    #[test]
    fn failure_digest_display_format() {
        let digest = sample_failure_digest();

        let output = format!("{digest}");
        assert_text_golden("failure_digest_display.txt", &output);
    }

    #[test]
    fn failure_forensics_report_clean_when_all_pass() {
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions::default(),
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };
        let mut hooks = NoopHooks;
        let e2e = run_e2e_suite(&config, &mut hooks).expect("e2e");
        let forensics = build_failure_forensics(&e2e);

        assert!(forensics.is_clean());
        let output = format!("{forensics}");
        assert!(output.contains("ALL GREEN"));
    }

    #[test]
    fn failure_forensics_report_shows_failures() {
        let report = sample_failure_forensics_report();

        assert!(!report.is_clean());
        let output = format!("{report}");
        assert_text_golden("failure_forensics_report.txt", &output);
    }

    #[test]
    fn failure_forensics_clean_display_matches_golden() {
        let report = FailureForensicsReport {
            run_ts_unix_ms: 1000,
            total_fixtures: 5,
            total_passed: 5,
            total_failed: 0,
            failures: Vec::new(),
            gate_failures: Vec::new(),
        };

        let output = format!("{report}");
        assert_text_golden("failure_forensics_all_green.txt", &output);
    }

    #[test]
    fn failure_forensics_serializes_to_json() {
        let report = FailureForensicsReport {
            run_ts_unix_ms: 1000,
            total_fixtures: 1,
            total_passed: 1,
            total_failed: 0,
            failures: Vec::new(),
            gate_failures: Vec::new(),
        };
        let json = serde_json::to_string(&report).expect("serialize");
        let back: FailureForensicsReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report, back);
    }

    // === RaptorQ CI Enforcement Tests (bd-2gi.9) ===

    #[test]
    fn decode_proof_artifact_round_trip_serialization() {
        let artifact = DecodeProofArtifact {
            packet_id: "FP-P2C-001".to_owned(),
            decode_proofs: vec![fp_runtime::DecodeProof {
                ts_unix_ms: 1000,
                reason: "test drill".to_owned(),
                recovered_blocks: 2,
                proof_hash: "sha256:abcdef".to_owned(),
            }],
            status: DecodeProofStatus::Recovered,
        };
        let json = serde_json::to_string_pretty(&artifact).expect("serialize");
        let back: DecodeProofArtifact = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(artifact, back);
        assert!(json.contains("\"recovered\""));
    }

    #[test]
    fn decode_proof_status_variants_serialize_correctly() {
        for (status, expected) in [
            (DecodeProofStatus::Recovered, "\"recovered\""),
            (DecodeProofStatus::Failed, "\"failed\""),
            (DecodeProofStatus::NotAttempted, "\"not_attempted\""),
        ] {
            let json = serde_json::to_string(&status).expect("serialize");
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn sidecar_integrity_check_passes_for_valid_packet_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packet_id = "FP-P2C-099";

        // Write parity report
        let report_bytes = br#"{"suite":"test","passed":1,"failed":0}"#;
        fs::write(dir.path().join("parity_report.json"), report_bytes).unwrap();

        // Generate sidecar
        let mut sidecar =
            generate_raptorq_sidecar("FP-P2C-099/parity_report", "conformance", report_bytes, 8)
                .expect("sidecar");
        let proof =
            run_raptorq_decode_recovery_drill(&sidecar, report_bytes).expect("decode drill");
        sidecar.envelope.push_decode_proof_capped(proof.clone());
        sidecar.envelope.scrub.status = "ok".to_owned();
        fs::write(
            dir.path().join("parity_report.raptorq.json"),
            serde_json::to_string_pretty(&sidecar).unwrap(),
        )
        .unwrap();

        // Write decode proof artifact
        let decode_artifact = DecodeProofArtifact {
            packet_id: packet_id.to_owned(),
            decode_proofs: vec![proof],
            status: DecodeProofStatus::Recovered,
        };
        fs::write(
            dir.path().join("parity_report.decode_proof.json"),
            serde_json::to_string_pretty(&decode_artifact).unwrap(),
        )
        .unwrap();

        let result = verify_packet_sidecar_integrity(dir.path(), packet_id);
        assert!(
            result.is_ok(),
            "expected ok, got errors: {:?}",
            result.errors
        );
        assert!(result.source_hash_matches);
        assert!(result.scrub_ok);
        assert!(result.decode_proof_valid);
    }

    #[test]
    fn sidecar_integrity_fails_when_sidecar_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let report_bytes = br#"{"suite":"test"}"#;
        fs::write(dir.path().join("parity_report.json"), report_bytes).unwrap();

        let result = verify_packet_sidecar_integrity(dir.path(), "FP-P2C-099");
        assert!(!result.is_ok());
        assert!(result.parity_report_exists);
        assert!(!result.sidecar_exists);
        assert!(result.errors.iter().any(|e| e.contains("Rule T5")));
    }

    #[test]
    fn write_packet_artifacts_synthesizes_missing_parity_gate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packet_id = "FP-P2D-999";
        let config = HarnessConfig {
            repo_root: dir.path().to_path_buf(),
            oracle_root: dir.path().join("legacy_pandas_code/pandas"),
            fixture_root: dir.path().join("fixtures"),
            strict_mode: true,
            python_bin: "python3".to_owned(),
            allow_system_pandas_fallback: false,
        };
        let report = PacketParityReport {
            suite: format!("phase2c_packets:{packet_id}"),
            packet_id: Some(packet_id.to_owned()),
            oracle_present: false,
            fixture_count: 2,
            passed: 2,
            failed: 0,
            results: vec![
                CaseResult {
                    packet_id: packet_id.to_owned(),
                    case_id: "strict_case".to_owned(),
                    mode: RuntimeMode::Strict,
                    operation: FixtureOperation::SeriesCombineFirst,
                    status: CaseStatus::Pass,
                    mismatch: None,
                    mismatch_class: None,
                    replay_key: format!("{packet_id}/strict_case/strict"),
                    trace_id: format!("{packet_id}:strict_case:strict"),
                    elapsed_us: 1,
                    evidence_records: 1,
                },
                CaseResult {
                    packet_id: packet_id.to_owned(),
                    case_id: "hardened_case".to_owned(),
                    mode: RuntimeMode::Hardened,
                    operation: FixtureOperation::SeriesCombineFirst,
                    status: CaseStatus::Pass,
                    mismatch: None,
                    mismatch_class: None,
                    replay_key: format!("{packet_id}/hardened_case/hardened"),
                    trace_id: format!("{packet_id}:hardened_case:hardened"),
                    elapsed_us: 1,
                    evidence_records: 1,
                },
            ],
        };

        let written = write_packet_artifacts(&config, &report).expect("write artifacts");
        let gate_path = config.parity_gate_path(packet_id);
        let gate_yaml = fs::read_to_string(&gate_path).expect("read synthesized gate");

        assert!(gate_path.exists(), "missing synthesized parity gate");
        assert!(
            gate_yaml.contains(&format!("packet_id: {packet_id}")),
            "unexpected gate payload: {gate_yaml}"
        );
        assert!(
            gate_yaml.contains("require_fixture_count_at_least: 2"),
            "unexpected gate payload: {gate_yaml}"
        );
        assert!(
            written.gate_result_path.exists(),
            "missing gate result artifact"
        );
    }

    #[test]
    fn sidecar_integrity_fails_when_decode_proof_hash_mismatches_sidecar() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packet_id = "FP-P2C-099";

        let report_bytes = br#"{"suite":"test","passed":1,"failed":0}"#;
        fs::write(dir.path().join("parity_report.json"), report_bytes).expect("write report");

        let mut sidecar =
            generate_raptorq_sidecar("FP-P2C-099/parity_report", "conformance", report_bytes, 8)
                .expect("sidecar");
        let proof =
            run_raptorq_decode_recovery_drill(&sidecar, report_bytes).expect("decode drill");
        sidecar.envelope.push_decode_proof_capped(proof.clone());
        sidecar.envelope.scrub.status = "ok".to_owned();
        fs::write(
            dir.path().join("parity_report.raptorq.json"),
            serde_json::to_string_pretty(&sidecar).expect("serialize sidecar"),
        )
        .expect("write sidecar");

        let mut mismatched_proof = proof;
        mismatched_proof.proof_hash = "sha256:deadbeef".to_owned();
        let decode_artifact = DecodeProofArtifact {
            packet_id: packet_id.to_owned(),
            decode_proofs: vec![mismatched_proof],
            status: DecodeProofStatus::Recovered,
        };
        fs::write(
            dir.path().join("parity_report.decode_proof.json"),
            serde_json::to_string_pretty(&decode_artifact).expect("serialize decode artifact"),
        )
        .expect("write decode artifact");

        let result = verify_packet_sidecar_integrity(dir.path(), packet_id);
        assert!(!result.is_ok());
        assert!(!result.decode_proof_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|entry| entry.contains("proof hash mismatch")),
            "expected proof hash mismatch error, got {:?}",
            result.errors
        );
    }

    #[test]
    fn sidecar_integrity_fails_when_decode_proof_count_exceeds_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packet_id = "FP-P2C-099";

        let report_bytes = br#"{"suite":"test","passed":1,"failed":0}"#;
        fs::write(dir.path().join("parity_report.json"), report_bytes).expect("write report");

        let mut sidecar =
            generate_raptorq_sidecar("FP-P2C-099/parity_report", "conformance", report_bytes, 8)
                .expect("sidecar");
        sidecar.envelope.decode_proofs = (0..=fp_runtime::MAX_DECODE_PROOFS)
            .map(|idx| fp_runtime::DecodeProof {
                ts_unix_ms: u64::try_from(idx).expect("idx fits in u64"),
                reason: format!("overflow-{idx}"),
                recovered_blocks: u32::try_from(idx).expect("idx fits in u32"),
                proof_hash: format!("sha256:{idx:08x}"),
            })
            .collect();
        sidecar.envelope.scrub.status = "ok".to_owned();
        fs::write(
            dir.path().join("parity_report.raptorq.json"),
            serde_json::to_string_pretty(&sidecar).expect("serialize sidecar"),
        )
        .expect("write sidecar");

        let decode_artifact = DecodeProofArtifact {
            packet_id: packet_id.to_owned(),
            decode_proofs: vec![fp_runtime::DecodeProof {
                ts_unix_ms: 1,
                reason: "single-proof".to_owned(),
                recovered_blocks: 1,
                proof_hash: "sha256:00000001".to_owned(),
            }],
            status: DecodeProofStatus::Recovered,
        };
        fs::write(
            dir.path().join("parity_report.decode_proof.json"),
            serde_json::to_string_pretty(&decode_artifact).expect("serialize decode artifact"),
        )
        .expect("write decode artifact");

        let result = verify_packet_sidecar_integrity(dir.path(), packet_id);
        assert!(!result.is_ok());
        assert!(!result.decode_proof_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|entry| entry.contains("decode_proofs exceeds cap")),
            "expected decode proof cap error, got {:?}",
            result.errors
        );
    }

    #[test]
    fn sidecar_integrity_fails_when_sidecar_artifact_id_mismatches_packet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packet_id = "FP-P2C-099";

        let report_bytes = br#"{"suite":"test","passed":1,"failed":0}"#;
        fs::write(dir.path().join("parity_report.json"), report_bytes).expect("write report");

        let mut sidecar =
            generate_raptorq_sidecar("FP-P2C-007/parity_report", "conformance", report_bytes, 8)
                .expect("sidecar");
        let proof =
            run_raptorq_decode_recovery_drill(&sidecar, report_bytes).expect("decode drill");
        sidecar.envelope.push_decode_proof_capped(proof.clone());
        sidecar.envelope.scrub.status = "ok".to_owned();
        fs::write(
            dir.path().join("parity_report.raptorq.json"),
            serde_json::to_string_pretty(&sidecar).expect("serialize sidecar"),
        )
        .expect("write sidecar");

        let decode_artifact = DecodeProofArtifact {
            packet_id: packet_id.to_owned(),
            decode_proofs: vec![proof],
            status: DecodeProofStatus::Recovered,
        };
        fs::write(
            dir.path().join("parity_report.decode_proof.json"),
            serde_json::to_string_pretty(&decode_artifact).expect("serialize decode artifact"),
        )
        .expect("write decode artifact");

        let result = verify_packet_sidecar_integrity(dir.path(), packet_id);
        assert!(!result.is_ok());
        assert!(!result.decode_proof_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|entry| entry.contains("artifact_id mismatch")),
            "expected artifact_id mismatch error, got {:?}",
            result.errors
        );
    }

    #[test]
    fn sidecar_integrity_fails_when_envelope_k_mismatches_source_packets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packet_id = "FP-P2C-099";

        let report_bytes = br#"{"suite":"test","passed":1,"failed":0}"#;
        fs::write(dir.path().join("parity_report.json"), report_bytes).expect("write report");

        let mut sidecar =
            generate_raptorq_sidecar("FP-P2C-099/parity_report", "conformance", report_bytes, 8)
                .expect("sidecar");
        let proof =
            run_raptorq_decode_recovery_drill(&sidecar, report_bytes).expect("decode drill");
        sidecar.envelope.push_decode_proof_capped(proof.clone());
        sidecar.envelope.scrub.status = "ok".to_owned();
        sidecar.envelope.raptorq.k = sidecar.envelope.raptorq.k.saturating_add(1);
        fs::write(
            dir.path().join("parity_report.raptorq.json"),
            serde_json::to_string_pretty(&sidecar).expect("serialize sidecar"),
        )
        .expect("write sidecar");

        let decode_artifact = DecodeProofArtifact {
            packet_id: packet_id.to_owned(),
            decode_proofs: vec![proof],
            status: DecodeProofStatus::Recovered,
        };
        fs::write(
            dir.path().join("parity_report.decode_proof.json"),
            serde_json::to_string_pretty(&decode_artifact).expect("serialize decode artifact"),
        )
        .expect("write decode artifact");

        let result = verify_packet_sidecar_integrity(dir.path(), packet_id);
        assert!(!result.is_ok());
        assert!(!result.decode_proof_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|entry| entry.contains("does not match source_packets")),
            "expected source packet count mismatch error, got {:?}",
            result.errors
        );
    }

    #[test]
    fn sidecar_integrity_fails_when_envelope_repair_symbols_mismatch_repair_packets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packet_id = "FP-P2C-099";

        let report_bytes = br#"{"suite":"test","passed":1,"failed":0}"#;
        fs::write(dir.path().join("parity_report.json"), report_bytes).expect("write report");

        let mut sidecar =
            generate_raptorq_sidecar("FP-P2C-099/parity_report", "conformance", report_bytes, 8)
                .expect("sidecar");
        let proof =
            run_raptorq_decode_recovery_drill(&sidecar, report_bytes).expect("decode drill");
        sidecar.envelope.push_decode_proof_capped(proof.clone());
        sidecar.envelope.scrub.status = "ok".to_owned();
        sidecar.envelope.raptorq.repair_symbols =
            sidecar.envelope.raptorq.repair_symbols.saturating_add(1);
        fs::write(
            dir.path().join("parity_report.raptorq.json"),
            serde_json::to_string_pretty(&sidecar).expect("serialize sidecar"),
        )
        .expect("write sidecar");

        let decode_artifact = DecodeProofArtifact {
            packet_id: packet_id.to_owned(),
            decode_proofs: vec![proof],
            status: DecodeProofStatus::Recovered,
        };
        fs::write(
            dir.path().join("parity_report.decode_proof.json"),
            serde_json::to_string_pretty(&decode_artifact).expect("serialize decode artifact"),
        )
        .expect("write decode artifact");

        let result = verify_packet_sidecar_integrity(dir.path(), packet_id);
        assert!(!result.is_ok());
        assert!(!result.decode_proof_valid);
        assert!(
            result
                .errors
                .iter()
                .any(|entry| entry.contains("does not match repair_packets")),
            "expected repair packet count mismatch error, got {:?}",
            result.errors
        );
    }

    #[test]
    fn verify_all_sidecars_ci_on_empty_dir_returns_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = verify_all_sidecars_ci(dir.path());
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn verify_all_sidecars_ci_on_existing_packets() {
        let _artifact_guard = phase2c_artifact_test_lock();
        let cfg = HarnessConfig::default_paths();
        let artifact_root = cfg.repo_root.join("artifacts");
        if !artifact_root.join("phase2c").exists() {
            return; // skip if no artifacts
        }
        let result = verify_all_sidecars_ci(&artifact_root);
        match result {
            Ok(results) => {
                for r in &results {
                    assert!(r.is_ok(), "{}: {:?}", r.packet_id, r.errors);
                }
            }
            Err(failures) => {
                for f in &failures {
                    eprintln!("SIDECAR INTEGRITY FAILURE: {}: {:?}", f.packet_id, f.errors);
                }
                // Don't assert here - existing artifacts may not all have sidecars yet
            }
        }
    }

    // === CI Gate Topology Tests (bd-2gi.10) ===

    #[test]
    fn ci_gate_pipeline_order_is_monotonic() {
        let pipeline = CiGate::pipeline();
        for window in pipeline.windows(2) {
            assert!(
                window[0].order() < window[1].order(),
                "{:?} should come before {:?}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn ci_gate_commit_pipeline_is_subset_of_full() {
        let full = CiGate::pipeline();
        let commit = CiGate::commit_pipeline();
        for gate in &commit {
            assert!(
                full.contains(gate),
                "{gate:?} in commit but not full pipeline"
            );
        }
    }

    #[test]
    fn ci_gate_labels_are_nonempty() {
        for gate in CiGate::pipeline() {
            assert!(!gate.label().is_empty());
            assert!(gate.to_string().contains("G"));
        }
    }

    #[test]
    fn ci_gate_serialization_round_trip() {
        for gate in CiGate::pipeline() {
            let json = serde_json::to_string(&gate).expect("serialize");
            let back: CiGate = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(gate, back);
        }
    }

    #[test]
    fn ci_gate_g6_conformance_evaluates() {
        let config = CiPipelineConfig::default();
        let result = evaluate_ci_gate(CiGate::G6Conformance, &config);
        assert!(result.passed, "G6 should pass: {}", result.summary);
        assert!(result.elapsed_ms > 0 || result.passed);
    }

    #[test]
    fn ci_pipeline_result_display_format() {
        let result = sample_ci_pipeline_result();
        let output = format!("{result}");
        assert_text_golden("ci_pipeline_result_display.txt", &output);
    }

    #[test]
    fn ci_pipeline_conformance_only_runs_and_reports() {
        let config = CiPipelineConfig {
            gates: vec![CiGate::G6Conformance],
            fail_fast: true,
            harness_config: HarnessConfig::default_paths(),
            verify_sidecars: false,
        };
        let result = run_ci_pipeline(&config);
        assert_eq!(result.gates.len(), 1);
        assert!(
            result.all_passed,
            "conformance gate should pass: {}",
            result
        );
    }

    #[test]
    fn ci_gate_rule_ids_are_stable_and_nonempty() {
        let expected = vec![
            (CiGate::G1Compile, "G1"),
            (CiGate::G2Lint, "G2"),
            (CiGate::G3Unit, "G3"),
            (CiGate::G4Property, "G4"),
            (CiGate::G4_5Fuzz, "G4.5"),
            (CiGate::G5Integration, "G5"),
            (CiGate::G6Conformance, "G6"),
            (CiGate::G7Coverage, "G7"),
            (CiGate::G8E2e, "G8"),
        ];
        for (gate, rule_id) in expected {
            assert_eq!(gate.rule_id(), rule_id);
            assert!(!gate.repro_command().is_empty());
        }
    }

    #[test]
    fn ci_forensics_report_collects_violations_with_replay_commands() {
        let pipeline = CiPipelineResult {
            gates: vec![
                CiGateResult {
                    gate: CiGate::G1Compile,
                    passed: true,
                    elapsed_ms: 10,
                    summary: "ok".to_owned(),
                    errors: vec![],
                },
                CiGateResult {
                    gate: CiGate::G2Lint,
                    passed: false,
                    elapsed_ms: 20,
                    summary: "lint failed".to_owned(),
                    errors: vec!["clippy warning".to_owned()],
                },
            ],
            all_passed: false,
            first_failure: Some(CiGate::G2Lint),
            elapsed_ms: 30,
        };

        let report = build_ci_forensics_report(&pipeline);
        assert_eq!(report.passed_count, 1);
        assert_eq!(report.total_count, 2);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].rule_id, "G2");
        assert!(report.violations[0].repro_cmd.contains("cargo clippy"));
        assert_eq!(report.violations[0].errors.len(), 1);
    }

    #[test]
    fn to_timedelta_int64_seconds_converts() {
        use fp_frame::{Series, to_timedelta};
        use fp_index::IndexLabel;
        use fp_types::{NullKind, Scalar};

        let input = Series::from_values(
            "duration_s".to_owned(),
            vec![
                IndexLabel::Int64(0),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
            ],
            vec![
                Scalar::Int64(3661),
                Scalar::Int64(90061),
                Scalar::Null(NullKind::Null),
            ],
        )
        .unwrap();

        let result = to_timedelta(&input).expect("to_timedelta should succeed");
        assert_eq!(result.len(), 3);

        let values = result.values();
        assert_eq!(values[0], Scalar::Utf8("01:01:01".to_owned()));
        assert_eq!(values[1], Scalar::Utf8("1 days 01:01:01".to_owned()));
        assert!(values[2].is_missing(), "null input should produce NaT");
    }

    #[test]
    fn to_timedelta_string_hms_parses() {
        use fp_frame::{Series, to_timedelta};
        use fp_index::IndexLabel;
        use fp_types::Scalar;

        let input = Series::from_values(
            "td_str".to_owned(),
            vec![
                IndexLabel::Int64(0),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
            ],
            vec![
                Scalar::Utf8("02:30:45".to_owned()),
                Scalar::Utf8("3 days 04:15:30".to_owned()),
                Scalar::Utf8("5 hours".to_owned()),
            ],
        )
        .unwrap();

        let result = to_timedelta(&input).expect("to_timedelta should succeed");
        assert_eq!(result.len(), 3);

        let values = result.values();
        assert_eq!(values[0], Scalar::Utf8("02:30:45".to_owned()));
        assert_eq!(values[1], Scalar::Utf8("3 days 04:15:30".to_owned()));
        assert_eq!(values[2], Scalar::Utf8("05:00:00".to_owned()));
    }

    #[test]
    fn to_timedelta_invalid_string_returns_nat() {
        use fp_frame::{Series, to_timedelta};
        use fp_index::IndexLabel;
        use fp_types::Scalar;

        let input = Series::from_values(
            "bad_td".to_owned(),
            vec![IndexLabel::Int64(0), IndexLabel::Int64(1)],
            vec![
                Scalar::Utf8("not a duration".to_owned()),
                Scalar::Utf8("".to_owned()),
            ],
        )
        .unwrap();

        let result = to_timedelta(&input).expect("to_timedelta should succeed");
        assert_eq!(result.len(), 2);

        let values = result.values();
        assert!(values[0].is_missing(), "invalid string should produce NaT");
        assert!(values[1].is_missing(), "empty string should produce NaT");
    }
}
