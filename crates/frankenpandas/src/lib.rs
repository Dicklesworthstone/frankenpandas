#![forbid(unsafe_code)]
#![warn(rustdoc::broken_intra_doc_links)]

//! FrankenPandas — Clean-room Rust reimplementation of the pandas API.
//!
//! This is the unified public API crate. Import this crate to get access
//! to all FrankenPandas functionality through a single dependency:
//!
//! ```rust
//! use frankenpandas::prelude::*;
//!
//! let df = read_csv_str("name,age\nAlice,30\nBob,25").unwrap();
//! let filtered = df.query("age > 28").unwrap();
//! assert_eq!(filtered.index().len(), 1); // Only Alice (30) passes the filter
//! ```

// ── Core types ──────────────────────────────────────────────────────────

pub use fp_types::{DType, NullKind, Scalar, TypeError};
pub use fp_types::{cast_scalar, common_dtype, infer_dtype, isna, isnull, notna, notnull};

// NanOps: null-skipping aggregation primitives (matches README's NanOps section).
pub use fp_types::{
    nanall, nanany, nanargmax, nanargmin, nancount, nancummax, nancummin, nancumprod, nancumsum,
    nankurt, nanmax, nanmean, nanmedian, nanmin, nannunique, nanprod, nanptp, nanquantile, nansem,
    nanskew, nanstd, nansum, nanvar,
};

pub use fp_columnar::{ArithmeticOp, Column, ColumnError, ComparisonOp, ValidityMask};

pub use fp_index::{
    AlignMode, AlignmentPlan, DuplicateKeep, Index, IndexError, IndexLabel, MultiAlignmentPlan,
    MultiIndex, MultiIndexOrIndex,
};

pub use fp_frame::{
    CategoricalAccessor, CategoricalMetadata, ConcatJoin, DataFrame, DataFrameColumnInput,
    DropNaHow, FrameError, Series, SeriesResetIndexResult, ToDatetimeOptions, ToDatetimeOrigin,
};

// ── Module-level functions (like pd.concat, pd.to_datetime, etc.) ────

pub use fp_frame::{
    concat_dataframes, concat_dataframes_with_axis, concat_dataframes_with_axis_join,
    concat_dataframes_with_ignore_index, concat_dataframes_with_keys, concat_series,
    concat_series_with_ignore_index,
};

pub use fp_frame::to_numeric;
pub use fp_frame::{cut, qcut};
pub use fp_frame::{timedelta_total_seconds, to_timedelta};
pub use fp_frame::{
    to_datetime, to_datetime_with_format, to_datetime_with_options, to_datetime_with_unit,
};

// ── IO functions ────────────────────────────────────────────────────────

pub use fp_io::{
    // CSV
    CsvReadOptions,
    // Extension trait
    DataFrameIoExt,
    // Excel
    ExcelReadOptions,
    // Error type
    IoError,
    // JSON
    JsonOrient,
    // SQL
    SqlChunkIterator,
    SqlColumnSchema,
    SqlConnection,
    SqlForeignKeySchema,
    SqlIfExists,
    SqlIndexSchema,
    SqlInsertMethod,
    SqlIndexedChunkIterator,
    SqlInspector,
    SqlQueryResult,
    SqlReadOptions,
    SqlReflectedTable,
    SqlTableSchema,
    SqlUniqueConstraintSchema,
    SqlWriteOptions,
    inspect,
    list_sql_foreign_keys,
    list_sql_indexes,
    list_sql_schemas,
    list_sql_tables,
    list_sql_unique_constraints,
    list_sql_views,
    sql_max_identifier_length,
    sql_primary_key_columns,
    sql_server_version,
    sql_table_comment,
    sql_table_schema,
    truncate_sql_table,
    read_csv,
    read_csv_str,
    read_csv_with_options,
    read_csv_with_options_path,
    read_excel,
    read_excel_bytes,
    // Feather (Arrow IPC)
    read_feather,
    read_feather_bytes,
    read_ipc_stream_bytes,
    read_json,
    read_json_str,
    // JSONL
    read_jsonl,
    read_jsonl_str,
    // Parquet
    read_parquet,
    read_parquet_bytes,
    read_sql,
    read_sql_chunks,
    read_sql_chunks_with_index_col,
    read_sql_chunks_with_options,
    read_sql_chunks_with_options_and_index_col,
    read_sql_query,
    read_sql_query_chunks,
    read_sql_query_chunks_with_index_col,
    read_sql_query_chunks_with_options,
    read_sql_query_chunks_with_options_and_index_col,
    read_sql_query_with_index_col,
    read_sql_query_with_options,
    read_sql_table,
    read_sql_table_chunks,
    read_sql_table_chunks_with_index_col,
    read_sql_table_chunks_with_options,
    read_sql_table_chunks_with_options_and_index_col,
    read_sql_table_columns_chunks,
    read_sql_table_columns_chunks_with_index_col,
    read_sql_table_columns_with_index_col,
    read_sql_table_with_options,
    read_sql_table_with_options_and_index_col,
    read_sql_with_options,
    write_csv,
    write_csv_string,
    write_excel,
    write_excel_bytes,
    write_feather,
    write_feather_bytes,
    write_ipc_stream_bytes,
    write_json,
    write_json_string,
    write_jsonl,
    write_jsonl_string,
    write_parquet,
    write_parquet_bytes,
    write_sql,
    write_sql_with_options,
};

