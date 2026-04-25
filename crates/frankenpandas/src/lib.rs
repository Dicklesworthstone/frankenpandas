#![forbid(unsafe_code)]

//! FrankenPandas — Clean-room Rust reimplementation of the pandas API.
//!
//! This is the unified public API crate. Import this crate to get access
//! to all FrankenPandas functionality through a single dependency:
//!
//! ```rust,ignore
//! use frankenpandas::prelude::*;
//!
//! let df = read_csv_str("name,age\nAlice,30\nBob,25").unwrap();
//! let filtered = df.query("age > 28").unwrap();
//! println!("{}", filtered);
//! ```

// ── Core types ──────────────────────────────────────────────────────────

pub use fp_types::{DType, NullKind, Scalar};
pub use fp_types::{cast_scalar, common_dtype, infer_dtype, isna, isnull, notna, notnull};

pub use fp_columnar::{ArithmeticOp, Column, ColumnError, ComparisonOp, ValidityMask};

pub use fp_index::{
    AlignMode, AlignmentPlan, Index, IndexError, IndexLabel, MultiAlignmentPlan, MultiIndex,
    MultiIndexOrIndex,
};

pub use fp_frame::{
    CategoricalAccessor, CategoricalMetadata, DataFrame, FrameError, Series,
    SeriesResetIndexResult, ToDatetimeOptions, ToDatetimeOrigin,
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
    SqlQueryResult,
    SqlReadOptions,
    SqlTableSchema,
    SqlUniqueConstraintSchema,
    SqlWriteOptions,
    list_sql_foreign_keys,
    list_sql_indexes,
    list_sql_schemas,
    list_sql_tables,
    list_sql_unique_constraints,
    list_sql_views,
    sql_max_identifier_length,
    sql_primary_key_columns,
    sql_server_version,
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

// ── Join/merge ──────────────────────────────────────────────────────────

pub use fp_join::{
    AsofDirection, DataFrameMergeExt, JoinError, JoinType, MergedDataFrame, join_series,
    merge_asof, merge_dataframes_on, merge_ordered,
};

// ── Runtime policy ──────────────────────────────────────────────────────

pub use fp_runtime::{EvidenceLedger, RuntimePolicy};

// ── Prelude ─────────────────────────────────────────────────────────────

/// Convenience prelude that imports the most commonly used types and traits.
///
/// ```rust,ignore
/// use frankenpandas::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        // Core types
        Column,
        CsvReadOptions,
        DType,
        DataFrame,
        // Traits
        DataFrameExprExt,
        DataFrameIoExt,
        DataFrameMergeExt,
        // Runtime
        EvidenceLedger,
        Index,
        IndexLabel,
        // Join
        JoinType,
        JsonOrient,
        MultiIndex,
        NullKind,
        RuntimePolicy,
        Scalar,
        Series,
        SeriesResetIndexResult,
        // Module-level functions
        concat_dataframes,
        concat_series,
        merge_ordered,
        // IO
        read_csv_str,
        read_feather_bytes,
        read_json_str,
        read_jsonl_str,
        read_parquet_bytes,
        to_datetime,
        to_numeric,
        to_timedelta,
        write_feather_bytes,
        write_parquet_bytes,
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
}