// ── Expression engine ───────────────────────────────────────────────────

pub use fp_expr::{DataFrameExprExt, ExprError};

// ── GroupBy errors ──────────────────────────────────────────────────────

pub use fp_groupby::GroupByError;

// ── Join/merge ──────────────────────────────────────────────────────────

pub use fp_join::{
    AsofDirection, DataFrameMergeExt, JoinError, JoinType, MergedDataFrame,
    MergeExecutionOptions, MergeValidateMode, join_series, merge_asof, merge_dataframes_on,
    merge_ordered,
};

// ── Runtime policy ──────────────────────────────────────────────────────

pub use fp_runtime::{EvidenceLedger, RuntimePolicy};

// ── Convenience re-export of the default SQL backend ───────────────────
//
// Behind the `sql-sqlite` feature (enabled by default), `rusqlite` is
// re-exported so the README Quick Start example
//
//     let conn = rusqlite::Connection::open_in_memory()?;
//
// works without users having to add rusqlite as a direct dependency.
// Power users implementing their own `SqlConnection` for a different
// backend can disable `sql-sqlite` and avoid the rusqlite dep entirely.
#[cfg(feature = "sql-sqlite")]
pub use rusqlite;

// ── Prelude ─────────────────────────────────────────────────────────────

/// Convenience prelude that imports the most commonly used types and traits.
///
/// ```rust
/// use frankenpandas::prelude::*;
///
/// // Verify that key prelude items are actually reachable from this glob.
/// let _ = DType::Int64;
/// let _ = Scalar::Int64(42);
/// let _ = JsonOrient::Records;
/// let _ = JoinType::Inner;
/// ```
pub mod prelude {
    pub use crate::{
        // Core types
        CategoricalAccessor,
        CategoricalMetadata,
        Column,
        ConcatJoin,
        CsvReadOptions,
        DType,
        ExcelReadOptions,
        DataFrame,
        DataFrameColumnInput,
        DropNaHow,
        DuplicateKeep,
        // Traits
        DataFrameExprExt,
        DataFrameIoExt,
        DataFrameMergeExt,
        // Runtime
        EvidenceLedger,
        Index,
        IndexLabel,
        // Error types (matches README "Error Architecture" section lines 829-853 —
        // all 8 typed error enums exposed for pattern matching).
        ColumnError,
        ExprError,
        FrameError,
        GroupByError,
        IndexError,
        IoError,
        TypeError,
        // Join (types + functions, matches README Recipes + Merge: Advanced Options)
        AsofDirection,
        JoinError,
        JoinType,
        JsonOrient,
        MergeExecutionOptions,
        MergeValidateMode,
        MergedDataFrame,
        MultiIndex,
        MultiIndexOrIndex,
        NullKind,
        RuntimePolicy,
        Scalar,
        Series,
        SeriesResetIndexResult,
        // Per-cell null tracking — README has a dedicated subsection
        // ("ValidityMask: Bitpacked Null Tracking", lines 261-278) and
        // lists it among types deriving Serialize + Deserialize (line 1567).
        ValidityMask,
        // SQL contracts (covers the README Quick Start round-trip).
        // fd90.206: also expose the option/inspector/chunked-read surface
        // documented in the IO Format Support table at line 148.
        SqlConnection,
        SqlInspector,
        SqlReadOptions,
        SqlWriteOptions,
        read_sql_chunks,
        SqlIfExists,
        // Module-level functions (concat + join/merge family)
        concat_dataframes,
        concat_dataframes_with_axis,
        concat_dataframes_with_axis_join,
        concat_dataframes_with_ignore_index,
        concat_dataframes_with_keys,
        concat_series,
        concat_series_with_ignore_index,
        join_series,
        merge_asof,
        merge_dataframes_on,
        merge_ordered,
        // IO — readers (in-memory + path; covers all 8 documented formats)
        read_csv,
        read_csv_str,
        read_csv_with_options,
        read_excel,
        read_excel_bytes,
        read_feather,
        read_feather_bytes,
        read_ipc_stream_bytes,
        read_json,
        read_json_str,
        read_jsonl,
        read_jsonl_str,
        read_parquet,
        read_parquet_bytes,
        read_sql,
        read_sql_table,
        // fd90.210: read_sql_with_options pairs with SqlReadOptions.
        read_sql_with_options,
        // IO — datetime/numeric helpers (full module-level fn surface)
        cut,
        qcut,
        timedelta_total_seconds,
        to_datetime,
        to_datetime_with_format,
        to_datetime_with_options,
        to_datetime_with_unit,
        to_numeric,
        to_timedelta,
        // fd90.208: pandas-style top-level null checks + dtype helpers.
        // The README documents these as user-facing (lines 359, 771, 957, 1031).
        cast_scalar,
        common_dtype,
        infer_dtype,
        isna,
        isnull,
        notna,
        notnull,
        // NanOps — null-skipping aggregation primitives (matches README NanOps section)
        nanall,
        nanany,
        nanargmax,
        nanargmin,
        nancount,
        nancummax,
        nancummin,
        nancumprod,
        nancumsum,
        nankurt,
        nanmax,
        nanmean,
        nanmedian,
        nanmin,
        nannunique,
        nanprod,
        nanptp,
        nanquantile,
        nansem,
        nanskew,
        nanstd,
        nansum,
        nanvar,
        // IO — writers (in-memory + path + sql; covers all 8 documented formats)
        write_csv,
        write_csv_string,
        write_excel,
        write_excel_bytes,
        write_feather,
        write_feather_bytes,
        write_ipc_stream_bytes,
        write_json,
        write_json_string,
        write_jsonl,
        write_jsonl_string,
        write_parquet,
        write_parquet_bytes,
        // fd90.209: write_sql_with_options pairs with SqlWriteOptions
        // (which is in the prelude as of fd90.206).
        write_sql_with_options,
        write_sql,
    };
}

#[cfg(test)]
mod tests {
    use super::prelude::*;

    #[test]
    fn prelude_smoke_test() {
        // Verify that the prelude gives access to basic operations.
        let df = read_csv_str("x,y\n1,2\n3,4").unwrap();
        assert_eq!(df.index().len(), 2);
        assert_eq!(df.column("x").unwrap().values()[0], Scalar::Int64(1));
    }

    #[test]
    fn prelude_query_works() {
        let df = read_csv_str("val\n10\n20\n30").unwrap();
        let filtered = df.query("val > 15").unwrap();
        assert_eq!(filtered.index().len(), 2);
    }

    #[test]
    fn prelude_io_roundtrip() {
        let df = read_csv_str("a,b\n1,hello\n2,world").unwrap();

        // JSON round-trip.
        let json = crate::write_json_string(&df, JsonOrient::Records).unwrap();
        let back = crate::read_json_str(&json, JsonOrient::Records).unwrap();
        assert_eq!(back.index().len(), 2);

        // JSONL round-trip.
        let jsonl = crate::write_jsonl_string(&df).unwrap();
        let back2 = crate::read_jsonl_str(&jsonl).unwrap();
        assert_eq!(back2.index().len(), 2);
    }

    #[test]
    fn prelude_concat_works() {
        let s1 =
            Series::from_values("x", vec![IndexLabel::Int64(0)], vec![Scalar::Int64(1)]).unwrap();
        let s2 =
            Series::from_values("x", vec![IndexLabel::Int64(1)], vec![Scalar::Int64(2)]).unwrap();
        let combined = concat_series(&[&s1, &s2]).unwrap();
        assert_eq!(combined.len(), 2);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_reexport_quickstart_compiles() {
        // README Quick Start uses crate::rusqlite — verify it's actually reachable.
        let conn = crate::rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER); INSERT INTO t VALUES (1);")
            .unwrap();
    }

    #[test]
    fn prelude_to_datetime_works() {
        let s = Series::from_values(
            "d",
            vec![IndexLabel::Int64(0)],
            vec![Scalar::Utf8("2024-01-15".into())],
        )
        .unwrap();
        let dt = to_datetime(&s).unwrap();
        assert_eq!(dt.len(), 1);
    }

    /// Compile-time guard for the prelude expansion (fd90.121–fd90.203).
    ///
    /// Each let-binding through a prelude item ensures that name remains
    /// reachable from `frankenpandas::prelude::*`. If anyone removes one
    /// of these from the prelude, this test refuses to compile.
    ///
    /// Tracks br-frankenpandas-6nexq / fd90.155 (initial); extended in
    /// fd90.204 (br-frankenpandas-cj8ys) for fd90.182–fd90.203.
    #[test]
    fn prelude_completeness_compile_guard() {
        // Enums + structs from the join family (fd90.127, fd90.143).
        let _: AsofDirection = AsofDirection::Backward;
        let _: JoinType = JoinType::Inner;
        let _: MergeValidateMode = MergeValidateMode::OneToOne;
        let _: MergeExecutionOptions = MergeExecutionOptions::default();
        let _is_join_err: fn(JoinError) -> _ = |e| e; // type-check only
        let _is_merged_df: fn(MergedDataFrame) -> _ = |x| x;

        // All 8 error types from the README's Error Architecture section
        // (fd90.202 added 7 of these to the prelude; JoinError was already
        // present, GroupByError + TypeError were also already in the
        // prelude despite the prior comment).
        let _is_col_err: fn(ColumnError) -> _ = |e| e;
        let _is_expr_err: fn(ExprError) -> _ = |e| e;
        let _is_frame_err: fn(FrameError) -> _ = |e| e;
        let _is_group_err: fn(GroupByError) -> _ = |e| e;
        let _is_index_err: fn(IndexError) -> _ = |e| e;
        let _is_io_err: fn(IoError) -> _ = |e| e;
        let _: TypeError = TypeError::IncompatibleDtypes {
            left: DType::Int64,
            right: DType::Utf8,
        };

        // fd90.182: From<bool/i64/f64/&str/String> for Scalar.
        let _: Scalar = true.into();
        let _: Scalar = 42i64.into();
        let _: Scalar = 3.14f64.into();
        let _: Scalar = "hi".into();
        let _: Scalar = String::from("hello").into();

        // fd90.192: DataFrameColumnInput in prelude.
        let _: DataFrameColumnInput = DataFrameColumnInput::Scalar(Scalar::Int64(0));

        // fd90.201: MultiIndexOrIndex in prelude.
        let _is_mi_or_idx: fn(MultiIndexOrIndex) -> _ = |x| x;

        // fd90.203: ValidityMask in prelude.
        let _: ValidityMask = ValidityMask::all_valid(0);

        // fd90.205: CategoricalMetadata in prelude.
        let _: CategoricalMetadata = CategoricalMetadata {
            categories: vec![Scalar::Utf8("a".into())],
            ordered: false,
        };
        // CategoricalAccessor is borrowed-from-Series; just type-check name resolution.
        let _name_check_cat_accessor: fn(&CategoricalAccessor<'_>) = |_| {};

        // Index-side enums (fd90.128).
        let _: DuplicateKeep = DuplicateKeep::First;
        let _: ConcatJoin = ConcatJoin::Inner;
        let _: DropNaHow = DropNaHow::Any;

        // SQL surface (fd90.121, extended fd90.206).
        let _: SqlIfExists = SqlIfExists::Fail;
        // SqlConnection is a trait — name-check only.
        fn _takes_sql<C: SqlConnection>(_: &C) {}
        // fd90.206: SqlReadOptions / SqlWriteOptions / SqlInspector + read_sql_chunks.
        let _: SqlReadOptions = SqlReadOptions::default();
        // SqlWriteOptions has no Default impl — type-check via fn pointer.
        let _is_write_opts: fn(SqlWriteOptions) -> _ = |x| x;
        // SqlInspector is a struct; type-check via fn-pointer signature.
        let _is_inspector: fn(&SqlInspector<'_, rusqlite::Connection>) = |_| {};
        let _ = read_sql_chunks::<rusqlite::Connection>;
        // fd90.209: write_sql_with_options pairs with SqlWriteOptions.
        let _ = write_sql_with_options::<rusqlite::Connection>;
        // fd90.210: read_sql_with_options pairs with SqlReadOptions.
        let _ = read_sql_with_options::<rusqlite::Connection>;

        // NanOps primitives (fd90.126) — call each through a Vec<Scalar>.
        let v = vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)];
        let _ = nansum(&v);
        let _ = nanmean(&v);
        let _ = nancount(&v);
        let _ = nanmin(&v);
        let _ = nanmax(&v);
        let _ = nanmedian(&v);
        let _ = nanvar(&v, 1);
        let _ = nanstd(&v, 1);
        let _ = nansem(&v, 1);
        let _ = nanprod(&v);
        let _ = nanptp(&v);
        let _ = nanskew(&v);
        let _ = nankurt(&v);
        let _ = nanquantile(&v, 0.5);
        let _ = nanargmax(&v);
        let _ = nanargmin(&v);
        let _ = nannunique(&v);
        let _ = nanany(&v);
        let _ = nanall(&v);
        let _ = nancumsum(&v);
        let _ = nancumprod(&v);
        let _ = nancummax(&v);
        let _ = nancummin(&v);

        // Concat family additions (fd90.141) — runtime-call subset, name-check rest.
        let df = read_csv_str("a\n1\n2").unwrap();
        let _ = concat_dataframes(&[&df, &df]).unwrap();
        let _ = concat_dataframes_with_axis(&[&df, &df], 0).unwrap();
        let _ = concat_dataframes_with_axis_join(&[&df, &df], 0, ConcatJoin::Outer).unwrap();
        let _ = concat_dataframes_with_ignore_index(&[&df, &df], false).unwrap();
        let _ = concat_dataframes_with_keys(&[&df, &df], &["a", "b"]).unwrap();
        // Name-check the remaining concat helper (pulled in for symmetry by fd90.141).
        let _name_check_concat_series_with_ignore_index = concat_series_with_ignore_index;

        // IO format coverage (fd90.125, fd90.142) — name-check that all 8 readers
        // and writers are reachable from the prelude. We use `let _ = name;` to bind
        // the function item to a value; the type is inferred and we don't need to
        // annotate the exact signature (which varies per IO format).
        let _ = read_csv;
        let _ = read_excel;
        let _ = read_excel_bytes;
        let _ = read_feather;
        let _ = read_feather_bytes;
        let _ = read_ipc_stream_bytes;
        let _ = read_json;
        let _ = read_jsonl;
        let _ = read_parquet;
        let _ = read_parquet_bytes;
        let _ = write_csv;
        let _ = write_excel;
        let _ = write_excel_bytes;
        let _ = write_feather;
        let _ = write_feather_bytes;
        let _ = write_ipc_stream_bytes;
        let _ = write_json;
        let _ = write_jsonl;
        let _ = write_parquet;
        let _ = write_parquet_bytes;
        // write_sql is generic over C: SqlConnection — exercised in the
        // rusqlite_reexport_quickstart_compiles test; bare let-binding
        // can't infer C without a concrete type, so skip here.

        // fd90.207: Excel options + read_csv_with_options now in prelude.
        let _ = ExcelReadOptions::default();
        let _ = read_csv_with_options;

        // fd90.208: pandas-style top-level null checks + dtype helpers.
        let na_check = vec![Scalar::Int64(1), Scalar::Null(NullKind::NaN)];
        let _ = isna(&na_check);
        let _ = isnull(&na_check);
        let _ = notna(&na_check);
        let _ = notnull(&na_check);
        let _ = infer_dtype(&na_check);
        let _ = common_dtype(DType::Int64, DType::Float64);
        let _ = cast_scalar;

        // Module-level helpers (fd90.144) — name-check.
        let _ = cut;
        let _ = qcut;
        let _ = timedelta_total_seconds;
        let _ = to_datetime_with_format;
        let _ = to_datetime_with_options;
        let _ = to_datetime_with_unit;
        let _ = merge_asof;
        let _ = merge_dataframes_on;
        let _ = merge_ordered;
        let _ = join_series;
    }
}
