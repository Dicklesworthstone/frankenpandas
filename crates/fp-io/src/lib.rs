#![forbid(unsafe_code)]
#![warn(rustdoc::broken_intra_doc_links)]

//! IO layer for **frankenpandas**: round-trips between `DataFrame` and the
//! seven supported on-disk / wire formats — CSV, JSON, JSONL, Parquet, Excel
//! (XLSX), Feather (Arrow IPC v2), and SQL.
//!
//! ## Format readers / writers
//!
//! - **CSV**: [`read_csv`], [`read_csv_with_options`], [`write_csv`],
//!   [`write_csv_string`]
//! - **JSON / JSONL**: [`read_json`], [`read_jsonl`], [`write_json`],
//!   [`write_jsonl`]
//! - **Parquet**: [`read_parquet`], [`write_parquet`]
//! - **Excel**: [`read_excel`], [`write_excel`]
//! - **Feather / Arrow IPC**: [`read_feather`], [`write_feather`],
//!   [`read_ipc_stream_bytes`], [`write_ipc_stream_bytes`]
//! - **SQL**: [`read_sql`], [`read_sql_table`], [`write_sql`],
//!   [`write_sql_with_options`], plus the chunked variants
//!   ([`read_sql_chunks`], [`SqlChunkIterator`]).
//!
//! Each format has a per-call options struct ([`CsvReadOptions`],
//! [`ExcelReadOptions`], [`SqlReadOptions`], [`SqlWriteOptions`], ...) so
//! pandas-shaped keyword arguments thread cleanly through the Rust API.
//! The [`DataFrameIoExt`] extension trait adds `df.to_csv(path)` /
//! `df.to_parquet(path)` / etc. methods on `DataFrame` for ergonomic
//! method-chain use.
//!
//! ## SQL backend abstraction
//!
//! SQL IO is built around the [`SqlConnection`] trait — a backend-neutral
//! contract that mirrors the supported subset of pandas /
//! `SQLAlchemy.Inspector`. Concrete backends (today: rusqlite via the
//! `sql-sqlite` feature) implement the trait and inherit:
//!
//! - **Mutation primitives**: [`SqlConnection::query`],
//!   [`SqlConnection::execute_batch`], [`SqlConnection::insert_rows`],
//!   [`SqlConnection::truncate_table`], [`SqlConnection::with_transaction`].
//! - **Capability probes**: [`SqlConnection::dialect_name`],
//!   [`SqlConnection::server_version`], [`SqlConnection::max_param_count`],
//!   [`SqlConnection::max_identifier_length`],
//!   [`SqlConnection::supports_returning`],
//!   [`SqlConnection::supports_schemas`].
//! - **Identifier / parameter shape hooks**:
//!   [`SqlConnection::quote_identifier`],
//!   [`SqlConnection::parameter_marker`].
//! - **Introspection** (matching `SQLAlchemy.Inspector` shape):
//!   [`SqlConnection::list_tables`], [`SqlConnection::list_views`],
//!   [`SqlConnection::list_schemas`], [`SqlConnection::table_schema`],
//!   [`SqlConnection::list_indexes`], [`SqlConnection::list_foreign_keys`],
//!   [`SqlConnection::list_unique_constraints`],
//!   [`SqlConnection::primary_key_columns`],
//!   [`SqlConnection::table_comment`].
//!
//! The [`SqlInspector`] facade wraps a `&C: SqlConnection` and exposes the
//! whole introspection API as methods on a single bundle:
//!
//! ```ignore
//! let inspector = SqlInspector::new(&conn);
//! let bundle = inspector
//!     .reflect_table("users", None)?
//!     .expect("table exists");
//! for col in &bundle.columns {
//!     println!("{}: {:?}", col.name, col.declared_type);
//! }
//! ```
//!
//! [`SqlReflectedTable`] is the bundled metadata returned by
//! [`SqlInspector::reflect_table`] / [`SqlInspector::reflect_all_tables`] /
//! [`SqlInspector::reflect_all_views`] — columns, primary key, indexes,
//! foreign keys, unique constraints, and table-level comment, with
//! per-column lookup helpers.
//!
//! ## Cargo features
//!
//! - `sql-sqlite` (**default**): bind [`SqlConnection`] for
//!   `rusqlite::Connection`.
//! - `sql-postgresql`, `sql-mysql`: placeholder feature flags for the fd90
//!   Phase 2 backend integrations (no concrete bindings yet).
//!
//! Use `default-features = false` to drop the rusqlite dep when only the
//! non-SQL formats are needed.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
};

use arrow::{
    array::{
        Array, BooleanArray, BooleanBuilder, Date32Array, Date64Array, Float64Array,
        Float64Builder, Int64Array, Int64Builder, RecordBatch, StringArray, StringBuilder,
        TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
        TimestampSecondArray,
    },
    datatypes::{DataType as ArrowDataType, Field, Schema, TimeUnit},
};
use csv::{ReaderBuilder, StringRecord, WriterBuilder};
use fp_columnar::{Column, ColumnError};
use fp_frame::{DataFrame, FrameError, Series, ToDatetimeOptions, to_datetime_with_options};
use fp_index::{Index, IndexError, IndexLabel, format_datetime_ns};
use fp_types::{DType, NullKind, Scalar, Timedelta, cast_scalar_owned};
use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum IoError {
    #[error("csv input has no headers")]
    MissingHeaders,
    #[error("csv index column '{0}' not found in headers")]
    MissingIndexColumn(String),
    #[error("duplicate column name '{0}'")]
    DuplicateColumnName(String),
    #[error("usecols contains missing columns: {0:?}")]
    MissingUsecols(Vec<String>),
    #[error("parse_dates contains missing columns: {0:?}")]
    MissingParseDateColumns(Vec<String>),
    #[error("json format error: {0}")]
    JsonFormat(String),
    #[error("parquet error: {0}")]
    Parquet(String),
    #[error("excel error: {0}")]
    Excel(String),
    #[error("arrow ipc error: {0}")]
    Arrow(String),
    #[error("sql error: {0}")]
    Sql(String),
    #[error(transparent)]
    Csv(#[from] csv::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Column(#[from] ColumnError),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Index(#[from] IndexError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonOrient {
    Records,
    Columns,
    Index,
    Split,
    Values,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsvOnBadLines {
    Error,
    Warn,
    Skip,
}

#[derive(Debug, Clone)]
pub struct CsvReadOptions {
    pub delimiter: u8,
    pub has_headers: bool,
    /// Additional NA values to recognize beyond the pandas defaults.
    pub na_values: Vec<String>,
    /// Whether to include the default NaN values when parsing data.
    /// If `na_values` are specified and `keep_default_na` is false, only the
    /// specified `na_values` will be treated as NA.
    /// Matches pandas `keep_default_na` parameter. Default: true.
    pub keep_default_na: bool,
    /// Detect missing value markers (empty strings and the value of na_values).
    /// In data without any NAs, passing `na_filter=false` can improve performance.
    /// Matches pandas `na_filter` parameter. Default: true.
    pub na_filter: bool,
    pub index_col: Option<String>,
    /// Read only these columns (by name). `None` means read all.
    /// Matches pandas `usecols` parameter.
    pub usecols: Option<Vec<String>>,
    /// Maximum number of data rows to read. `None` means read all.
    /// Matches pandas `nrows` parameter.
    pub nrows: Option<usize>,
    /// Number of initial lines to skip at the start of the file (including
    /// the header line when `has_headers` is true).
    /// Matches pandas `skiprows` parameter (when given as int).
    pub skiprows: usize,
    /// Force specific dtypes for columns. Map of column name -> DType.
    /// Matches pandas `dtype` parameter.
    pub dtype: Option<std::collections::HashMap<String, DType>>,
    /// Column names to coerce via pandas-style parse_dates handling.
    /// Currently supports explicit column-name selection.
    pub parse_dates: Option<Vec<String>>,
    /// Column groups to combine and coerce via pandas-style parse_dates handling.
    /// Each group replaces its source columns with a new `<a>_<b>_...` datetime column.
    pub parse_date_combinations: Option<Vec<Vec<String>>>,
    /// Named column groups to combine and parse as datetime, matching
    /// `pd.read_csv(parse_dates={'new_name': ['year', 'month', 'day']})`.
    /// Each `(new_name, [source_cols])` entry replaces its source columns
    /// with a single combined datetime column using the caller-supplied
    /// name instead of the default `<a>_<b>_...` joined form.
    pub parse_date_combinations_named: Option<Vec<(String, Vec<String>)>>,
    /// Character whose lines are treated as comments and skipped entirely.
    /// Must be a single byte (ASCII); multi-byte characters are rejected.
    /// Matches pandas `comment` parameter. Default: `None`.
    pub comment: Option<u8>,
    /// Additional string values to coerce to `true` during CSV parsing.
    /// Matches pandas `true_values` parameter.
    pub true_values: Vec<String>,
    /// Additional string values to coerce to `false` during CSV parsing.
    /// Matches pandas `false_values` parameter.
    pub false_values: Vec<String>,
    /// Character to recognize as the decimal separator when parsing floats.
    /// Matches pandas `decimal` parameter. Default: `.`.
    pub decimal: u8,
    /// How to handle rows with more fields than the header width.
    /// Matches pandas `on_bad_lines` parameter for the supported
    /// `error`/`warn`/`skip` modes.
    pub on_bad_lines: CsvOnBadLines,
    /// Thousands separator stripped from numeric fields before parsing.
    /// Matches pandas `thousands` parameter. Must differ from `decimal`
    /// (otherwise the option is silently ignored, matching pandas).
    pub thousands: Option<u8>,
    /// Number of trailing data rows to drop (after the header is
    /// consumed). Matches pandas `skipfooter` parameter. Default: `0`.
    pub skipfooter: usize,
    /// Character used to quote fields that contain the delimiter, a
    /// newline, or the quote character itself. Defaults to `"` (ASCII
    /// double-quote). Matches pandas `quotechar` parameter.
    pub quotechar: u8,
    /// Character used to escape the quote character inside a quoted
    /// field when `doublequote` is false. `None` disables backslash-
    /// style escaping entirely. Matches pandas `escapechar` parameter.
    pub escapechar: Option<u8>,
    /// When true (the default), a doubled quote character inside a
    /// quoted field is interpreted as a single literal quote. When
    /// false, `escapechar` must be used to quote the quote character.
    /// Matches pandas `doublequote` parameter.
    pub doublequote: bool,
    /// Custom single-byte line terminator. When set, the reader treats
    /// only that byte as a record separator (instead of CRLF/LF).
    /// Matches pandas `lineterminator` (C-engine only). `None` keeps
    /// the default CRLF/LF handling.
    pub lineterminator: Option<u8>,
}

impl Default for CsvReadOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            has_headers: true,
            na_values: Vec::new(),
            keep_default_na: true,
            na_filter: true,
            index_col: None,
            usecols: None,
            nrows: None,
            skiprows: 0,
            dtype: None,
            parse_dates: None,
            parse_date_combinations: None,
            parse_date_combinations_named: None,
            comment: None,
            true_values: Vec::new(),
            false_values: Vec::new(),
            decimal: b'.',
            on_bad_lines: CsvOnBadLines::Error,
            thousands: None,
            quotechar: b'"',
            escapechar: None,
            doublequote: true,
            skipfooter: 0,
            lineterminator: None,
        }
    }
}

pub fn read_csv_str(input: &str) -> Result<DataFrame, IoError> {
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_reader(input.as_bytes());

    let headers_record = reader.headers().cloned().map_err(IoError::from)?;

    if headers_record.is_empty() {
        return Err(IoError::MissingHeaders);
    }
    let headers: Vec<String> = headers_record.iter().map(ToOwned::to_owned).collect();
    reject_duplicate_headers(&headers)?;

    // AG-07: Vec-based column accumulation (O(1) per cell vs O(log c) BTreeMap).
    // Capacity hint from byte length avoids reallocation for typical CSVs.
    let header_count = headers.len();
    let row_hint = input.len() / (header_count * 8).max(1);
    let mut columns: Vec<Vec<Scalar>> = (0..header_count)
        .map(|_| Vec::with_capacity(row_hint))
        .collect();

    let mut row_count: i64 = 0;
    for row in reader.records() {
        let record = row?;
        for (idx, col) in columns.iter_mut().enumerate() {
            let field = record.get(idx).unwrap_or_default();
            col.push(parse_scalar(field));
        }
        row_count += 1;
    }

    let mut out_columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(header_count);
    for (idx, values) in columns.into_iter().enumerate() {
        let name = headers.get(idx).cloned().unwrap_or_default();
        out_columns.insert(name.clone(), Column::from_values(values)?);
        column_order.push(name);
    }

    let index = Index::from_i64((0..row_count).collect());
    Ok(DataFrame::new_with_column_order(
        index,
        out_columns,
        column_order,
    )?)
}

pub fn write_csv_string(frame: &DataFrame) -> Result<String, IoError> {
    write_csv_string_with_options(frame, &CsvWriteOptions::default())
}

/// Options controlling CSV serialization.
///
/// Mirrors the subset of pandas `DataFrame.to_csv` parameters that do
/// not depend on file IO (that layer is handled by `write_csv`).
#[derive(Debug, Clone)]
pub struct CsvWriteOptions {
    /// Field delimiter. Matches pandas `sep`. Default: `,`.
    pub delimiter: u8,
    /// String written for missing values. Matches pandas `na_rep`. Default: `""`.
    pub na_rep: String,
    /// If false, the header row is omitted. Matches pandas `header=False`.
    pub header: bool,
    /// If true, include the index as the first column. Matches pandas `index`.
    pub include_index: bool,
    /// Optional label for the index column header. Matches pandas `index_label`.
    /// When omitted, a named index uses its name and an unnamed index writes an
    /// empty header cell.
    pub index_label: Option<String>,
}

impl Default for CsvWriteOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            na_rep: String::new(),
            header: true,
            include_index: false,
            index_label: None,
        }
    }
}

/// Serialize a DataFrame to CSV with explicit options.
///
/// Matches `pd.DataFrame.to_csv(sep, na_rep, header, index, index_label)`
/// for the in-memory string form. Null and NaN-like values are
/// substituted with `options.na_rep`; all other scalars use the same
/// stringification as the default `write_csv_string`.
pub fn write_csv_string_with_options(
    frame: &DataFrame,
    options: &CsvWriteOptions,
) -> Result<String, IoError> {
    if options.include_index && frame.row_multiindex().is_some() {
        let materialized = materialize_named_row_multiindex_columns(frame)?;
        let mut nested_options = options.clone();
        nested_options.include_index = false;
        nested_options.index_label = None;
        return write_csv_string_with_options(&materialized, &nested_options);
    }

    let mut writer = WriterBuilder::new()
        .delimiter(options.delimiter)
        .from_writer(Vec::new());

    let headers = frame
        .column_names()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    if options.header {
        let mut header_row =
            Vec::with_capacity(headers.len() + if options.include_index { 1 } else { 0 });
        if options.include_index {
            header_row.push(resolve_csv_index_header(frame, options));
        }
        header_row.extend(headers.iter().cloned());
        writer.write_record(&header_row)?;
    }

    for row_idx in 0..frame.index().len() {
        let mut row = Vec::with_capacity(headers.len() + if options.include_index { 1 } else { 0 });
        if options.include_index {
            row.push(frame.index().labels()[row_idx].to_string());
        }
        row.extend(headers.iter().map(|name| {
            let value = frame.column(name).and_then(|column| column.value(row_idx));
            match value {
                Some(scalar) => scalar_to_csv_with_na(scalar, &options.na_rep),
                None => options.na_rep.clone(),
            }
        }));
        writer.write_record(&row)?;
    }

    let bytes = writer.into_inner().map_err(|err| err.into_error())?;
    Ok(String::from_utf8(bytes)?)
}

fn resolve_csv_index_header(frame: &DataFrame, options: &CsvWriteOptions) -> String {
    options
        .index_label
        .clone()
        .or_else(|| frame.index().name().map(ToOwned::to_owned))
        .unwrap_or_default()
}

fn scalar_to_csv_with_na(scalar: &Scalar, na_rep: &str) -> String {
    match scalar {
        Scalar::Null(_) => na_rep.to_owned(),
        Scalar::Float64(v) if v.is_nan() => na_rep.to_owned(),
        Scalar::Timedelta64(v) if *v == Timedelta::NAT => na_rep.to_owned(),
        other => scalar_to_csv(other),
    }
}

/// Default NA values recognized by pandas read_csv.
/// See: <https://pandas.pydata.org/docs/reference/api/pandas.read_csv.html>
const PANDAS_DEFAULT_NA_VALUES: &[&str] = &[
    "", "#N/A", "#N/A N/A", "#NA", "-1.#IND", "-1.#QNAN", "-NaN", "-nan", "1.#IND", "1.#QNAN",
    "<NA>", "N/A", "NA", "NULL", "NaN", "None", "n/a", "nan", "null",
];

fn is_pandas_default_na(s: &str) -> bool {
    PANDAS_DEFAULT_NA_VALUES.contains(&s)
}

fn parse_scalar(field: &str) -> Scalar {
    let trimmed = field.trim();
    if is_pandas_default_na(trimmed) {
        return Scalar::Null(NullKind::Null);
    }

    if let Ok(value) = trimmed.parse::<i64>() {
        return Scalar::Int64(value);
    }
    if let Ok(value) = trimmed.parse::<f64>() {
        return Scalar::Float64(value);
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return Scalar::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Scalar::Bool(false);
    }

    Scalar::Utf8(trimmed.to_owned())
}

fn scalar_to_csv(scalar: &Scalar) -> String {
    match scalar {
        Scalar::Null(_) => String::new(),
        Scalar::Bool(v) => v.to_string(),
        Scalar::Int64(v) => v.to_string(),
        Scalar::Float64(v) => {
            if v.is_nan() {
                String::new()
            } else {
                v.to_string()
            }
        }
        Scalar::Utf8(v) => v.clone(),
        Scalar::Timedelta64(v) => {
            if *v == Timedelta::NAT {
                String::new()
            } else {
                Timedelta::format(*v)
            }
        }
    }
}

/// Parse a field with NA value handling respecting pandas options.
///
/// - `na_filter`: If false, skip NA detection entirely for performance
/// - `keep_default_na`: If true, use pandas default NA values
/// - `na_values`: Additional NA values to recognize
#[allow(clippy::too_many_arguments)]
fn parse_scalar_with_options(
    field: &str,
    na_filter: bool,
    keep_default_na: bool,
    na_values: &[String],
    true_values: &[String],
    false_values: &[String],
    decimal: u8,
    thousands: Option<u8>,
) -> Scalar {
    let trimmed = field.trim();

    // Check NA values only if na_filter is enabled
    if na_filter {
        let is_default_na = keep_default_na && is_pandas_default_na(trimmed);
        let is_custom_na = na_values.iter().any(|na| na == trimmed);
        if is_default_na || is_custom_na {
            return Scalar::Null(NullKind::Null);
        }
    }

    // `thousands` is silently ignored if it equals the decimal separator,
    // matching pandas semantics.
    let thousands_effective = thousands.filter(|t| *t != decimal);
    let numeric_candidate = if let Some(t) = thousands_effective {
        let ch = char::from(t);
        if trimmed.contains(ch) {
            trimmed.replace(ch, "")
        } else {
            trimmed.to_owned()
        }
    } else {
        trimmed.to_owned()
    };

    if let Ok(value) = numeric_candidate.parse::<i64>() {
        return Scalar::Int64(value);
    }

    let float_candidate = if decimal == b'.' {
        numeric_candidate.clone()
    } else {
        numeric_candidate.replace(char::from(decimal), ".")
    };
    if let Ok(value) = float_candidate.parse::<f64>() {
        return Scalar::Float64(value);
    }

    if true_values.iter().any(|value| value == trimmed) {
        return Scalar::Bool(true);
    }
    if false_values.iter().any(|value| value == trimmed) {
        return Scalar::Bool(false);
    }

    if trimmed.eq_ignore_ascii_case("true") {
        return Scalar::Bool(true);
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Scalar::Bool(false);
    }
    Scalar::Utf8(trimmed.to_owned())
}

fn reject_duplicate_headers(headers: &[String]) -> Result<(), IoError> {
    let mut used = BTreeSet::new();
    for name in headers {
        if !used.insert(name.clone()) {
            return Err(IoError::DuplicateColumnName(name.clone()));
        }
    }
    Ok(())
}

fn validate_usecols(headers: &[String], usecols: &[String]) -> Result<(), IoError> {
    let header_set: std::collections::BTreeSet<&String> = headers.iter().collect();
    let mut missing = Vec::new();
    for name in usecols {
        if !header_set.contains(name) {
            missing.push(name.clone());
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(IoError::MissingUsecols(missing))
    }
}

fn validate_parse_dates(headers: &[String], parse_dates: &[String]) -> Result<(), IoError> {
    let header_set: std::collections::BTreeSet<&String> = headers.iter().collect();
    let mut missing = Vec::new();
    for name in parse_dates {
        if !header_set.contains(name) {
            missing.push(name.clone());
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(IoError::MissingParseDateColumns(missing))
    }
}

fn validate_parse_date_combinations(
    headers: &[String],
    parse_date_combinations: &[Vec<String>],
) -> Result<(), IoError> {
    let header_set: std::collections::BTreeSet<&String> = headers.iter().collect();
    let mut missing = BTreeSet::new();
    for combo in parse_date_combinations {
        for name in combo {
            if !header_set.contains(name) {
                missing.insert(name.clone());
            }
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(IoError::MissingParseDateColumns(
            missing.into_iter().collect(),
        ))
    }
}

fn apply_parse_dates(
    headers: &[String],
    columns: &mut [Vec<Scalar>],
    parse_dates: &[String],
) -> Result<(), IoError> {
    if parse_dates.is_empty() {
        return Ok(());
    }

    validate_parse_dates(headers, parse_dates)?;

    for column_name in parse_dates {
        let Some(column_idx) = headers.iter().position(|header| header == column_name) else {
            continue;
        };

        let index_labels = (0..columns[column_idx].len() as i64)
            .map(IndexLabel::Int64)
            .collect::<Vec<_>>();
        let series = Series::from_values(
            column_name.clone(),
            index_labels,
            columns[column_idx].clone(),
        )?;
        if let Some(parsed) = parse_csv_datetime_column(&series)? {
            columns[column_idx] = parsed.values().to_vec();
        }
    }

    Ok(())
}

fn parse_sql_float_text(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut normalized = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        match ch {
            ',' => {}
            '$' if normalized.is_empty() || normalized == "+" || normalized == "-" => {}
            _ => normalized.push(ch),
        }
    }

    if matches!(normalized.as_str(), "" | "+" | "-" | ".") {
        return None;
    }

    let value = normalized.parse::<f64>().ok()?;
    value.is_finite().then_some(value)
}

fn apply_sql_coerce_float(columns: &mut [Vec<Scalar>]) {
    for column in columns {
        let mut saw_text_float = false;
        let mut parsed_values = Vec::with_capacity(column.len());

        for value in column.iter() {
            match value {
                Scalar::Utf8(text) => {
                    let Some(parsed) = parse_sql_float_text(text) else {
                        saw_text_float = false;
                        parsed_values.clear();
                        break;
                    };
                    saw_text_float = true;
                    parsed_values.push(Some(parsed));
                }
                Scalar::Null(_) | Scalar::Int64(_) | Scalar::Float64(_) => {
                    parsed_values.push(None);
                }
                Scalar::Bool(_) | Scalar::Timedelta64(_) => {
                    saw_text_float = false;
                    parsed_values.clear();
                    break;
                }
            }
        }

        if !saw_text_float {
            continue;
        }

        for (value, parsed) in column.iter_mut().zip(parsed_values) {
            if let Some(parsed) = parsed {
                *value = Scalar::Float64(parsed);
            }
        }
    }
}

fn combine_parse_date_values(column_group: &[Vec<Scalar>]) -> Vec<Scalar> {
    let len = column_group.first().map_or(0, Vec::len);
    let mut combined = Vec::with_capacity(len);

    for row in 0..len {
        if column_group
            .iter()
            .any(|column| matches!(column[row], Scalar::Null(_)))
        {
            combined.push(Scalar::Null(NullKind::NaT));
            continue;
        }

        let joined = column_group
            .iter()
            .map(|column| match &column[row] {
                Scalar::Utf8(value) => value.clone(),
                other => other.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        combined.push(Scalar::Utf8(joined));
    }

    combined
}

fn apply_one_parse_date_combination(
    headers: &mut Vec<String>,
    columns: &mut Vec<Vec<Scalar>>,
    combined_name: String,
    sources: &[String],
) -> Result<(), IoError> {
    let mut positions = sources
        .iter()
        .map(|name| {
            headers
                .iter()
                .position(|header| header == name)
                .ok_or_else(|| IoError::MissingParseDateColumns(vec![name.clone()]))
        })
        .collect::<Result<Vec<_>, _>>()?;
    positions.sort_unstable();

    let index_labels = (0..columns[positions[0]].len() as i64)
        .map(IndexLabel::Int64)
        .collect::<Vec<_>>();
    let combined_values = combine_parse_date_values(
        &positions
            .iter()
            .map(|&idx| columns[idx].clone())
            .collect::<Vec<_>>(),
    );
    let combined_series =
        Series::from_values(combined_name.clone(), index_labels, combined_values)?;
    let parsed = parse_csv_datetime_column(&combined_series)?.unwrap_or(combined_series);

    for idx in positions.iter().rev() {
        headers.remove(*idx);
        columns.remove(*idx);
    }
    headers.insert(positions[0], combined_name);
    columns.insert(positions[0], parsed.values().to_vec());
    Ok(())
}

fn parse_csv_datetime_column(series: &Series) -> Result<Option<Series>, IoError> {
    let parsed = to_datetime_with_options(
        series,
        ToDatetimeOptions {
            infer_mixed_timezone: true,
            ..ToDatetimeOptions::default()
        },
    )?;
    let parse_failed = series
        .values()
        .iter()
        .zip(parsed.values())
        .any(|(original, parsed)| !original.is_missing() && parsed.is_missing());

    if parse_failed {
        Ok(None)
    } else {
        Ok(Some(parsed))
    }
}

fn pandas_csv_numeric_column_requires_float(values: &[Scalar]) -> bool {
    let mut saw_numeric = false;
    let mut saw_missing = false;
    let mut saw_float = false;

    for value in values {
        match value {
            Scalar::Int64(_) => saw_numeric = true,
            Scalar::Float64(_) => {
                saw_numeric = true;
                saw_float = true;
            }
            Scalar::Null(_) => saw_missing = true,
            Scalar::Bool(_) | Scalar::Utf8(_) | Scalar::Timedelta64(_) => return false,
        }
    }

    saw_numeric && (saw_missing || saw_float)
}

fn apply_pandas_csv_numeric_promotions(columns: &mut [Vec<Scalar>]) {
    for column in columns {
        if !pandas_csv_numeric_column_requires_float(column) {
            continue;
        }

        for value in column {
            if let Scalar::Int64(v) = value {
                *value = Scalar::Float64(*v as f64);
            }
        }
    }
}

fn apply_parse_date_combinations(
    headers: &mut Vec<String>,
    columns: &mut Vec<Vec<Scalar>>,
    parse_date_combinations: &[Vec<String>],
) -> Result<(), IoError> {
    if parse_date_combinations.is_empty() {
        return Ok(());
    }

    validate_parse_date_combinations(headers, parse_date_combinations)?;

    for combination in parse_date_combinations {
        if combination.is_empty() {
            continue;
        }
        let combined_name = combination.join("_");
        apply_one_parse_date_combination(headers, columns, combined_name, combination)?;
    }

    Ok(())
}

fn apply_parse_date_combinations_named(
    headers: &mut Vec<String>,
    columns: &mut Vec<Vec<Scalar>>,
    parse_date_combinations_named: &[(String, Vec<String>)],
) -> Result<(), IoError> {
    if parse_date_combinations_named.is_empty() {
        return Ok(());
    }

    let mut assigned_names: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (new_name, _) in parse_date_combinations_named {
        if !assigned_names.insert(new_name.clone()) {
            return Err(IoError::DuplicateColumnName(new_name.clone()));
        }
    }

    let combos_only: Vec<Vec<String>> = parse_date_combinations_named
        .iter()
        .map(|(_, sources)| sources.clone())
        .collect();
    validate_parse_date_combinations(headers, &combos_only)?;

    for (new_name, sources) in parse_date_combinations_named {
        if sources.is_empty() {
            continue;
        }
        apply_one_parse_date_combination(headers, columns, new_name.clone(), sources)?;
    }

    Ok(())
}

fn append_csv_record(columns: &mut [Vec<Scalar>], record: &StringRecord, options: &CsvReadOptions) {
    for (idx, col) in columns.iter_mut().enumerate() {
        let field = record.get(idx).unwrap_or_default();
        col.push(parse_scalar_with_options(
            field,
            options.na_filter,
            options.keep_default_na,
            &options.na_values,
            &options.true_values,
            &options.false_values,
            options.decimal,
            options.thousands,
        ));
    }
}

fn should_skip_bad_csv_record(
    record: &StringRecord,
    expected_fields: usize,
    on_bad_lines: CsvOnBadLines,
) -> bool {
    if record.len() <= expected_fields {
        return false;
    }

    match on_bad_lines {
        CsvOnBadLines::Error => false,
        CsvOnBadLines::Warn => {
            eprintln!(
                "Skipping bad CSV line: expected {expected_fields} fields, found {}",
                record.len()
            );
            true
        }
        CsvOnBadLines::Skip => true,
    }
}

// ── CSV with options ───────────────────────────────────────────────────

pub fn read_csv_with_options(input: &str, options: &CsvReadOptions) -> Result<DataFrame, IoError> {
    let mut builder = ReaderBuilder::new();
    builder
        .has_headers(false)
        .delimiter(options.delimiter)
        .quote(options.quotechar)
        .double_quote(options.doublequote)
        .escape(options.escapechar);
    if options.on_bad_lines != CsvOnBadLines::Error {
        builder.flexible(true);
    }
    if let Some(c) = options.comment {
        builder.comment(Some(c));
    }
    if let Some(term) = options.lineterminator {
        builder.terminator(csv::Terminator::Any(term));
    }
    let mut reader = builder.from_reader(input.as_bytes());

    let max_rows = options.nrows.unwrap_or(usize::MAX);
    let skip = options.skiprows;

    let mut records = reader.records();
    for _ in 0..skip {
        if records.next().transpose()?.is_none() {
            return Err(IoError::MissingHeaders);
        }
    }

    let mut row_count: i64 = 0;
    let (headers, mut columns) = if options.has_headers {
        let headers_record = records.next().transpose()?.ok_or(IoError::MissingHeaders)?;
        if headers_record.is_empty() {
            return Err(IoError::MissingHeaders);
        }

        let header_count = headers_record.len();
        let row_hint = input.len() / (header_count * 8).max(1);
        let columns: Vec<Vec<Scalar>> = (0..header_count)
            .map(|_| Vec::with_capacity(row_hint))
            .collect();

        (
            headers_record
                .iter()
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>(),
            columns,
        )
    } else {
        let first_record = records.next().transpose()?.ok_or(IoError::MissingHeaders)?;
        if first_record.is_empty() {
            return Err(IoError::MissingHeaders);
        }

        let header_count = first_record.len();
        let row_hint = input.len() / (header_count * 8).max(1);
        let mut columns: Vec<Vec<Scalar>> = (0..header_count)
            .map(|_| Vec::with_capacity(row_hint))
            .collect();

        if (row_count as usize) < max_rows {
            append_csv_record(&mut columns, &first_record, options);
            row_count += 1;
        }

        (
            (0..header_count)
                .map(|idx| format!("column_{idx}"))
                .collect(),
            columns,
        )
    };

    for row in records {
        if (row_count as usize) >= max_rows {
            break;
        }
        let record = row?;
        if should_skip_bad_csv_record(&record, columns.len(), options.on_bad_lines) {
            continue;
        }
        append_csv_record(&mut columns, &record, options);
        row_count += 1;
    }

    // Drop the last `skipfooter` data rows. Matches pandas semantics:
    // footer rows are dropped *after* header parsing and nrows limit.
    if options.skipfooter > 0 && (row_count as usize) > 0 {
        let drop = options.skipfooter.min(row_count as usize);
        for col in columns.iter_mut() {
            let new_len = col.len().saturating_sub(drop);
            col.truncate(new_len);
        }
        row_count -= drop as i64;
    }
    reject_duplicate_headers(&headers)?;
    if let Some(ref usecols) = options.usecols {
        validate_usecols(&headers, usecols)?;
    }

    // Apply usecols filter: keep only selected columns.
    let (mut headers, mut columns) = if let Some(ref usecols) = options.usecols {
        let mut fh = Vec::new();
        let mut fc = Vec::new();
        for (h, c) in headers.into_iter().zip(columns) {
            if usecols.contains(&h) {
                fh.push(h);
                fc.push(c);
            }
        }
        (fh, fc)
    } else {
        (headers, columns)
    };

    if let Some(ref parse_date_combinations) = options.parse_date_combinations {
        apply_parse_date_combinations(&mut headers, &mut columns, parse_date_combinations)?;
    }

    if let Some(ref named) = options.parse_date_combinations_named {
        apply_parse_date_combinations_named(&mut headers, &mut columns, named)?;
    }

    if let Some(ref parse_dates) = options.parse_dates {
        apply_parse_dates(&headers, &mut columns, parse_dates)?;
    }

    apply_pandas_csv_numeric_promotions(&mut columns);

    // Apply dtype coercion if specified.
    if let Some(ref dtype_map) = options.dtype {
        for (i, name) in headers.iter().enumerate() {
            if let Some(&target_dt) = dtype_map.get(name) {
                let coerced = columns[i]
                    .iter()
                    .map(|v| fp_types::cast_scalar(v, target_dt))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|err| IoError::Column(ColumnError::from(err)))?;
                columns[i] = coerced;
            }
        }
    }

    let header_count = headers.len();

    // If index_col is set, extract that column as the index
    if let Some(ref idx_col_name) = options.index_col {
        let idx_pos = headers
            .iter()
            .position(|h| h == idx_col_name)
            .ok_or_else(|| IoError::MissingIndexColumn(idx_col_name.clone()))?;

        let index_values = columns.remove(idx_pos);
        let index_labels: Vec<fp_index::IndexLabel> = index_values
            .into_iter()
            .map(|s| match s {
                Scalar::Int64(v) => fp_index::IndexLabel::Int64(v),
                Scalar::Utf8(v) => fp_index::IndexLabel::Utf8(v),
                Scalar::Float64(v) => fp_index::IndexLabel::Utf8(v.to_string()),
                Scalar::Bool(v) => fp_index::IndexLabel::Utf8(v.to_string()),
                Scalar::Null(_) => fp_index::IndexLabel::Utf8("<null>".to_owned()),
                Scalar::Timedelta64(v) => {
                    if v == Timedelta::NAT {
                        fp_index::IndexLabel::Utf8("<NaT>".to_owned())
                    } else {
                        fp_index::IndexLabel::Utf8(Timedelta::format(v))
                    }
                }
            })
            .collect();
        let index = Index::new(index_labels);

        let mut out_columns = BTreeMap::new();
        let mut column_order = Vec::with_capacity(headers.len() - 1);
        let mut col_idx = 0;
        for (orig_idx, _) in headers.iter().enumerate() {
            if orig_idx == idx_pos {
                continue;
            }
            let name = headers.get(orig_idx).cloned().unwrap_or_default();
            out_columns.insert(name.clone(), Column::from_values(columns[col_idx].clone())?);
            column_order.push(name);
            col_idx += 1;
        }
        Ok(DataFrame::new_with_column_order(
            index,
            out_columns,
            column_order,
        )?)
    } else {
        let mut out_columns = BTreeMap::new();
        let mut column_order = Vec::with_capacity(header_count);
        for (idx, values) in columns.into_iter().enumerate() {
            let name = headers.get(idx).cloned().unwrap_or_default();
            out_columns.insert(name.clone(), Column::from_values(values)?);
            column_order.push(name);
        }
        let index = Index::from_i64((0..row_count).collect());
        Ok(DataFrame::new_with_column_order(
            index,
            out_columns,
            column_order,
        )?)
    }
}

/// Read CSV and promote the named columns into a row index / row MultiIndex.
///
/// For multiple names this mirrors pandas `index_col=[...]`.
pub fn read_csv_with_index_cols(
    input: &str,
    options: &CsvReadOptions,
    index_cols: &[&str],
) -> Result<DataFrame, IoError> {
    let frame = read_csv_with_options(input, options)?;
    promote_frame_index_columns(&frame, index_cols)
}

// ── File-based CSV ─────────────────────────────────────────────────────

pub fn read_csv(path: &Path) -> Result<DataFrame, IoError> {
    read_csv_with_options_path(path, &CsvReadOptions::default())
}

pub fn read_csv_with_options_path(
    path: &Path,
    options: &CsvReadOptions,
) -> Result<DataFrame, IoError> {
    let content = std::fs::read_to_string(path)?;
    read_csv_with_options(&content, options)
}

pub fn read_csv_with_index_cols_path(
    path: &Path,
    options: &CsvReadOptions,
    index_cols: &[&str],
) -> Result<DataFrame, IoError> {
    let content = std::fs::read_to_string(path)?;
    read_csv_with_index_cols(&content, options, index_cols)
}

pub fn write_csv(frame: &DataFrame, path: &Path) -> Result<(), IoError> {
    let content = write_csv_string(frame)?;
    std::fs::write(path, content)?;
    Ok(())
}

// ── JSON IO ────────────────────────────────────────────────────────────

fn json_value_to_scalar(val: &serde_json::Value) -> Scalar {
    match val {
        serde_json::Value::Null => Scalar::Null(NullKind::Null),
        serde_json::Value::Bool(b) => Scalar::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Scalar::Int64(i)
            } else if let Some(f) = n.as_f64() {
                Scalar::Float64(f)
            } else {
                Scalar::Utf8(n.to_string())
            }
        }
        serde_json::Value::String(s) => Scalar::Utf8(s.clone()),
        other => Scalar::Utf8(other.to_string()),
    }
}

fn parse_json_value_allowing_pandas_nan(input: &str) -> Result<serde_json::Value, IoError> {
    match serde_json::from_str(input) {
        Ok(value) => Ok(value),
        Err(original) => {
            let normalized = normalize_bare_json_nan_tokens(input);
            if normalized == input {
                return Err(original.into());
            }
            serde_json::from_str(&normalized).map_err(IoError::from)
        }
    }
}

fn normalize_bare_json_nan_tokens(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;

    while index < input.len() {
        let rest = &input[index..];
        let Some(ch) = rest.chars().next() else {
            break;
        };

        if in_string {
            output.push(ch);
            index += ch.len_utf8();
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            output.push(ch);
            index += ch.len_utf8();
            continue;
        }

        if rest.starts_with("NaN")
            && is_json_value_start_boundary(input, index)
            && is_json_value_end_boundary(input, index + 3)
        {
            output.push_str("null");
            index += 3;
            continue;
        }

        output.push(ch);
        index += ch.len_utf8();
    }

    output
}

fn is_json_value_start_boundary(input: &str, index: usize) -> bool {
    input[..index]
        .chars()
        .rev()
        .find(|ch| !ch.is_whitespace())
        .is_none_or(|ch| matches!(ch, ':' | '[' | ','))
}

fn is_json_value_end_boundary(input: &str, index: usize) -> bool {
    input[index..]
        .chars()
        .find(|ch| !ch.is_whitespace())
        .is_none_or(|ch| matches!(ch, ',' | ']' | '}'))
}

fn column_from_json_records_values(values: Vec<Scalar>) -> Result<Column, IoError> {
    let saw_utf8 = values.iter().any(|value| matches!(value, Scalar::Utf8(_)));
    let saw_missing = values.iter().any(Scalar::is_missing);
    let saw_numeric_like = values.iter().any(|value| {
        matches!(
            value,
            Scalar::Int64(_) | Scalar::Float64(_) | Scalar::Bool(_)
        )
    });

    if !saw_utf8 && saw_missing && (saw_numeric_like || values.iter().all(Scalar::is_missing)) {
        let promoted = values
            .into_iter()
            .map(|value| match value {
                Scalar::Int64(value) => Scalar::Float64(value as f64),
                Scalar::Bool(value) => Scalar::Float64(if value { 1.0 } else { 0.0 }),
                Scalar::Null(_) => Scalar::Null(NullKind::NaN),
                other => other,
            })
            .collect();
        return Column::new(DType::Float64, promoted).map_err(IoError::from);
    }

    Column::from_values(values).map_err(IoError::from)
}

fn scalar_to_json(scalar: &Scalar) -> serde_json::Value {
    match scalar {
        Scalar::Null(_) => serde_json::Value::Null,
        Scalar::Bool(b) => serde_json::Value::Bool(*b),
        Scalar::Int64(i) => serde_json::json!(*i),
        Scalar::Float64(f) => {
            if f.is_nan() || f.is_infinite() {
                serde_json::Value::Null
            } else {
                serde_json::json!(*f)
            }
        }
        Scalar::Utf8(s) => serde_json::Value::String(s.clone()),
        Scalar::Timedelta64(v) => {
            if *v == Timedelta::NAT {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(Timedelta::format(*v))
            }
        }
    }
}

fn column_promotes_int_json_values_to_float(values: &[Scalar]) -> bool {
    let mut saw_int = false;
    let mut saw_missing = false;

    for value in values {
        match value {
            Scalar::Int64(_) => saw_int = true,
            Scalar::Null(_) => saw_missing = true,
            Scalar::Float64(_) => {}
            Scalar::Bool(_) | Scalar::Utf8(_) | Scalar::Timedelta64(_) => return false,
        }
    }

    saw_int && saw_missing
}

fn scalar_to_json_with_column_promotion(
    scalar: &Scalar,
    promote_int_to_float: bool,
) -> serde_json::Value {
    if promote_int_to_float && let Scalar::Int64(v) = scalar {
        return serde_json::json!(*v as f64);
    }
    scalar_to_json(scalar)
}

fn json_value_to_index_label(value: &serde_json::Value) -> IndexLabel {
    match value {
        serde_json::Value::Number(n) => n
            .as_i64()
            .map(IndexLabel::Int64)
            .unwrap_or_else(|| IndexLabel::Utf8(n.to_string())),
        serde_json::Value::String(s) => IndexLabel::Utf8(s.clone()),
        serde_json::Value::Bool(b) => IndexLabel::Utf8(b.to_string()),
        serde_json::Value::Null => IndexLabel::Utf8("null".to_owned()),
        other => IndexLabel::Utf8(other.to_string()),
    }
}

fn json_value_to_column_name(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_owned(),
        other => other.to_string(),
    }
}

fn json_key_to_index_label(value: &str) -> IndexLabel {
    value
        .parse::<i64>()
        .map(IndexLabel::Int64)
        .unwrap_or_else(|_| IndexLabel::Utf8(value.to_owned()))
}

fn index_label_to_json(label: &IndexLabel) -> serde_json::Value {
    match label {
        IndexLabel::Int64(v) => serde_json::json!(*v),
        IndexLabel::Utf8(v) => serde_json::Value::String(v.clone()),
        IndexLabel::Timedelta64(ns) => serde_json::json!(*ns),
        IndexLabel::Datetime64(ns) => serde_json::json!(*ns),
    }
}

const SYNTHETIC_ROW_MULTIINDEX_PREFIX: &str = "__index_level_";

fn index_label_to_scalar_value(label: &IndexLabel) -> Scalar {
    match label {
        IndexLabel::Int64(v) => Scalar::Int64(*v),
        IndexLabel::Utf8(v) => Scalar::Utf8(v.clone()),
        IndexLabel::Timedelta64(v) => Scalar::Timedelta64(*v),
        IndexLabel::Datetime64(v) => Scalar::Utf8(format_datetime_ns(*v)),
    }
}

fn synthetic_row_multiindex_names(nlevels: usize) -> Vec<String> {
    (0..nlevels)
        .map(|level| format!("{SYNTHETIC_ROW_MULTIINDEX_PREFIX}{level}__"))
        .collect()
}

fn materialize_row_multiindex_columns(
    frame: &DataFrame,
    names: &[String],
) -> Result<DataFrame, IoError> {
    let Some(row_multiindex) = frame.row_multiindex() else {
        return Ok(frame.clone());
    };

    let mut columns = BTreeMap::new();
    let mut column_order = Vec::with_capacity(names.len() + frame.column_names().len());
    for (level, name) in names.iter().enumerate() {
        let level_index = row_multiindex.get_level_values(level)?;
        let values = level_index
            .labels()
            .iter()
            .map(index_label_to_scalar_value)
            .collect::<Vec<_>>();
        columns.insert(name.clone(), Column::from_values(values)?);
        column_order.push(name.clone());
    }

    for name in frame.column_names() {
        let column = frame
            .column(name)
            .ok_or_else(|| {
                IoError::Frame(FrameError::CompatibilityRejected(format!(
                    "column not found: '{name}'"
                )))
            })?
            .clone();
        columns.insert(name.clone(), column);
        column_order.push(name.clone());
    }

    let index = Index::from_i64((0..frame.len() as i64).collect());
    DataFrame::new_with_column_order(index, columns, column_order).map_err(IoError::from)
}

fn materialize_named_row_multiindex_columns(frame: &DataFrame) -> Result<DataFrame, IoError> {
    if frame.row_multiindex().is_some() {
        frame.reset_index(false).map_err(IoError::from)
    } else {
        Ok(frame.clone())
    }
}

fn materialize_synthetic_row_multiindex_columns(frame: &DataFrame) -> Result<DataFrame, IoError> {
    let Some(row_multiindex) = frame.row_multiindex() else {
        return Ok(frame.clone());
    };
    let names = synthetic_row_multiindex_names(row_multiindex.nlevels());
    materialize_row_multiindex_columns(frame, &names)
}

fn promote_frame_index_columns(
    frame: &DataFrame,
    index_cols: &[&str],
) -> Result<DataFrame, IoError> {
    if index_cols.is_empty() {
        return Ok(frame.clone());
    }
    if index_cols.len() == 1 {
        frame.set_index(index_cols[0], true).map_err(IoError::from)
    } else {
        frame
            .set_index_multi(index_cols, true, "|")
            .map_err(IoError::from)
    }
}

fn detect_synthetic_row_multiindex_columns(frame: &DataFrame) -> Vec<String> {
    let mut out = Vec::new();
    for (level, name) in frame.column_names().iter().enumerate() {
        let expected = format!("{SYNTHETIC_ROW_MULTIINDEX_PREFIX}{level}__");
        if **name == expected {
            out.push(expected);
        } else {
            break;
        }
    }
    out
}

fn promote_synthetic_row_multiindex_if_present(frame: &DataFrame) -> Result<DataFrame, IoError> {
    let synthetic_cols = detect_synthetic_row_multiindex_columns(frame);
    if synthetic_cols.len() < 2 {
        return Ok(frame.clone());
    }
    let refs = synthetic_cols
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    promote_frame_index_columns(frame, &refs)
}

pub fn read_json_str(input: &str, orient: JsonOrient) -> Result<DataFrame, IoError> {
    let parsed: serde_json::Value = serde_json::from_str(input)?;

    match orient {
        JsonOrient::Records => {
            let arr = parsed
                .as_array()
                .ok_or_else(|| IoError::JsonFormat("expected array for records orient".into()))?;
            if arr.is_empty() {
                return Ok(DataFrame::new(Index::new(Vec::new()), BTreeMap::new())?);
            }

            // Collect column names from all records to handle heterogeneous keys
            let mut col_names_set = std::collections::BTreeSet::new();
            let mut col_names = Vec::new();
            for record in arr {
                let obj = record
                    .as_object()
                    .ok_or_else(|| IoError::JsonFormat("each record must be an object".into()))?;
                for key in obj.keys() {
                    if col_names_set.insert(key.clone()) {
                        col_names.push(key.clone());
                    }
                }
            }

            let mut columns: BTreeMap<String, Vec<Scalar>> = BTreeMap::new();
            for name in &col_names {
                columns.insert(name.clone(), Vec::with_capacity(arr.len()));
            }

            for record in arr {
                let obj = record
                    .as_object()
                    .ok_or_else(|| IoError::JsonFormat("each record must be an object".into()))?;
                for name in &col_names {
                    let val = obj.get(name).unwrap_or(&serde_json::Value::Null);
                    columns
                        .get_mut(name)
                        .ok_or_else(|| {
                            IoError::JsonFormat(format!(
                                "records orient missing column accumulator for '{name}'"
                            ))
                        })?
                        .push(json_value_to_scalar(val));
                }
            }

            let row_count = arr.len() as i64;
            let mut out = BTreeMap::new();
            for (name, vals) in columns {
                out.insert(name, Column::from_values(vals)?);
            }
            let index = Index::from_i64((0..row_count).collect());
            let frame = DataFrame::new_with_column_order(index, out, col_names)?;
            promote_synthetic_row_multiindex_if_present(&frame)
        }
        JsonOrient::Columns => {
            let obj = parsed
                .as_object()
                .ok_or_else(|| IoError::JsonFormat("expected object for columns orient".into()))?;

            if obj.is_empty() {
                return Ok(DataFrame::new(Index::new(Vec::new()), BTreeMap::new())?);
            }

            let mut raw_columns: BTreeMap<String, Vec<Scalar>> = BTreeMap::new();
            let mut column_order = Vec::with_capacity(obj.len());
            let mut index_labels = Vec::new();
            let mut index_lookup = BTreeMap::new();
            for (col_name, col_data) in obj {
                let col_obj = col_data.as_object().ok_or_else(|| {
                    IoError::JsonFormat("column data must be {index: val}".into())
                })?;
                let mut values = vec![Scalar::Null(NullKind::Null); index_labels.len()];
                for (label_key, val) in col_obj {
                    let label = json_key_to_index_label(label_key);
                    let row_idx = if let Some(&existing_idx) = index_lookup.get(&label) {
                        existing_idx
                    } else {
                        let next_idx = index_labels.len();
                        index_labels.push(label.clone());
                        index_lookup.insert(label, next_idx);
                        for existing_values in raw_columns.values_mut() {
                            existing_values.push(Scalar::Null(NullKind::Null));
                        }
                        values.push(Scalar::Null(NullKind::Null));
                        next_idx
                    };
                    if row_idx >= values.len() {
                        values.resize(index_labels.len(), Scalar::Null(NullKind::Null));
                    }
                    values[row_idx] = json_value_to_scalar(val);
                }
                if values.len() < index_labels.len() {
                    values.resize(index_labels.len(), Scalar::Null(NullKind::Null));
                }
                raw_columns.insert(col_name.clone(), values);
                column_order.push(col_name.clone());
            }

            let mut out = BTreeMap::new();
            for (name, vals) in raw_columns {
                out.insert(name, Column::from_values(vals)?);
            }

            let frame =
                DataFrame::new_with_column_order(Index::new(index_labels), out, column_order)?;
            promote_synthetic_row_multiindex_if_present(&frame)
        }
        JsonOrient::Index => {
            let obj = parsed
                .as_object()
                .ok_or_else(|| IoError::JsonFormat("expected object for index orient".into()))?;

            if obj.is_empty() {
                return Ok(DataFrame::new(Index::new(Vec::new()), BTreeMap::new())?);
            }

            let mut index_labels = Vec::with_capacity(obj.len());
            let mut columns: BTreeMap<String, Vec<Scalar>> = BTreeMap::new();
            let mut column_order = Vec::new();
            let mut seen_columns = std::collections::HashSet::new();

            for (row_label, row_data) in obj {
                let row_obj = row_data.as_object().ok_or_else(|| {
                    IoError::JsonFormat("index orient rows must be objects".into())
                })?;

                let row_idx = index_labels.len();

                // Pre-fill this row as null for all known columns, then overwrite present cells.
                for values in columns.values_mut() {
                    values.push(Scalar::Null(NullKind::Null));
                }

                let parsed_label = row_label
                    .parse::<i64>()
                    .map(IndexLabel::Int64)
                    .unwrap_or_else(|_| IndexLabel::Utf8(row_label.clone()));
                index_labels.push(parsed_label);

                for (col_name, value) in row_obj {
                    if seen_columns.insert(col_name.clone()) {
                        column_order.push(col_name.clone());
                    }
                    let scalar = json_value_to_scalar(value);
                    if let Some(values) = columns.get_mut(col_name) {
                        values[row_idx] = scalar;
                    } else {
                        let mut values = vec![Scalar::Null(NullKind::Null); row_idx + 1];
                        values[row_idx] = scalar;
                        columns.insert(col_name.clone(), values);
                    }
                }
            }

            let mut out = BTreeMap::new();
            for (name, vals) in columns {
                out.insert(name, Column::from_values(vals)?);
            }
            let frame =
                DataFrame::new_with_column_order(Index::new(index_labels), out, column_order)?;
            promote_synthetic_row_multiindex_if_present(&frame)
        }
        JsonOrient::Split => {
            let obj = parsed
                .as_object()
                .ok_or_else(|| IoError::JsonFormat("expected object for split orient".into()))?;

            let col_names: Vec<String> = obj
                .get("columns")
                .and_then(|v| v.as_array())
                .ok_or_else(|| IoError::JsonFormat("split orient needs 'columns' array".into()))?
                .iter()
                .map(json_value_to_column_name)
                .collect();
            reject_duplicate_headers(&col_names)?;

            let data = obj
                .get("data")
                .and_then(|v| v.as_array())
                .ok_or_else(|| IoError::JsonFormat("split orient needs 'data' array".into()))?;

            let explicit_index = obj
                .get("index")
                .map(|v| {
                    v.as_array()
                        .ok_or_else(|| {
                            IoError::JsonFormat("split orient 'index' must be an array".into())
                        })
                        .map(|arr| {
                            arr.iter()
                                .map(json_value_to_index_label)
                                .collect::<Vec<_>>()
                        })
                })
                .transpose()?;

            let mut columns: BTreeMap<String, Vec<Scalar>> = BTreeMap::new();
            for name in &col_names {
                columns.insert(name.clone(), Vec::with_capacity(data.len()));
            }

            for (row_idx, row) in data.iter().enumerate() {
                let arr = row
                    .as_array()
                    .ok_or_else(|| IoError::JsonFormat("each data row must be an array".into()))?;
                if arr.len() != col_names.len() {
                    return Err(IoError::JsonFormat(format!(
                        "split orient row {row_idx} length ({}) does not match columns length ({})",
                        arr.len(),
                        col_names.len()
                    )));
                }
                for (i, name) in col_names.iter().enumerate() {
                    let val = arr.get(i).unwrap_or(&serde_json::Value::Null);
                    columns
                        .get_mut(name)
                        .ok_or_else(|| {
                            IoError::JsonFormat(format!(
                                "split orient missing column accumulator for '{name}'"
                            ))
                        })?
                        .push(json_value_to_scalar(val));
                }
            }

            let row_count = data.len() as i64;
            let mut out = BTreeMap::new();
            for (name, vals) in columns {
                out.insert(name, Column::from_values(vals)?);
            }
            let index = match explicit_index {
                Some(labels) => {
                    if labels.len() != row_count as usize {
                        return Err(IoError::JsonFormat(format!(
                            "split orient index length ({}) must match data row count ({row_count})",
                            labels.len()
                        )));
                    }
                    Index::new(labels)
                }
                None => Index::from_i64((0..row_count).collect()),
            };
            let frame = DataFrame::new_with_column_order(index, out, col_names)?;
            promote_synthetic_row_multiindex_if_present(&frame)
        }
        JsonOrient::Values => {
            let rows = parsed
                .as_array()
                .ok_or_else(|| IoError::JsonFormat("expected array for values orient".into()))?;

            if rows.is_empty() {
                return Ok(DataFrame::new(Index::new(Vec::new()), BTreeMap::new())?);
            }

            let mut width = 0usize;
            for row in rows {
                let arr = row.as_array().ok_or_else(|| {
                    IoError::JsonFormat("each values row must be an array".into())
                })?;
                width = width.max(arr.len());
            }

            let column_order: Vec<String> = (0..width).map(|idx| idx.to_string()).collect();
            let mut columns: BTreeMap<String, Vec<Scalar>> = column_order
                .iter()
                .cloned()
                .map(|name| (name, Vec::with_capacity(rows.len())))
                .collect();

            for row in rows {
                let arr = row.as_array().ok_or_else(|| {
                    IoError::JsonFormat("each values row must be an array".into())
                })?;
                for (col_idx, name) in column_order.iter().enumerate() {
                    let val = arr.get(col_idx).unwrap_or(&serde_json::Value::Null);
                    columns
                        .get_mut(name)
                        .ok_or_else(|| {
                            IoError::JsonFormat(format!(
                                "values orient missing column accumulator for '{name}'"
                            ))
                        })?
                        .push(json_value_to_scalar(val));
                }
            }

            let mut out = BTreeMap::new();
            for (name, vals) in columns {
                out.insert(name, Column::from_values(vals)?);
            }
            let index = Index::from_i64((0..rows.len() as i64).collect());
            let frame = DataFrame::new_with_column_order(index, out, column_order)?;
            promote_synthetic_row_multiindex_if_present(&frame)
        }
    }
}

pub fn write_json_string(frame: &DataFrame, orient: JsonOrient) -> Result<String, IoError> {
    if frame.row_multiindex().is_some() && orient != JsonOrient::Values {
        let materialized = materialize_synthetic_row_multiindex_columns(frame)?;
        return write_json_string(&materialized, orient);
    }

    let headers: Vec<String> = frame.column_names().into_iter().cloned().collect();
    let row_count = frame.index().len();

    match orient {
        JsonOrient::Records => {
            let column_float_promotions = headers
                .iter()
                .map(|name| {
                    frame.column(name).is_some_and(|column| {
                        column_promotes_int_json_values_to_float(column.values())
                    })
                })
                .collect::<Vec<_>>();
            let mut records = Vec::with_capacity(row_count);
            for row_idx in 0..row_count {
                let mut obj = serde_json::Map::new();
                for (name, promote_int_to_float) in
                    headers.iter().zip(column_float_promotions.iter())
                {
                    let val = frame
                        .column(name)
                        .and_then(|c| c.value(row_idx))
                        .map(|value| {
                            scalar_to_json_with_column_promotion(value, *promote_int_to_float)
                        })
                        .unwrap_or(serde_json::Value::Null);
                    obj.insert(name.clone(), val);
                }
                records.push(serde_json::Value::Object(obj));
            }
            Ok(serde_json::to_string(&records)?)
        }
        JsonOrient::Columns => {
            let mut outer = serde_json::Map::new();
            for name in &headers {
                let mut col_obj = serde_json::Map::new();
                if let Some(col) = frame.column(name) {
                    for (label, val) in frame.index().labels().iter().zip(col.values()) {
                        let key = label.to_string();
                        if col_obj.insert(key.clone(), scalar_to_json(val)).is_some() {
                            return Err(IoError::JsonFormat(format!(
                                "columns orient cannot encode duplicate index label key: {key}"
                            )));
                        }
                    }
                }
                outer.insert(name.clone(), serde_json::Value::Object(col_obj));
            }
            Ok(serde_json::to_string(&serde_json::Value::Object(outer))?)
        }
        JsonOrient::Index => {
            let mut outer = serde_json::Map::new();
            for row_idx in 0..row_count {
                let mut row_obj = serde_json::Map::new();
                for name in &headers {
                    let val = frame
                        .column(name)
                        .and_then(|c| c.value(row_idx))
                        .map(scalar_to_json)
                        .unwrap_or(serde_json::Value::Null);
                    row_obj.insert(name.clone(), val);
                }

                let row_label = frame.index().labels()[row_idx].to_string();
                if outer
                    .insert(row_label.clone(), serde_json::Value::Object(row_obj))
                    .is_some()
                {
                    return Err(IoError::JsonFormat(format!(
                        "index orient cannot encode duplicate index label key: {row_label}"
                    )));
                }
            }
            Ok(serde_json::to_string(&serde_json::Value::Object(outer))?)
        }
        JsonOrient::Split => {
            let col_array: Vec<serde_json::Value> = headers
                .iter()
                .map(|h| serde_json::Value::String(h.clone()))
                .collect();
            let index_array: Vec<serde_json::Value> = frame
                .index()
                .labels()
                .iter()
                .map(index_label_to_json)
                .collect();

            let mut data = Vec::with_capacity(row_count);
            for row_idx in 0..row_count {
                let row: Vec<serde_json::Value> = headers
                    .iter()
                    .map(|name| {
                        frame
                            .column(name)
                            .and_then(|c| c.value(row_idx))
                            .map(scalar_to_json)
                            .unwrap_or(serde_json::Value::Null)
                    })
                    .collect();
                data.push(serde_json::Value::Array(row));
            }

            let mut obj = serde_json::Map::new();
            obj.insert("columns".into(), serde_json::Value::Array(col_array));
            obj.insert("index".into(), serde_json::Value::Array(index_array));
            obj.insert("data".into(), serde_json::Value::Array(data));
            Ok(serde_json::to_string(&serde_json::Value::Object(obj))?)
        }
        JsonOrient::Values => {
            let mut data = Vec::with_capacity(row_count);
            for row_idx in 0..row_count {
                let row: Vec<serde_json::Value> = headers
                    .iter()
                    .map(|name| {
                        frame
                            .column(name)
                            .and_then(|c| c.value(row_idx))
                            .map(scalar_to_json)
                            .unwrap_or(serde_json::Value::Null)
                    })
                    .collect();
                data.push(serde_json::Value::Array(row));
            }
            Ok(serde_json::to_string(&serde_json::Value::Array(data))?)
        }
    }
}

// ── File-based JSON ────────────────────────────────────────────────────

pub fn read_json(path: &Path, orient: JsonOrient) -> Result<DataFrame, IoError> {
    let content = std::fs::read_to_string(path)?;
    read_json_str(&content, orient)
}

pub fn write_json(frame: &DataFrame, path: &Path, orient: JsonOrient) -> Result<(), IoError> {
    let content = write_json_string(frame, orient)?;
    std::fs::write(path, content)?;
    Ok(())
}

// ── JSONL (JSON Lines) I/O ──────────────────────────────────────────────

/// Write a DataFrame to JSONL (JSON Lines) format.
///
/// Matches `pd.DataFrame.to_json(orient='records', lines=True)`.
/// Each row is written as a separate JSON object on its own line,
/// with no enclosing array. This format is standard for streaming
/// data pipelines and log processing.
pub fn write_jsonl_string(frame: &DataFrame) -> Result<String, IoError> {
    let headers: Vec<String> = frame.column_names().into_iter().cloned().collect();
    let row_count = frame.index().len();

    let mut lines = Vec::with_capacity(row_count);
    for row_idx in 0..row_count {
        let mut obj = serde_json::Map::new();
        for name in &headers {
            let val = frame
                .column(name)
                .and_then(|c| c.value(row_idx))
                .map(scalar_to_json)
                .unwrap_or(serde_json::Value::Null);
            obj.insert(name.clone(), val);
        }
        lines.push(serde_json::to_string(&serde_json::Value::Object(obj))?);
    }

    Ok(lines.join("\n"))
}

/// Read a DataFrame from JSONL (JSON Lines) format.
///
/// Matches `pd.read_json(input, lines=True)`.
/// Each line must be a valid JSON object with the same keys.
pub fn read_jsonl_str(input: &str) -> Result<DataFrame, IoError> {
    let mut all_rows: Vec<serde_json::Map<String, serde_json::Value>> = Vec::new();

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed = parse_json_value_allowing_pandas_nan(trimmed)?;
        let obj = parsed
            .as_object()
            .ok_or_else(|| IoError::JsonFormat("JSONL: each line must be a JSON object".into()))?;
        all_rows.push(obj.clone());
    }

    if all_rows.is_empty() {
        return DataFrame::new(Index::new(Vec::new()), BTreeMap::new()).map_err(IoError::Frame);
    }

    // Collect column names as the UNION of all keys across all rows.
    // This matches pandas behavior: missing keys in a row become null.
    let mut col_name_set = std::collections::BTreeSet::new();
    let mut col_names_ordered: Vec<String> = Vec::new();
    for row in &all_rows {
        for key in row.keys() {
            if col_name_set.insert(key.clone()) {
                col_names_ordered.push(key.clone());
            }
        }
    }
    let col_names = col_names_ordered;
    let mut columns: Vec<Vec<Scalar>> = col_names
        .iter()
        .map(|_| Vec::with_capacity(all_rows.len()))
        .collect();

    for row in &all_rows {
        for (col_idx, name) in col_names.iter().enumerate() {
            let val = row.get(name).unwrap_or(&serde_json::Value::Null);
            columns[col_idx].push(json_value_to_scalar(val));
        }
    }

    let mut out_columns = BTreeMap::new();
    let mut column_order = Vec::new();
    for (name, values) in col_names.into_iter().zip(columns) {
        out_columns.insert(name.clone(), column_from_json_records_values(values)?);
        column_order.push(name);
    }

    let index = Index::from_i64((0..all_rows.len() as i64).collect());
    Ok(DataFrame::new_with_column_order(
        index,
        out_columns,
        column_order,
    )?)
}

/// Write a DataFrame to a JSONL file.
pub fn write_jsonl(frame: &DataFrame, path: &Path) -> Result<(), IoError> {
    let content = write_jsonl_string(frame)?;
    std::fs::write(path, content)?;
    Ok(())
}

/// Read a DataFrame from a JSONL file.
pub fn read_jsonl(path: &Path) -> Result<DataFrame, IoError> {
    let content = std::fs::read_to_string(path)?;
    read_jsonl_str(&content)
}

// ── Parquet I/O ─────────────────────────────────────────────────────────────

/// Convert an fp-types DType to an Arrow DataType.
fn dtype_to_arrow(dtype: DType) -> ArrowDataType {
    match dtype {
        DType::Int64 => ArrowDataType::Int64,
        DType::Float64 => ArrowDataType::Float64,
        DType::Utf8 => ArrowDataType::Utf8,
        DType::Categorical => ArrowDataType::Utf8,
        DType::Bool => ArrowDataType::Boolean,
        DType::Null => ArrowDataType::Utf8, // fallback: null-only columns as string
        DType::Timedelta64 => ArrowDataType::Int64, // store as nanoseconds
        DType::Sparse => ArrowDataType::Utf8, // marker fallback until sparse arrays land
    }
}

fn column_to_arrow_array(column: &Column) -> Result<Arc<dyn Array>, IoError> {
    let arr: Arc<dyn Array> = match column.dtype() {
        DType::Int64 => {
            let mut builder = Int64Builder::with_capacity(column.len());
            for value in column.values() {
                match value {
                    Scalar::Int64(n) => builder.append_value(*n),
                    _ if value.is_missing() => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DType::Float64 => {
            let mut builder = Float64Builder::with_capacity(column.len());
            for value in column.values() {
                match value {
                    Scalar::Float64(n) => {
                        if n.is_nan() {
                            builder.append_null();
                        } else {
                            builder.append_value(*n);
                        }
                    }
                    _ if value.is_missing() => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DType::Bool => {
            let mut builder = BooleanBuilder::with_capacity(column.len());
            for value in column.values() {
                match value {
                    Scalar::Bool(flag) => builder.append_value(*flag),
                    _ if value.is_missing() => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
        DType::Utf8 | DType::Categorical | DType::Null | DType::Sparse => {
            let mut builder = StringBuilder::with_capacity(column.len(), column.len() * 8);
            for value in column.values() {
                match value {
                    Scalar::Utf8(text) => builder.append_value(text),
                    _ if value.is_missing() => builder.append_null(),
                    _ => builder.append_value(format!("{value:?}")),
                }
            }
            Arc::new(builder.finish())
        }
        DType::Timedelta64 => {
            let mut builder = Int64Builder::with_capacity(column.len());
            for value in column.values() {
                match value {
                    Scalar::Timedelta64(nanos) => {
                        if *nanos == Timedelta::NAT {
                            builder.append_null();
                        } else {
                            builder.append_value(*nanos);
                        }
                    }
                    _ if value.is_missing() => builder.append_null(),
                    _ => builder.append_null(),
                }
            }
            Arc::new(builder.finish())
        }
    };

    Ok(arr)
}

/// Convert a Series to its Arrow data type plus backing array.
///
/// This is the Arrow-level building block under Feather / IPC round-trips and
/// preserves nullable Int64 columns as Arrow null-bitmaps rather than coercing
/// through Float64.
pub fn series_to_arrow_array(series: &Series) -> Result<(ArrowDataType, Arc<dyn Array>), IoError> {
    let dt = dtype_to_arrow(series.column().dtype());
    Ok((dt, column_to_arrow_array(series.column())?))
}

/// Rebuild a Series from an Arrow array and explicit dtype metadata.
pub fn series_from_arrow_array(
    name: impl Into<String>,
    index_labels: Vec<IndexLabel>,
    arr: &dyn Array,
    dt: &ArrowDataType,
) -> Result<Series, IoError> {
    let values = arrow_array_to_scalars(arr, dt)?;
    Series::from_values(name, index_labels, values).map_err(IoError::from)
}

/// Build an Arrow RecordBatch from a DataFrame.
fn dataframe_to_record_batch(frame: &DataFrame) -> Result<RecordBatch, IoError> {
    let materialized = if frame.row_multiindex().is_some() {
        Some(materialize_synthetic_row_multiindex_columns(frame)?)
    } else {
        None
    };
    let frame = materialized.as_ref().unwrap_or(frame);

    let col_names: Vec<String> = frame.column_names().into_iter().cloned().collect();
    let mut fields = Vec::with_capacity(col_names.len());
    let mut arrays: Vec<Arc<dyn Array>> = Vec::with_capacity(col_names.len());

    for name in &col_names {
        let col = frame
            .column(name)
            .ok_or_else(|| IoError::Parquet(format!("missing column: {name}")))?;
        let dt = col.dtype();
        fields.push(Field::new(name.as_str(), dtype_to_arrow(dt), true));
        let arr = column_to_arrow_array(col)?;
        arrays.push(arr);
    }

    let schema = Arc::new(Schema::new(fields));
    RecordBatch::try_new(schema, arrays).map_err(|e| IoError::Parquet(e.to_string()))
}

/// Convert an Arrow RecordBatch back into a DataFrame.
fn record_batch_to_dataframe(batch: &RecordBatch) -> Result<DataFrame, IoError> {
    let n_rows = batch.num_rows();
    let schema = batch.schema();
    let mut columns = BTreeMap::new();
    let mut col_order = Vec::new();

    for (i, field) in schema.fields().iter().enumerate() {
        let name = field.name().clone();
        let arr = batch.column(i);
        let values = arrow_array_to_scalars(arr.as_ref(), field.data_type())?;
        let dtype = fp_dtype_for_arrow_data_type(field.data_type());
        let col = Column::new(dtype, values)?;
        columns.insert(name.clone(), col);
        col_order.push(name);
    }

    let labels: Vec<IndexLabel> = (0..n_rows).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);

    let frame = DataFrame::new_with_column_order(index, columns, col_order)?;
    promote_synthetic_row_multiindex_if_present(&frame)
}

fn fp_dtype_for_arrow_data_type(dt: &ArrowDataType) -> DType {
    match dt {
        ArrowDataType::Int8
        | ArrowDataType::Int16
        | ArrowDataType::Int32
        | ArrowDataType::Int64
        | ArrowDataType::UInt8
        | ArrowDataType::UInt16
        | ArrowDataType::UInt32
        | ArrowDataType::UInt64 => DType::Int64,
        ArrowDataType::Float16 | ArrowDataType::Float32 | ArrowDataType::Float64 => DType::Float64,
        ArrowDataType::Boolean => DType::Bool,
        ArrowDataType::Utf8
        | ArrowDataType::LargeUtf8
        | ArrowDataType::Date32
        | ArrowDataType::Date64
        | ArrowDataType::Timestamp(_, _) => DType::Utf8,
        _ => DType::Utf8,
    }
}

/// Convert an Arrow array + data type to a Vec of Scalars.
fn arrow_array_to_scalars(arr: &dyn Array, dt: &ArrowDataType) -> Result<Vec<Scalar>, IoError> {
    let len = arr.len();
    let mut scalars = Vec::with_capacity(len);

    match dt {
        ArrowDataType::Int64 => {
            let typed = arr
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(|| IoError::Parquet("expected Int64Array".into()))?;
            for i in 0..len {
                if typed.is_null(i) {
                    scalars.push(Scalar::Null(NullKind::Null));
                } else {
                    scalars.push(Scalar::Int64(typed.value(i)));
                }
            }
        }
        ArrowDataType::Int32 => {
            let typed = arr
                .as_any()
                .downcast_ref::<arrow::array::Int32Array>()
                .ok_or_else(|| IoError::Parquet("expected Int32Array".into()))?;
            for i in 0..len {
                if typed.is_null(i) {
                    scalars.push(Scalar::Null(NullKind::Null));
                } else {
                    scalars.push(Scalar::Int64(i64::from(typed.value(i))));
                }
            }
        }
        ArrowDataType::Float64 => {
            let typed = arr
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| IoError::Parquet("expected Float64Array".into()))?;
            for i in 0..len {
                if typed.is_null(i) {
                    scalars.push(Scalar::Null(NullKind::NaN));
                } else {
                    scalars.push(Scalar::Float64(typed.value(i)));
                }
            }
        }
        ArrowDataType::Float32 => {
            let typed = arr
                .as_any()
                .downcast_ref::<arrow::array::Float32Array>()
                .ok_or_else(|| IoError::Parquet("expected Float32Array".into()))?;
            for i in 0..len {
                if typed.is_null(i) {
                    scalars.push(Scalar::Null(NullKind::NaN));
                } else {
                    scalars.push(Scalar::Float64(f64::from(typed.value(i))));
                }
            }
        }
        ArrowDataType::Boolean => {
            let typed = arr
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(|| IoError::Parquet("expected BooleanArray".into()))?;
            for i in 0..len {
                if typed.is_null(i) {
                    scalars.push(Scalar::Null(NullKind::Null));
                } else {
                    scalars.push(Scalar::Bool(typed.value(i)));
                }
            }
        }
        ArrowDataType::Utf8 => {
            let typed = arr
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| IoError::Parquet("expected StringArray".into()))?;
            for i in 0..len {
                if typed.is_null(i) {
                    scalars.push(Scalar::Null(NullKind::Null));
                } else {
                    scalars.push(Scalar::Utf8(typed.value(i).to_owned()));
                }
            }
        }
        ArrowDataType::LargeUtf8 => {
            let typed = arr
                .as_any()
                .downcast_ref::<arrow::array::LargeStringArray>()
                .ok_or_else(|| IoError::Parquet("expected LargeStringArray".into()))?;
            for i in 0..len {
                if typed.is_null(i) {
                    scalars.push(Scalar::Null(NullKind::Null));
                } else {
                    scalars.push(Scalar::Utf8(typed.value(i).to_owned()));
                }
            }
        }
        ArrowDataType::Date32 => {
            let typed = arr
                .as_any()
                .downcast_ref::<Date32Array>()
                .ok_or_else(|| IoError::Parquet("expected Date32Array".into()))?;
            for i in 0..len {
                if typed.is_null(i) {
                    scalars.push(Scalar::Null(NullKind::NaT));
                } else {
                    if let Some(date) = arrow::temporal_conversions::as_date::<
                        arrow::datatypes::Date32Type,
                    >(typed.value(i).into())
                    {
                        scalars.push(Scalar::Utf8(date.format("%Y-%m-%d").to_string()));
                    } else {
                        scalars.push(Scalar::Null(NullKind::NaT));
                    }
                }
            }
        }
        ArrowDataType::Date64 => {
            let typed = arr
                .as_any()
                .downcast_ref::<Date64Array>()
                .ok_or_else(|| IoError::Parquet("expected Date64Array".into()))?;
            for i in 0..len {
                if typed.is_null(i) {
                    scalars.push(Scalar::Null(NullKind::NaT));
                } else {
                    if let Some(dt) = arrow::temporal_conversions::as_datetime::<
                        arrow::datatypes::Date64Type,
                    >(typed.value(i))
                    {
                        scalars.push(Scalar::Utf8(dt.format("%Y-%m-%d").to_string()));
                    } else {
                        scalars.push(Scalar::Null(NullKind::NaT));
                    }
                }
            }
        }
        ArrowDataType::Timestamp(unit, _tz) => match unit {
            TimeUnit::Second => {
                let typed = arr
                    .as_any()
                    .downcast_ref::<TimestampSecondArray>()
                    .ok_or_else(|| IoError::Parquet("expected TimestampSecondArray".into()))?;
                for i in 0..len {
                    if typed.is_null(i) {
                        scalars.push(Scalar::Null(NullKind::NaT));
                    } else {
                        if let Some(dt) = arrow::temporal_conversions::as_datetime::<
                            arrow::datatypes::TimestampSecondType,
                        >(typed.value(i))
                        {
                            scalars.push(Scalar::Utf8(dt.format("%Y-%m-%d %H:%M:%S").to_string()));
                        } else {
                            scalars.push(Scalar::Null(NullKind::NaT));
                        }
                    }
                }
            }
            TimeUnit::Millisecond => {
                let typed = arr
                    .as_any()
                    .downcast_ref::<TimestampMillisecondArray>()
                    .ok_or_else(|| IoError::Parquet("expected TimestampMillisecondArray".into()))?;
                for i in 0..len {
                    if typed.is_null(i) {
                        scalars.push(Scalar::Null(NullKind::NaT));
                    } else {
                        if let Some(dt) = arrow::temporal_conversions::as_datetime::<
                            arrow::datatypes::TimestampMillisecondType,
                        >(typed.value(i))
                        {
                            scalars.push(Scalar::Utf8(dt.format("%Y-%m-%d %H:%M:%S").to_string()));
                        } else {
                            scalars.push(Scalar::Null(NullKind::NaT));
                        }
                    }
                }
            }
            TimeUnit::Microsecond => {
                let typed = arr
                    .as_any()
                    .downcast_ref::<TimestampMicrosecondArray>()
                    .ok_or_else(|| IoError::Parquet("expected TimestampMicrosecondArray".into()))?;
                for i in 0..len {
                    if typed.is_null(i) {
                        scalars.push(Scalar::Null(NullKind::NaT));
                    } else {
                        if let Some(dt) = arrow::temporal_conversions::as_datetime::<
                            arrow::datatypes::TimestampMicrosecondType,
                        >(typed.value(i))
                        {
                            scalars
                                .push(Scalar::Utf8(dt.format("%Y-%m-%d %H:%M:%S%.6f").to_string()));
                        } else {
                            scalars.push(Scalar::Null(NullKind::NaT));
                        }
                    }
                }
            }
            TimeUnit::Nanosecond => {
                let typed = arr
                    .as_any()
                    .downcast_ref::<TimestampNanosecondArray>()
                    .ok_or_else(|| IoError::Parquet("expected TimestampNanosecondArray".into()))?;
                for i in 0..len {
                    if typed.is_null(i) {
                        scalars.push(Scalar::Null(NullKind::NaT));
                    } else {
                        if let Some(dt) = arrow::temporal_conversions::as_datetime::<
                            arrow::datatypes::TimestampNanosecondType,
                        >(typed.value(i))
                        {
                            scalars
                                .push(Scalar::Utf8(dt.format("%Y-%m-%d %H:%M:%S%.9f").to_string()));
                        } else {
                            scalars.push(Scalar::Null(NullKind::NaT));
                        }
                    }
                }
            }
        },
        other => {
            return Err(IoError::Parquet(format!(
                "unsupported Arrow data type: {other:?}"
            )));
        }
    }

    Ok(scalars)
}

/// Write a DataFrame to an in-memory Parquet buffer.
pub fn write_parquet_bytes(frame: &DataFrame) -> Result<Vec<u8>, IoError> {
    let batch = dataframe_to_record_batch(frame)?;
    let mut buf = Vec::new();
    let mut writer = ArrowWriter::try_new(&mut buf, batch.schema(), None)
        .map_err(|e| IoError::Parquet(e.to_string()))?;
    writer
        .write(&batch)
        .map_err(|e| IoError::Parquet(e.to_string()))?;
    writer
        .close()
        .map_err(|e| IoError::Parquet(e.to_string()))?;
    Ok(buf)
}

/// Read a DataFrame from in-memory Parquet bytes.
pub fn read_parquet_bytes(data: &[u8]) -> Result<DataFrame, IoError> {
    let b = bytes::Bytes::from(data.to_vec());
    let reader = ParquetRecordBatchReaderBuilder::try_new(b)
        .map_err(|e| IoError::Parquet(e.to_string()))?
        .build()
        .map_err(|e| IoError::Parquet(e.to_string()))?;

    let mut all_frames: Vec<DataFrame> = Vec::new();
    for batch_result in reader {
        let batch: RecordBatch =
            batch_result.map_err(|e: arrow::error::ArrowError| IoError::Parquet(e.to_string()))?;
        all_frames.push(record_batch_to_dataframe(&batch)?);
    }

    if all_frames.is_empty() {
        // Return empty DataFrame
        return Ok(DataFrame::new_with_column_order(
            Index::new(vec![]),
            BTreeMap::new(),
            vec![],
        )?);
    }

    // For a single batch (common case), return directly
    if all_frames.len() == 1 {
        if let Some(frame) = all_frames.into_iter().next() {
            return Ok(frame);
        }
        return Err(IoError::Parquet(
            "parquet reader produced zero record batches".to_owned(),
        ));
    }

    // Multiple batches: concatenate via fp_frame::concat_dataframes
    let refs: Vec<&DataFrame> = all_frames.iter().collect();
    fp_frame::concat_dataframes(&refs).map_err(IoError::from)
}

/// Write a DataFrame to a Parquet file.
pub fn write_parquet(frame: &DataFrame, path: &Path) -> Result<(), IoError> {
    let bytes = write_parquet_bytes(frame)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Read a DataFrame from a Parquet file.
pub fn read_parquet(path: &Path) -> Result<DataFrame, IoError> {
    let data = std::fs::read(path)?;
    read_parquet_bytes(&data)
}

// ── Excel (xlsx) I/O ────────────────────────────────────────────────────

/// Options for reading Excel files.
#[derive(Debug, Clone)]
pub struct ExcelReadOptions {
    /// Sheet name to read. If `None`, reads the first sheet.
    pub sheet_name: Option<String>,
    /// Whether the first row contains column headers.
    pub has_headers: bool,
    /// Read only these columns (by name). `None` means read all.
    /// Matches pandas `usecols` parameter for label-based selection.
    pub usecols: Option<Vec<String>>,
    /// Explicit column names to use instead of worksheet headers or
    /// auto-generated `column_N` names. Matches pandas `names=...`.
    pub names: Option<Vec<String>>,
    /// Optional column to use as the DataFrame index.
    pub index_col: Option<String>,
    /// Number of initial rows to skip before reading headers/data.
    pub skip_rows: usize,
}

impl Default for ExcelReadOptions {
    fn default() -> Self {
        Self {
            sheet_name: None,
            has_headers: true,
            usecols: None,
            names: None,
            index_col: None,
            skip_rows: 0,
        }
    }
}

/// Convert a calamine `Data` cell value to a `Scalar`.
fn excel_cell_to_scalar(cell: &calamine::Data) -> Scalar {
    match cell {
        calamine::Data::Int(v) => Scalar::Int64(*v),
        calamine::Data::Float(v) => {
            if v.is_nan() {
                Scalar::Null(NullKind::NaN)
            } else if v.fract() == 0.0 && *v >= i64::MIN as f64 && *v <= i64::MAX as f64 {
                // Excel stores integers as floats; recover Int64 for whole numbers.
                Scalar::Int64(*v as i64)
            } else {
                Scalar::Float64(*v)
            }
        }
        calamine::Data::String(s) => {
            if s.is_empty() {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Utf8(s.clone())
            }
        }
        calamine::Data::Bool(b) => Scalar::Bool(*b),
        calamine::Data::Empty => Scalar::Null(NullKind::Null),
        calamine::Data::DateTime(dt) => {
            // Convert ExcelDateTime to string representation for now.
            Scalar::Utf8(format!("{dt}"))
        }
        calamine::Data::DateTimeIso(s) => Scalar::Utf8(s.clone()),
        calamine::Data::DurationIso(s) => Scalar::Utf8(s.clone()),
        calamine::Data::Error(e) => Scalar::Utf8(format!("#ERROR:{e:?}")),
    }
}

/// Convert a Scalar to an IndexLabel, handling float precision correctly.
fn scalar_to_index_label(scalar: Scalar) -> IndexLabel {
    match scalar {
        Scalar::Int64(v) => IndexLabel::Int64(v),
        Scalar::Utf8(s) => IndexLabel::Utf8(s),
        Scalar::Float64(v) if v.fract() == 0.0 && v >= i64::MIN as f64 && v <= i64::MAX as f64 => {
            IndexLabel::Int64(v as i64)
        }
        Scalar::Float64(v) => IndexLabel::Utf8(v.to_string()),
        Scalar::Bool(b) => IndexLabel::Utf8(b.to_string()),
        _ => IndexLabel::Utf8(String::new()),
    }
}

fn infer_writer_emitted_default_excel_index_col(
    headers: &[String],
    header_generated: &[bool],
    columns: &[Vec<Scalar>],
    options: &ExcelReadOptions,
) -> Option<usize> {
    if !options.has_headers
        || options.index_col.is_some()
        || options.usecols.is_some()
        || options.names.is_some()
    {
        return None;
    }

    if headers.first()?.as_str() != "column_0"
        || !header_generated.first().copied().unwrap_or(false)
    {
        return None;
    }

    let first_col = columns.first()?;
    if first_col
        .iter()
        .enumerate()
        .all(|(idx, scalar)| matches!(scalar, Scalar::Int64(value) if *value == idx as i64))
    {
        Some(0)
    } else {
        None
    }
}

/// Shared parsing logic for Excel data after extracting rows from a workbook.
fn parse_excel_rows(
    rows: Vec<Vec<calamine::Data>>,
    options: &ExcelReadOptions,
) -> Result<DataFrame, IoError> {
    if rows.is_empty() {
        return DataFrame::new(Index::new(Vec::new()), BTreeMap::new()).map_err(IoError::Frame);
    }

    let resolve_names = |width: usize| -> Result<Option<Vec<String>>, IoError> {
        options.names.as_ref().map_or(Ok(None), |names| {
            if names.len() == width {
                Ok(Some(names.clone()))
            } else {
                Err(IoError::Excel(format!(
                    "expected {width} column names, got {}",
                    names.len()
                )))
            }
        })
    };

    // Extract headers.
    let (headers, header_generated, data_rows) = if options.has_headers {
        let header_row = &rows[0];
        let header_width = header_row.len();
        let provided_names = resolve_names(header_width)?;
        let (headers, header_generated): (Vec<_>, Vec<_>) = if let Some(names) = provided_names {
            (names, vec![false; header_width])
        } else {
            let header_pairs: Vec<(String, bool)> = header_row
                .iter()
                .enumerate()
                .map(|(i, cell)| match cell {
                    calamine::Data::String(s) if !s.is_empty() => (s.clone(), false),
                    _ => (format!("column_{i}"), true),
                })
                .collect();
            header_pairs.into_iter().unzip()
        };
        (headers, header_generated, &rows[1..])
    } else {
        let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
        let provided_names = resolve_names(ncols)?;
        let (headers, header_generated) = if let Some(names) = provided_names {
            (names, vec![false; ncols])
        } else {
            let headers: Vec<String> = (0..ncols).map(|i| format!("column_{i}")).collect();
            let header_generated = vec![true; ncols];
            (headers, header_generated)
        };
        (headers, header_generated, rows.as_slice())
    };
    reject_duplicate_headers(&headers)?;

    if let Some(ref usecols) = options.usecols {
        validate_usecols(&headers, usecols)?;
    }

    let ncols = headers.len();

    // Accumulate columns.
    let mut columns: Vec<Vec<Scalar>> = (0..ncols)
        .map(|_| Vec::with_capacity(data_rows.len()))
        .collect();

    for row in data_rows {
        for (col_idx, col_vec) in columns.iter_mut().enumerate() {
            let cell = row.get(col_idx).unwrap_or(&calamine::Data::Empty);
            col_vec.push(excel_cell_to_scalar(cell));
        }
    }

    let (headers, header_generated, columns) = if let Some(ref usecols) = options.usecols {
        let mut filtered_headers = Vec::new();
        let mut filtered_generated = Vec::new();
        let mut filtered_columns = Vec::new();
        for ((name, generated), values) in headers.into_iter().zip(header_generated).zip(columns) {
            if usecols.contains(&name) {
                filtered_headers.push(name);
                filtered_generated.push(generated);
                filtered_columns.push(values);
            }
        }
        (filtered_headers, filtered_generated, filtered_columns)
    } else {
        (headers, header_generated, columns)
    };

    // Handle index_col if specified.
    let index_col_idx = if let Some(ref idx_name) = options.index_col {
        let pos = headers.iter().position(|h| h == idx_name);
        if pos.is_none() {
            return Err(IoError::MissingIndexColumn(idx_name.clone()));
        }
        pos
    } else {
        infer_writer_emitted_default_excel_index_col(&headers, &header_generated, &columns, options)
    };

    let index_name = index_col_idx.and_then(|idx_pos| {
        if !header_generated[idx_pos] {
            Some(headers[idx_pos].clone())
        } else {
            None
        }
    });

    let mut out_columns = BTreeMap::new();
    let mut column_order = Vec::new();

    for (idx, (name, values)) in headers.into_iter().zip(columns).enumerate() {
        if Some(idx) == index_col_idx {
            continue; // skip index column from data columns
        }
        out_columns.insert(name.clone(), Column::from_values(values)?);
        column_order.push(name);
    }

    let index = if let Some(idx_pos) = index_col_idx {
        let idx_labels: Vec<IndexLabel> = data_rows
            .iter()
            .map(|row| {
                let cell = row.get(idx_pos).unwrap_or(&calamine::Data::Empty);
                scalar_to_index_label(excel_cell_to_scalar(cell))
            })
            .collect();
        Index::new(idx_labels).set_names(index_name.as_deref())
    } else {
        Index::from_i64((0..data_rows.len() as i64).collect())
    };

    Ok(DataFrame::new_with_column_order(
        index,
        out_columns,
        column_order,
    )?)
}

/// Read an Excel (.xlsx/.xls/.xlsb/.ods) file into a DataFrame.
///
/// Matches `pd.read_excel(path)` for basic usage.
pub fn read_excel(path: &Path, options: &ExcelReadOptions) -> Result<DataFrame, IoError> {
    use calamine::{Reader, open_workbook_auto};

    let mut workbook = open_workbook_auto(path)
        .map_err(|e| IoError::Excel(format!("cannot open workbook: {e}")))?;

    let sheet_name = if let Some(ref name) = options.sheet_name {
        name.clone()
    } else {
        let names = workbook.sheet_names();
        if names.is_empty() {
            return Err(IoError::Excel("workbook contains no sheets".into()));
        }
        names[0].clone()
    };

    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|e| IoError::Excel(format!("cannot read sheet '{sheet_name}': {e}")))?;

    let rows: Vec<Vec<calamine::Data>> = range
        .rows()
        .skip(options.skip_rows)
        .map(|r| r.to_vec())
        .collect();

    parse_excel_rows(rows, options)
}

pub fn read_excel_with_index_cols(
    path: &Path,
    options: &ExcelReadOptions,
    index_cols: &[&str],
) -> Result<DataFrame, IoError> {
    let frame = read_excel(path, options)?;
    promote_frame_index_columns(&frame, index_cols)
}

/// Read Excel from in-memory bytes.
pub fn read_excel_bytes(data: &[u8], options: &ExcelReadOptions) -> Result<DataFrame, IoError> {
    use calamine::{Reader, open_workbook_auto_from_rs};

    let cursor = std::io::Cursor::new(data);
    let mut workbook = open_workbook_auto_from_rs(cursor)
        .map_err(|e| IoError::Excel(format!("cannot open workbook from bytes: {e}")))?;

    let sheet_name = if let Some(ref name) = options.sheet_name {
        name.clone()
    } else {
        let names = workbook.sheet_names();
        if names.is_empty() {
            return Err(IoError::Excel("workbook contains no sheets".into()));
        }
        names[0].clone()
    };

    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|e| IoError::Excel(format!("cannot read sheet '{sheet_name}': {e}")))?;

    let rows: Vec<Vec<calamine::Data>> = range
        .rows()
        .skip(options.skip_rows)
        .map(|r| r.to_vec())
        .collect();

    parse_excel_rows(rows, options)
}

pub fn read_excel_bytes_with_index_cols(
    data: &[u8],
    options: &ExcelReadOptions,
    index_cols: &[&str],
) -> Result<DataFrame, IoError> {
    let frame = read_excel_bytes(data, options)?;
    promote_frame_index_columns(&frame, index_cols)
}

/// Read multiple sheets from an Excel file.
///
/// Matches `pd.read_excel(path, sheet_name=[...])` when `sheet_name`
/// is a list of sheet names — pandas returns a dict
/// `{name: DataFrame}`. Pass `sheet_names=None` to read every sheet
/// in the workbook (pandas `sheet_name=None`).
///
/// The outer Excel reader options (`has_headers`, `index_col`,
/// `skip_rows`) are applied uniformly to each selected sheet. The
/// per-sheet `sheet_name` option on `options` is ignored here
/// because the explicit `sheet_names` argument drives selection.
/// Read multiple sheets preserving workbook iteration order.
///
/// Matches `pd.read_excel(sheet_name=None)` exactly — pandas returns
/// a `dict` and, since Python 3.7, dict iteration order matches
/// insertion order, which in turn matches workbook sheet position.
/// `BTreeMap` (used by `read_excel_sheets`) would alphabetize, so
/// this sibling returns `Vec<(String, DataFrame)>` to preserve order.
pub fn read_excel_sheets_ordered(
    path: &Path,
    sheet_names: Option<&[String]>,
    options: &ExcelReadOptions,
) -> Result<Vec<(String, DataFrame)>, IoError> {
    use calamine::{Reader, open_workbook_auto};

    let mut workbook = open_workbook_auto(path)
        .map_err(|e| IoError::Excel(format!("cannot open workbook: {e}")))?;
    let available: Vec<String> = workbook.sheet_names();
    let selected: Vec<String> = match sheet_names {
        Some(names) => {
            for name in names {
                if !available.iter().any(|s| s == name) {
                    return Err(IoError::Excel(format!(
                        "workbook does not contain sheet {name:?}"
                    )));
                }
            }
            names.to_vec()
        }
        None => available.clone(),
    };
    if selected.is_empty() {
        return Err(IoError::Excel("no sheets selected".to_owned()));
    }
    let mut out = Vec::with_capacity(selected.len());
    for sheet in &selected {
        let range = workbook
            .worksheet_range(sheet)
            .map_err(|e| IoError::Excel(format!("cannot read sheet {sheet:?}: {e}")))?;
        let rows: Vec<Vec<calamine::Data>> = range
            .rows()
            .skip(options.skip_rows)
            .map(|r| r.to_vec())
            .collect();
        let frame = parse_excel_rows(rows, options)?;
        out.push((sheet.clone(), frame));
    }
    Ok(out)
}

/// Byte-based counterpart to `read_excel_sheets_ordered`.
pub fn read_excel_sheets_ordered_bytes(
    data: &[u8],
    sheet_names: Option<&[String]>,
    options: &ExcelReadOptions,
) -> Result<Vec<(String, DataFrame)>, IoError> {
    use calamine::{Reader, open_workbook_auto_from_rs};

    let cursor = std::io::Cursor::new(data);
    let mut workbook = open_workbook_auto_from_rs(cursor)
        .map_err(|e| IoError::Excel(format!("cannot open workbook from bytes: {e}")))?;
    let available: Vec<String> = workbook.sheet_names();
    let selected: Vec<String> = match sheet_names {
        Some(names) => {
            for name in names {
                if !available.iter().any(|s| s == name) {
                    return Err(IoError::Excel(format!(
                        "workbook does not contain sheet {name:?}"
                    )));
                }
            }
            names.to_vec()
        }
        None => available.clone(),
    };
    if selected.is_empty() {
        return Err(IoError::Excel("no sheets selected".to_owned()));
    }
    let mut out = Vec::with_capacity(selected.len());
    for sheet in &selected {
        let range = workbook
            .worksheet_range(sheet)
            .map_err(|e| IoError::Excel(format!("cannot read sheet {sheet:?}: {e}")))?;
        let rows: Vec<Vec<calamine::Data>> = range
            .rows()
            .skip(options.skip_rows)
            .map(|r| r.to_vec())
            .collect();
        let frame = parse_excel_rows(rows, options)?;
        out.push((sheet.clone(), frame));
    }
    Ok(out)
}

pub fn read_excel_sheets(
    path: &Path,
    sheet_names: Option<&[String]>,
    options: &ExcelReadOptions,
) -> Result<BTreeMap<String, DataFrame>, IoError> {
    use calamine::{Reader, open_workbook_auto};

    let mut workbook = open_workbook_auto(path)
        .map_err(|e| IoError::Excel(format!("cannot open workbook: {e}")))?;
    let available: Vec<String> = workbook.sheet_names();
    let selected: Vec<String> = match sheet_names {
        Some(names) => {
            for name in names {
                if !available.iter().any(|s| s == name) {
                    return Err(IoError::Excel(format!(
                        "workbook does not contain sheet {name:?}"
                    )));
                }
            }
            names.to_vec()
        }
        None => available.clone(),
    };
    if selected.is_empty() {
        return Err(IoError::Excel("no sheets selected".to_owned()));
    }

    let mut out = BTreeMap::new();
    for sheet in &selected {
        let range = workbook
            .worksheet_range(sheet)
            .map_err(|e| IoError::Excel(format!("cannot read sheet {sheet:?}: {e}")))?;
        let rows: Vec<Vec<calamine::Data>> = range
            .rows()
            .skip(options.skip_rows)
            .map(|r| r.to_vec())
            .collect();
        let frame = parse_excel_rows(rows, options)?;
        out.insert(sheet.clone(), frame);
    }
    Ok(out)
}

/// Read multiple sheets from Excel bytes.
///
/// Byte-based counterpart to `read_excel_sheets`.
pub fn read_excel_sheets_bytes(
    data: &[u8],
    sheet_names: Option<&[String]>,
    options: &ExcelReadOptions,
) -> Result<BTreeMap<String, DataFrame>, IoError> {
    use calamine::{Reader, open_workbook_auto_from_rs};

    let cursor = std::io::Cursor::new(data);
    let mut workbook = open_workbook_auto_from_rs(cursor)
        .map_err(|e| IoError::Excel(format!("cannot open workbook from bytes: {e}")))?;
    let available: Vec<String> = workbook.sheet_names();
    let selected: Vec<String> = match sheet_names {
        Some(names) => {
            for name in names {
                if !available.iter().any(|s| s == name) {
                    return Err(IoError::Excel(format!(
                        "workbook does not contain sheet {name:?}"
                    )));
                }
            }
            names.to_vec()
        }
        None => available.clone(),
    };
    if selected.is_empty() {
        return Err(IoError::Excel("no sheets selected".to_owned()));
    }

    let mut out = BTreeMap::new();
    for sheet in &selected {
        let range = workbook
            .worksheet_range(sheet)
            .map_err(|e| IoError::Excel(format!("cannot read sheet {sheet:?}: {e}")))?;
        let rows: Vec<Vec<calamine::Data>> = range
            .rows()
            .skip(options.skip_rows)
            .map(|r| r.to_vec())
            .collect();
        let frame = parse_excel_rows(rows, options)?;
        out.insert(sheet.clone(), frame);
    }
    Ok(out)
}

/// Write a DataFrame to an Excel (.xlsx) file.
///
/// Matches `pd.DataFrame.to_excel(path)`.
pub fn write_excel(frame: &DataFrame, path: &Path) -> Result<(), IoError> {
    let bytes = write_excel_bytes(frame)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn write_excel_index_label(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    excel_row: u32,
    excel_col: u16,
    label: &IndexLabel,
) -> Result<(), IoError> {
    match label {
        IndexLabel::Int64(v) => {
            worksheet
                .write_number(excel_row, excel_col, *v as f64)
                .map_err(|e| IoError::Excel(format!("write index int: {e}")))?;
        }
        IndexLabel::Utf8(s) => {
            worksheet
                .write_string(excel_row, excel_col, s.as_str())
                .map_err(|e| IoError::Excel(format!("write index string: {e}")))?;
        }
        IndexLabel::Timedelta64(v) => {
            if *v != Timedelta::NAT {
                worksheet
                    .write_string(excel_row, excel_col, Timedelta::format(*v))
                    .map_err(|e| IoError::Excel(format!("write index timedelta: {e}")))?;
            }
        }
        IndexLabel::Datetime64(v) => {
            if *v != i64::MIN {
                worksheet
                    .write_string(excel_row, excel_col, label.to_string())
                    .map_err(|e| IoError::Excel(format!("write index datetime: {e}")))?;
            }
        }
    }
    Ok(())
}

fn write_excel_scalar(
    worksheet: &mut rust_xlsxwriter::Worksheet,
    excel_row: u32,
    excel_col: u16,
    scalar: &Scalar,
) -> Result<(), IoError> {
    match scalar {
        Scalar::Int64(v) => {
            worksheet
                .write_number(excel_row, excel_col, *v as f64)
                .map_err(|e| IoError::Excel(format!("write int: {e}")))?;
        }
        Scalar::Float64(v) if !v.is_nan() => {
            worksheet
                .write_number(excel_row, excel_col, *v)
                .map_err(|e| IoError::Excel(format!("write float: {e}")))?;
        }
        Scalar::Bool(b) => {
            worksheet
                .write_boolean(excel_row, excel_col, *b)
                .map_err(|e| IoError::Excel(format!("write bool: {e}")))?;
        }
        Scalar::Utf8(s) => {
            worksheet
                .write_string(excel_row, excel_col, s.as_str())
                .map_err(|e| IoError::Excel(format!("write string: {e}")))?;
        }
        Scalar::Timedelta64(v) => {
            if *v != Timedelta::NAT {
                worksheet
                    .write_string(excel_row, excel_col, Timedelta::format(*v))
                    .map_err(|e| IoError::Excel(format!("write timedelta: {e}")))?;
            }
        }
        Scalar::Float64(_) | Scalar::Null(_) => {}
    }
    Ok(())
}

/// Write a DataFrame to Excel (.xlsx) bytes in memory.
///
/// Matches pandas `DataFrame.to_excel()` default index behavior by emitting
/// the index as the first worksheet column.
pub fn write_excel_bytes(frame: &DataFrame) -> Result<Vec<u8>, IoError> {
    write_excel_bytes_with_options(frame, &ExcelWriteOptions::default())
}

/// Options for serializing a DataFrame to Excel.
///
/// Mirrors the subset of `pd.DataFrame.to_excel` parameters that
/// don't depend on the workbook writer itself.
#[derive(Debug, Clone)]
pub struct ExcelWriteOptions {
    /// Target sheet name. Default: `"Sheet1"` (rust_xlsxwriter default).
    pub sheet_name: String,
    /// Whether to write the row index as a leading column. Matches
    /// pandas `index=True|False`. Default: true.
    pub index: bool,
    /// Header label for the index column when `index=true`. When
    /// `None`, the frame's index name is used (falling back to an
    /// empty string). Matches pandas `index_label=...`.
    pub index_label: Option<String>,
    /// Whether to emit the column-name header row. Matches pandas
    /// `header=True|False`. Default: true.
    pub header: bool,
}

impl Default for ExcelWriteOptions {
    fn default() -> Self {
        Self {
            sheet_name: "Sheet1".to_string(),
            index: true,
            index_label: None,
            header: true,
        }
    }
}

/// Serialize a DataFrame to Excel bytes with explicit options.
///
/// Matches `pd.DataFrame.to_excel(sheet_name, index, index_label,
/// header)` for the in-memory byte form. The default `ExcelWriteOptions`
/// reproduces the existing `write_excel_bytes` behavior (index=true,
/// sheet_name="Sheet1").
pub fn write_excel_bytes_with_options(
    frame: &DataFrame,
    options: &ExcelWriteOptions,
) -> Result<Vec<u8>, IoError> {
    if options.index && frame.row_multiindex().is_some() {
        let materialized = materialize_named_row_multiindex_columns(frame)?;
        let mut nested_options = options.clone();
        nested_options.index = false;
        nested_options.index_label = None;
        return write_excel_bytes_with_options(&materialized, &nested_options);
    }

    use rust_xlsxwriter::Workbook;

    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet
        .set_name(options.sheet_name.as_str())
        .map_err(|e| IoError::Excel(format!("set sheet name: {e}")))?;

    let col_names: Vec<String> = frame.column_names().into_iter().cloned().collect();
    let data_col_offset: u16 = if options.index { 1 } else { 0 };

    // Header row (optional).
    if options.header {
        if options.index {
            let idx_header = options
                .index_label
                .as_deref()
                .unwrap_or_else(|| frame.index().name().unwrap_or(""));
            worksheet
                .write_string(0, 0, idx_header)
                .map_err(|e| IoError::Excel(format!("write index header: {e}")))?;
        }
        for (col_idx, name) in col_names.iter().enumerate() {
            worksheet
                .write_string(0, data_col_offset + col_idx as u16, name.as_str())
                .map_err(|e| IoError::Excel(format!("write header: {e}")))?;
        }
    }

    // Data rows — when header=true the first data row lands at excel
    // row 1; when header=false data starts at row 0.
    let header_rows: u32 = if options.header { 1 } else { 0 };
    let nrows = frame.index().len();
    for row_idx in 0..nrows {
        let excel_row = row_idx as u32 + header_rows;
        if options.index
            && let Some(label) = frame.index().labels().get(row_idx)
        {
            write_excel_index_label(worksheet, excel_row, 0, label)?;
        }
        for (col_idx, name) in col_names.iter().enumerate() {
            if let Some(col) = frame.column(name)
                && let Some(scalar) = col.value(row_idx)
            {
                write_excel_scalar(
                    worksheet,
                    excel_row,
                    data_col_offset + col_idx as u16,
                    scalar,
                )?;
            }
        }
    }

    let buf = workbook
        .save_to_buffer()
        .map_err(|e| IoError::Excel(format!("save workbook: {e}")))?;

    Ok(buf)
}

/// File-based counterpart to `write_excel_bytes_with_options`.
pub fn write_excel_with_options(
    frame: &DataFrame,
    path: &Path,
    options: &ExcelWriteOptions,
) -> Result<(), IoError> {
    let bytes = write_excel_bytes_with_options(frame, options)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

// ── Arrow IPC / Feather I/O ──────────────────────────────────────────────

/// Write a DataFrame to Arrow IPC (Feather v2) bytes in memory.
///
/// Matches `pd.DataFrame.to_feather()`. Feather v2 is the Arrow IPC file format
/// — the fastest columnar interchange format, recommended by pandas over HDF5.
pub fn write_feather_bytes(frame: &DataFrame) -> Result<Vec<u8>, IoError> {
    use arrow::ipc::writer::FileWriter;

    let batch = dataframe_to_record_batch(frame)?;
    let schema = batch.schema();

    let mut buf = Vec::new();
    let mut writer =
        FileWriter::try_new(&mut buf, &schema).map_err(|e| IoError::Arrow(e.to_string()))?;
    writer
        .write(&batch)
        .map_err(|e| IoError::Arrow(e.to_string()))?;
    writer.finish().map_err(|e| IoError::Arrow(e.to_string()))?;
    Ok(buf)
}

/// Read a DataFrame from Arrow IPC (Feather v2) bytes in memory.
///
/// Matches `pd.read_feather()`.
pub fn read_feather_bytes(data: &[u8]) -> Result<DataFrame, IoError> {
    use arrow::ipc::reader::FileReader;

    let cursor = std::io::Cursor::new(data);
    let reader = FileReader::try_new(cursor, None).map_err(|e| IoError::Arrow(e.to_string()))?;

    let mut all_frames: Vec<DataFrame> = Vec::new();
    for batch_result in reader {
        let batch = batch_result.map_err(|e| IoError::Arrow(e.to_string()))?;
        all_frames.push(record_batch_to_dataframe(&batch)?);
    }

    if all_frames.is_empty() {
        return Ok(DataFrame::new_with_column_order(
            Index::new(vec![]),
            BTreeMap::new(),
            vec![],
        )?);
    }

    if all_frames.len() == 1 {
        if let Some(frame) = all_frames.into_iter().next() {
            return Ok(frame);
        }
        return Err(IoError::Arrow(
            "feather reader produced zero record batches".to_owned(),
        ));
    }

    let refs: Vec<&DataFrame> = all_frames.iter().collect();
    fp_frame::concat_dataframes(&refs).map_err(IoError::from)
}

/// Write a DataFrame to an Arrow IPC (Feather v2) file.
///
/// Matches `pd.DataFrame.to_feather(path)`.
pub fn write_feather(frame: &DataFrame, path: &Path) -> Result<(), IoError> {
    let bytes = write_feather_bytes(frame)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Read a DataFrame from an Arrow IPC (Feather v2) file.
///
/// Matches `pd.read_feather(path)`.
pub fn read_feather(path: &Path) -> Result<DataFrame, IoError> {
    let data = std::fs::read(path)?;
    read_feather_bytes(&data)
}

/// Write a DataFrame to Arrow IPC stream bytes (streaming format, no random access).
///
/// Unlike Feather (file format), the stream format has no footer and supports
/// streaming reads without seeking. Used for inter-process communication.
pub fn write_ipc_stream_bytes(frame: &DataFrame) -> Result<Vec<u8>, IoError> {
    use arrow::ipc::writer::StreamWriter;

    let batch = dataframe_to_record_batch(frame)?;
    let schema = batch.schema();

    let mut buf = Vec::new();
    let mut writer =
        StreamWriter::try_new(&mut buf, &schema).map_err(|e| IoError::Arrow(e.to_string()))?;
    writer
        .write(&batch)
        .map_err(|e| IoError::Arrow(e.to_string()))?;
    writer.finish().map_err(|e| IoError::Arrow(e.to_string()))?;
    Ok(buf)
}

/// Read a DataFrame from Arrow IPC stream bytes (streaming format).
pub fn read_ipc_stream_bytes(data: &[u8]) -> Result<DataFrame, IoError> {
    use arrow::ipc::reader::StreamReader;

    let cursor = std::io::Cursor::new(data);
    let reader = StreamReader::try_new(cursor, None).map_err(|e| IoError::Arrow(e.to_string()))?;

    let mut all_frames: Vec<DataFrame> = Vec::new();
    for batch_result in reader {
        let batch = batch_result.map_err(|e| IoError::Arrow(e.to_string()))?;
        all_frames.push(record_batch_to_dataframe(&batch)?);
    }

    if all_frames.is_empty() {
        return Ok(DataFrame::new_with_column_order(
            Index::new(vec![]),
            BTreeMap::new(),
            vec![],
        )?);
    }

    if all_frames.len() == 1 {
        if let Some(frame) = all_frames.into_iter().next() {
            return Ok(frame);
        }
        return Err(IoError::Arrow(
            "ipc stream reader produced zero record batches".to_owned(),
        ));
    }

    let refs: Vec<&DataFrame> = all_frames.iter().collect();
    fp_frame::concat_dataframes(&refs).map_err(IoError::from)
}

// ── SQL I/O ─────────────────────────────────────────────────────────────

/// Options for writing a DataFrame to SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlIfExists {
    /// Raise an error if the table already exists.
    Fail,
    /// Drop the table and recreate it.
    Replace,
    /// Insert new rows into the existing table.
    Append,
}

/// Strategy for emitting INSERT statements during `write_sql`.
///
/// Matches `pd.DataFrame.to_sql(.., method=...)` shape:
/// `Single` (default) emits one `INSERT INTO t VALUES (?, ...)` per row,
/// reusing a prepared statement under a transaction. `Multi` builds a
/// single multi-row `INSERT INTO t VALUES (...), (...), ...` statement
/// per chunk, where chunk size is `max_param_count() / num_cols` (or
/// the whole frame when the backend reports no max). `Multi` typically
/// wins on backends with high per-statement overhead (PostgreSQL,
/// MySQL); SQLite is already fast under prepared-statement reuse so the
/// gap there is small.
///
/// Per br-frankenpandas-i0ml (fd90.19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SqlInsertMethod {
    /// One INSERT per row, prepared once and re-bound per row.
    #[default]
    Single,
    /// Multi-row INSERT, chunked by the backend's parameter limit.
    Multi,
}

/// Options for reading SQL query results into a DataFrame.
#[derive(Debug, Clone)]
pub struct SqlReadOptions {
    /// Positional parameters to bind to `?` placeholders in the SQL query.
    pub params: Option<Vec<Scalar>>,
    /// Column names to coerce via pandas-style parse_dates handling.
    /// Currently supports explicit column-name selection.
    pub parse_dates: Option<Vec<String>>,
    /// Promote decimal-like SQL text result columns to Float64.
    ///
    /// This is an opt-in form of pandas' `coerce_float` behavior for
    /// backends that expose NUMERIC/DECIMAL/MONEY values through text.
    /// Columns containing any non-numeric strings are left unchanged.
    pub coerce_float: bool,
    /// Per-column dtype override applied after row materialization.
    ///
    /// Matches `pd.read_sql(.., dtype={'col': 'float64'})`. Each entry casts
    /// the named column to the declared dtype using `fp_types::cast_scalar_owned`.
    /// Map entries for columns not present in the result are silently
    /// ignored (matches pandas). Columns also listed in `parse_dates` are
    /// skipped to avoid double-cast errors — parse_dates wins.
    ///
    /// Per br-frankenpandas-l9pt (fd90.11).
    pub dtype: Option<BTreeMap<String, DType>>,
    /// Optional schema namespace for `read_sql_table` lookups.
    ///
    /// Matches `pd.read_sql_table(table, con, schema=...)`. When the
    /// backend reports `supports_schemas() == true` (PostgreSQL, MySQL,
    /// MSSQL, etc.) and `schema` is `Some(s)`, the SELECT references
    /// `s.table` with each part quoted by `conn.quote_identifier(...)`.
    /// When the backend reports `supports_schemas() == false` (SQLite),
    /// any `Some(s)` here is silently ignored and the bare `table`
    /// reference is used — pandas/SQLAlchemy raises NotImplementedError
    /// there but we choose ignore-with-doc-note so SQLite users with
    /// pandas-shaped option structs aren't broken.
    ///
    /// Per br-frankenpandas-u6zn (fd90.14).
    pub schema: Option<String>,
    /// Optional projection list for `read_sql_table` reads.
    ///
    /// Matches `pd.read_sql_table(table, con, columns=[...])`. When
    /// `Some(list)`, the emitted SELECT projects only those columns
    /// (and in that order) instead of `SELECT *`. `None` preserves
    /// `SELECT *`. An empty Vec is rejected with `IoError::Sql` —
    /// pandas raises ValueError there. Each entry is validated via
    /// the standard alphanumeric+underscore policy.
    ///
    /// Note: `read_sql` / `read_sql_query` ignore this field — it
    /// only takes effect on `read_sql_table*` paths, where
    /// frankenpandas builds the SELECT itself.
    ///
    /// Per br-frankenpandas-d3e9 (fd90.34).
    pub columns: Option<Vec<String>>,
    /// Optional column name to promote to the DataFrame index.
    ///
    /// Matches `pd.read_sql(.., index_col=...)` and
    /// `pd.read_sql_table(table, con, index_col=...)`. When
    /// `Some(name)`, after row materialization the named column is
    /// removed from the result and used as the DataFrame index. The
    /// column must exist in the read result (or in the projection
    /// when `columns` is also set). `None` preserves the default
    /// RangeIndex.
    ///
    /// Empty string is rejected with `IoError::Sql` to match
    /// pandas' ValueError. List-of-strings (for MultiIndex) is out
    /// of scope for this slice — single index only.
    ///
    /// Per br-frankenpandas-c1h9 (fd90.36).
    pub index_col: Option<String>,
}

impl Default for SqlReadOptions {
    /// Per br-frankenpandas-o0x6 (fd90.41): pandas defaults
    /// `coerce_float=True` for `read_sql` / `read_sql_query` /
    /// `read_sql_table` so we follow suit. Other defaults are the
    /// natural empty / None values.
    fn default() -> Self {
        Self {
            params: None,
            parse_dates: None,
            coerce_float: true,
            dtype: None,
            schema: None,
            columns: None,
            index_col: None,
        }
    }
}

/// Options for writing a DataFrame to SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlWriteOptions {
    /// Behavior when the target table already exists.
    pub if_exists: SqlIfExists,
    /// Whether to materialize the DataFrame index as the leading SQL column.
    pub index: bool,
    /// Optional override for the emitted index column name.
    pub index_label: Option<String>,
    /// Optional schema namespace for CREATE TABLE / INSERT routing.
    ///
    /// Matches `pd.DataFrame.to_sql(table, con, schema=...)`. When the
    /// backend reports `supports_schemas() == true` and `schema` is
    /// `Some(s)`, generated CREATE TABLE / INSERT statements target
    /// `\"s\".\"table\"`. On backends that report `false` (SQLite),
    /// `Some(s)` is silently ignored — preserves SQLite users' existing
    /// option structs.
    ///
    /// Per br-frankenpandas-udn6 (fd90.15).
    pub schema: Option<String>,
    /// Per-column SQL-type override applied during CREATE TABLE.
    ///
    /// Matches `pd.DataFrame.to_sql(.., dtype={'amount': 'NUMERIC(10,2)'})`.
    /// Each entry's value is the literal SQL type string emitted in the
    /// column definition for that column. Map entries for columns not
    /// in the frame are silently ignored (matches pandas). Falls back
    /// to `conn.dtype_sql(DType)` when no override is present.
    ///
    /// Per br-frankenpandas-ev2s (fd90.18).
    pub dtype: Option<BTreeMap<String, String>>,
    /// INSERT-emission strategy.
    ///
    /// Default `Single` matches pandas' default: one INSERT per row,
    /// re-binding a prepared statement under a transaction. `Multi`
    /// switches to multi-row VALUES batched by `conn.max_param_count()`,
    /// matching `pd.to_sql(.., method='multi')`.
    ///
    /// Per br-frankenpandas-i0ml (fd90.19).
    pub method: SqlInsertMethod,
    /// Maximum rows per transaction-bounded INSERT chunk.
    ///
    /// Matches `pd.DataFrame.to_sql(.., chunksize=...)`. When `Some(n)`,
    /// the row emit loop batches into chunks of `n` rows, each routed
    /// through its own `insert_rows` call (which on transactional
    /// backends commits a fresh transaction per chunk). This caps WAL /
    /// journal size for huge frames where the default `Single`
    /// single-transaction mode would balloon. For `Multi` mode the
    /// effective per-chunk row count is `min(chunksize,
    /// max_param_count / num_cols)`. `None` preserves existing
    /// single-transaction semantics.
    ///
    /// `Some(0)` is rejected — pandas raises ValueError there too.
    ///
    /// Per br-frankenpandas-ls9z (fd90.33).
    pub chunksize: Option<usize>,
}

/// Backend-agnostic in-memory representation of a SQL query result.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Scalar>>,
}

/// Backend-neutral SQL column metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlColumnSchema {
    pub name: String,
    pub declared_type: Option<String>,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub primary_key_ordinal: Option<usize>,
    /// Column-level comment text, when the backend exposes one.
    ///
    /// Per br-frankenpandas-cfld (fd90.35). SQLite has no column-comment
    /// storage so the rusqlite override always emits `None`. PostgreSQL
    /// impls populate from `pg_catalog.pg_description.col_description`;
    /// MySQL uses `information_schema.columns.column_comment`; MSSQL
    /// reads from `sys.extended_properties`. Companion to the
    /// table-level `table_comment` (fd90.32) — together they round
    /// out SQLAlchemy.Inspector.get_columns() parity (its dict shape
    /// includes a `'comment'` key).
    pub comment: Option<String>,
    /// Whether the column is auto-incrementing.
    ///
    /// Per br-frankenpandas-bkl2 (fd90.37). Completes
    /// SQLAlchemy.Inspector.get_columns() parity (the dict shape
    /// includes an `'autoincrement'` key).
    ///
    /// SQLite detection rule (in the rusqlite `table_schema`
    /// override): true when `declared_type` is `INTEGER`
    /// (case-insensitive) AND the column is the sole primary key
    /// (`primary_key_ordinal == Some(0)`) — SQLite makes
    /// `INTEGER PRIMARY KEY` an alias for the auto-increment
    /// `rowid`. The optional `AUTOINCREMENT` keyword affects
    /// whether IDs are reused after delete but does not change the
    /// "is auto-incrementing" property pandas cares about.
    ///
    /// Other backends: PG SERIAL/BIGSERIAL/IDENTITY columns
    /// `true`; MySQL `AUTO_INCREMENT` modifier `true`; otherwise
    /// `false`.
    pub autoincrement: bool,
}

/// Backend-neutral SQL table metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlTableSchema {
    pub table_name: String,
    pub columns: Vec<SqlColumnSchema>,
}

impl SqlTableSchema {
    pub fn column(&self, name: &str) -> Option<&SqlColumnSchema> {
        self.columns.iter().find(|column| column.name == name)
    }
}

/// Backend-neutral SQL index metadata.
///
/// Per br-frankenpandas-bgv9 (fd90.28). Used by `list_indexes` /
/// `list_sql_indexes` to surface user-defined indexes so callers can
/// align with `SQLAlchemy.Inspector.get_indexes()` shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlIndexSchema {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

/// Backend-neutral SQL unique-constraint metadata.
///
/// Per br-frankenpandas-sh4v (fd90.31). Surfaces inline `UNIQUE`
/// declarations and `UNIQUE (...)` table constraints separately from
/// user-created `CREATE UNIQUE INDEX` (those land in
/// `SqlIndexSchema` via `list_indexes`). `name` may be backend-
/// generated (SQLite reports `sqlite_autoindex_<table>_<n>`) when
/// the constraint was declared inline without an explicit name —
/// we surface the backend's name verbatim rather than fabricating.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlUniqueConstraintSchema {
    pub name: String,
    pub columns: Vec<String>,
}

/// Bundle of all introspection metadata for a single SQL table.
///
/// Per br-frankenpandas-76mw (fd90.40). Returned by
/// `SqlInspector::reflect_table` to give callers the full picture of
/// a table in one call instead of 5 separate trait dispatches.
/// Mirrors the bundled view that `SQLAlchemy.MetaData.reflect_table`
/// builds internally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlReflectedTable {
    pub table_name: String,
    pub columns: Vec<SqlColumnSchema>,
    pub primary_key_columns: Vec<String>,
    pub indexes: Vec<SqlIndexSchema>,
    pub foreign_keys: Vec<SqlForeignKeySchema>,
    pub unique_constraints: Vec<SqlUniqueConstraintSchema>,
    pub comment: Option<String>,
}

impl SqlReflectedTable {
    /// Look up a column by name. Mirrors `SqlTableSchema::column`.
    ///
    /// Per br-frankenpandas-63ac (fd90.51).
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&SqlColumnSchema> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Look up an index by name.
    ///
    /// Per br-frankenpandas-63ac (fd90.51).
    #[must_use]
    pub fn index(&self, name: &str) -> Option<&SqlIndexSchema> {
        self.indexes.iter().find(|i| i.name == name)
    }

    /// Look up a unique constraint by name (backend-generated names
    /// like `sqlite_autoindex_*` count too — match what
    /// `list_unique_constraints` surfaced).
    ///
    /// Per br-frankenpandas-63ac (fd90.51).
    #[must_use]
    pub fn unique_constraint(&self, name: &str) -> Option<&SqlUniqueConstraintSchema> {
        self.unique_constraints.iter().find(|u| u.name == name)
    }

    /// Find every foreign key whose `columns` slice contains `column_name`.
    ///
    /// A given column may participate in multiple FK constraints (e.g.
    /// the same column referenced by separate FKs to different parents
    /// — rare but valid SQL). Returns the matching FKs in their
    /// declaration order from PRAGMA foreign_key_list.
    ///
    /// Per br-frankenpandas-63ac (fd90.51).
    #[must_use]
    pub fn foreign_keys_for_column(&self, column_name: &str) -> Vec<&SqlForeignKeySchema> {
        self.foreign_keys
            .iter()
            .filter(|fk| fk.columns.iter().any(|c| c == column_name))
            .collect()
    }

    /// Find every index whose `columns` slice contains `column_name`.
    ///
    /// Matches multi-column indexes where the column appears at any
    /// position (first, middle, last) — useful for answering "is this
    /// column indexed" rather than the more restrictive "is this
    /// column the leading entry of an index" question.
    ///
    /// Per br-frankenpandas-37uy (fd90.52).
    #[must_use]
    pub fn indexes_for_column(&self, column_name: &str) -> Vec<&SqlIndexSchema> {
        self.indexes
            .iter()
            .filter(|i| i.columns.iter().any(|c| c == column_name))
            .collect()
    }

    /// Find every UNIQUE constraint whose `columns` slice contains
    /// `column_name`.
    ///
    /// Same any-position match semantics as `indexes_for_column` and
    /// `foreign_keys_for_column`.
    ///
    /// Per br-frankenpandas-37uy (fd90.52).
    #[must_use]
    pub fn unique_constraints_for_column(
        &self,
        column_name: &str,
    ) -> Vec<&SqlUniqueConstraintSchema> {
        self.unique_constraints
            .iter()
            .filter(|u| u.columns.iter().any(|c| c == column_name))
            .collect()
    }
}

/// Backend-neutral SQL foreign-key constraint metadata.
///
/// Per br-frankenpandas-uht8 (fd90.29). Aligns with
/// `SQLAlchemy.Inspector.get_foreign_keys()` shape: a single FK
/// constraint may span multiple columns (composite FK), and
/// `columns[i]` references `referenced_columns[i]` on
/// `referenced_table`. `constraint_name` is `None` for SQLite
/// inline FKs declared without an explicit CONSTRAINT name (PRAGMA
/// foreign_key_list does not surface a name, so we return None
/// there rather than fabricating one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlForeignKeySchema {
    pub constraint_name: Option<String>,
    pub columns: Vec<String>,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
}

/// Iterator over DataFrame chunks produced by a SQL query.
#[derive(Debug, Clone)]
pub struct SqlChunkIterator {
    headers: Vec<String>,
    columns: Vec<Vec<Scalar>>,
    chunk_size: usize,
    next_row: usize,
}

impl Iterator for SqlChunkIterator {
    type Item = Result<DataFrame, IoError>;

    fn next(&mut self) -> Option<Self::Item> {
        let row_count = self.columns.first().map_or(0, Vec::len);
        if self.next_row >= row_count {
            return None;
        }

        let start = self.next_row;
        let end = start.saturating_add(self.chunk_size).min(row_count);
        self.next_row = end;

        let chunk_columns = self
            .columns
            .iter()
            .map(|column| column[start..end].to_vec())
            .collect();
        Some(dataframe_from_sql_columns(
            self.headers.clone(),
            chunk_columns,
        ))
    }
}

/// Iterator over SQL DataFrame chunks with optional per-chunk index promotion.
#[derive(Debug, Clone)]
pub struct SqlIndexedChunkIterator {
    inner: SqlChunkIterator,
    index_col: Option<String>,
}

impl Iterator for SqlIndexedChunkIterator {
    type Item = Result<DataFrame, IoError>;

    fn next(&mut self) -> Option<Self::Item> {
        let chunk = self.inner.next()?;
        Some(match (chunk, self.index_col.as_deref()) {
            (Ok(frame), Some(index_col)) => promote_column_to_index(&frame, index_col),
            (Ok(frame), None) => Ok(frame),
            (Err(err), _) => Err(err),
        })
    }
}

fn sql_indexed_chunks(
    inner: SqlChunkIterator,
    index_col: Option<&str>,
) -> Result<SqlIndexedChunkIterator, IoError> {
    if let Some(col_name) = index_col
        && !inner.headers.iter().any(|header| header == col_name)
    {
        return Err(IoError::Sql(format!(
            "index_col {col_name:?} not present in result columns"
        )));
    }
    Ok(SqlIndexedChunkIterator {
        inner,
        index_col: index_col.map(str::to_owned),
    })
}

/// Minimal SQL connection surface needed by FrankenPandas SQL IO.
pub trait SqlConnection {
    fn query(&self, query: &str, params: &[Scalar]) -> Result<SqlQueryResult, IoError>;

    fn execute_batch(&self, sql: &str) -> Result<(), IoError>;

    fn table_exists(&self, table_name: &str) -> Result<bool, IoError>;

    fn insert_rows(&self, insert_sql: &str, rows: &[Vec<Scalar>]) -> Result<(), IoError>;

    fn dtype_sql(&self, dtype: DType) -> &'static str;

    fn index_dtype_sql(&self, index: &Index) -> &'static str;

    /// Return the bind marker for the one-based parameter ordinal.
    ///
    /// SQLite and MySQL accept `?`; PostgreSQL-style backends use `$1`,
    /// `$2`, ... . Keeping marker generation on the backend trait lets
    /// write_sql stay generic without leaking backend dialect branches.
    fn parameter_marker(&self, _ordinal: usize) -> String {
        "?".to_owned()
    }

    // ── Backend-capability + dialect probes (br-frankenpandas-6dtf) ─────
    //
    // Default impls return conservative values so existing implementations
    // stay backwards-compatible; concrete backends override per their
    // engine. Phase 2 wires these probes into the read_sql / read_sql_table
    // dispatch path so per-backend SQL quirks (RETURNING, max param count,
    // transaction semantics) can fan out without leaking concrete types.

    /// Short identifier for this backend's SQL dialect.
    ///
    /// Used by diagnostics + DISCREPANCIES.md routing. Backends should
    /// override with the canonical pandas/SQLAlchemy dialect name:
    /// `"sqlite"`, `"postgresql"`, `"mysql"`, `"mariadb"`, `"oracle"`, etc.
    /// Default `"unknown"` flags un-customized impls during reviews.
    fn dialect_name(&self) -> &'static str {
        "unknown"
    }

    /// Whether this backend honors `INSERT ... RETURNING ...` natively.
    ///
    /// Drives the write_sql path's choice between RETURNING-based row
    /// retrieval and a follow-up SELECT. Default `false` is the
    /// conservative choice (forces follow-up SELECT path) until each
    /// backend opts in.
    fn supports_returning(&self) -> bool {
        false
    }

    /// Hard upper bound on bound-parameter count per statement, if known.
    ///
    /// SQLite (3.32+): 32766. PostgreSQL: 65535. MySQL: 65535. Backends
    /// that don't surface a meaningful cap return `None`. The bulk-insert
    /// path uses this to chunk multi-row INSERTs so a single executemany
    /// stays under the backend's parameter ceiling.
    fn max_param_count(&self) -> Option<usize> {
        None
    }

    /// Maximum identifier length supported by the backend, or `None`
    /// when no documented limit exists (or the limit is irrelevant for
    /// the deployment).
    ///
    /// Per br-frankenpandas-cs81 (fd90.26). Defaults to `None`. Known
    /// caps: PostgreSQL = 63, MySQL = 64, MSSQL = 128, Oracle = 30
    /// (pre-12.2) or 128 (12.2+), SQLite = no documented limit.
    /// Useful for to_sql validation when emitting auto-generated
    /// index/constraint/column names that could otherwise exceed
    /// backend limits and produce truncated or rejected DDL.
    fn max_identifier_length(&self) -> Option<usize> {
        None
    }

    /// Run `f` inside a transaction. The default impl runs `f` without
    /// BEGIN/COMMIT — backends that support transactions should override
    /// to wrap in their native transaction primitive (rusqlite `BEGIN`,
    /// tokio-postgres `BEGIN`, mysql `START TRANSACTION`, ...). On `Err`
    /// from `f`, transactional backends roll back; on `Ok` they commit.
    ///
    /// The default impl is intentionally a no-op so non-transactional
    /// connection wrappers (e.g. test doubles) compile without
    /// implementation effort. Production backends MUST override.
    fn with_transaction<T, F>(&self, f: F) -> Result<T, IoError>
    where
        F: FnOnce(&Self) -> Result<T, IoError>,
        Self: Sized,
    {
        f(self)
    }

    /// Quote a SQL identifier (table name, column name, schema name) for
    /// safe inclusion in a generated statement.
    ///
    /// Default impl: ANSI `"..."` form with embedded `"` doubled (matches
    /// SQLite + PostgreSQL). MySQL/MariaDB backends must override to
    /// produce `\`...\`` (backtick). MSSQL backends use `[...]`. Per
    /// br-frankenpandas-2y7w (fd90.10).
    ///
    /// NUL bytes in the identifier are rejected (security: prevents
    /// statement-injection via embedded null terminators in C-string
    /// driver layers).
    fn quote_identifier(&self, ident: &str) -> Result<String, IoError> {
        if ident.contains('\0') {
            return Err(IoError::Sql(
                "invalid SQL identifier: NUL byte".to_owned(),
            ));
        }
        Ok(format!("\"{}\"", ident.replace('"', "\"\"")))
    }

    /// Whether this backend exposes multi-schema namespacing.
    ///
    /// PostgreSQL / MySQL / MariaDB / MSSQL / Oracle: true. SQLite (one
    /// schema per file connection — though ATTACH adds named schemas as
    /// a special case): false. Drives the `pd.read_sql_table(.., schema=X)`
    /// path's choice between qualifying the table reference (`schema.table`)
    /// and a connection-level pre-SET. Per br-frankenpandas-6dk9 (fd90.13).
    fn supports_schemas(&self) -> bool {
        false
    }

    /// Default schema for unqualified table references, if the backend
    /// has one.
    ///
    /// PostgreSQL: `'public'` (or whatever the connection's `search_path`
    /// resolves to first). MySQL: the database name passed to the
    /// connection URL. SQLite: `None` (single namespace). The `read_sql_table`
    /// dispatch uses this when the user passes `schema=None` to choose
    /// between a bare `SELECT * FROM \"table\"` and a schema-qualified form.
    /// Default `None` keeps behavior identical to today's SQLite-only path.
    fn default_schema(&self) -> Option<String> {
        None
    }

    /// Schema-aware table-existence check.
    ///
    /// Per br-frankenpandas-70d1 (fd90.17). Default impl delegates to
    /// `table_exists(table)` and ignores the schema argument — matches
    /// single-namespace embedded backends like SQLite. Multi-schema
    /// backends (PostgreSQL, MySQL, MSSQL) override to scope the check
    /// to the requested schema, so write_sql's `Fail` branch correctly
    /// distinguishes `analytics.users` from `audit.users`. The schema
    /// arg passes through unchanged — backends MAY consult
    /// `default_schema()` for their own fallback logic when `schema`
    /// is `None` (per fd90.57: this fallback is NOT applied by the
    /// default impl or the SQLite override).
    fn table_exists_in_schema(
        &self,
        table_name: &str,
        _schema: Option<&str>,
    ) -> Result<bool, IoError> {
        self.table_exists(table_name)
    }

    /// List user-visible table names, optionally scoped to `schema`.
    ///
    /// Per br-frankenpandas-vhq2 (fd90.20). Default impl returns an empty
    /// vector — backends that cannot introspect (or that haven't yet
    /// implemented this method) report "no tables visible" rather than
    /// raising. Multi-schema backends (PostgreSQL, MySQL, MSSQL) override
    /// to query their information_schema; embedded backends (SQLite)
    /// override to query their internal catalog and ignore `schema`.
    fn list_tables(&self, _schema: Option<&str>) -> Result<Vec<String>, IoError> {
        Ok(Vec::new())
    }

    /// Introspect a table's column metadata, optionally schema-scoped.
    ///
    /// Per br-frankenpandas-w43q (fd90.21). Returns `Ok(None)` if the
    /// table does not exist. Default impl returns `Ok(None)` for
    /// backends that cannot introspect; rusqlite overrides to use
    /// `PRAGMA table_info`. Multi-schema backends override to query
    /// `information_schema.columns` filtered by `schema`. Schema arg is
    /// silently ignored when `supports_schemas() == false`.
    fn table_schema(
        &self,
        _table_name: &str,
        _schema: Option<&str>,
    ) -> Result<Option<SqlTableSchema>, IoError> {
        Ok(None)
    }

    /// List user-visible schemas (PostgreSQL "schemas", MySQL "databases").
    ///
    /// Per br-frankenpandas-lxhi (fd90.22). Default impl returns an empty
    /// vector. Single-namespace backends (SQLite) return empty as well —
    /// they have no meaningful schema concept. Multi-schema backends
    /// (PostgreSQL, MySQL, MSSQL) override to query their catalog and
    /// filter out internal/system schemas (`pg_*`, `information_schema`,
    /// `mysql`, `performance_schema`, etc.) so user-visible schemas
    /// surface cleanly.
    fn list_schemas(&self) -> Result<Vec<String>, IoError> {
        Ok(Vec::new())
    }

    /// Probe the backend server's version string.
    ///
    /// Per br-frankenpandas-e23k (fd90.24). Useful for dialect-version
    /// gating (INSERT ... RETURNING needs PG 8.2+ / SQLite 3.35.0+,
    /// JSON operators need MySQL 5.7.8+, etc.) and for diagnostics.
    /// Default impl returns `Ok(None)` so backends that can't probe
    /// (or that haven't yet implemented this) report "unknown" rather
    /// than raising. rusqlite override returns the SQLite library
    /// version. PostgreSQL/MySQL impls should override with
    /// `SHOW server_version` / `SELECT VERSION()`.
    fn server_version(&self) -> Result<Option<String>, IoError> {
        Ok(None)
    }

    /// List user-visible view names, optionally scoped to `schema`.
    ///
    /// Per br-frankenpandas-gm3r (fd90.30). Default impl returns an
    /// empty vector. rusqlite override queries `sqlite_master WHERE
    /// type='view'`, excluding internal `sqlite_*` views. Multi-schema
    /// backends override with `information_schema.views` filtered by
    /// the schema arg. Schema is silently ignored when
    /// `supports_schemas() == false`. Companion to `list_tables` —
    /// pandas/SQLAlchemy keep tables and views in distinct buckets so
    /// `pd.read_sql_table` can distinguish them.
    fn list_views(&self, _schema: Option<&str>) -> Result<Vec<String>, IoError> {
        Ok(Vec::new())
    }

    /// List indexes defined on a table, optionally schema-scoped.
    ///
    /// Per br-frankenpandas-bgv9 (fd90.28). Default impl returns
    /// `Ok(Vec::new())` for backends that can't introspect. rusqlite
    /// override uses `PRAGMA index_list(table)` + `PRAGMA index_info`
    /// per index, surfacing only user-created indexes (the auto-created
    /// indexes for PRIMARY KEY constraints are filtered out to match
    /// SQLAlchemy.Inspector.get_indexes() semantics). Multi-schema
    /// backends override with information_schema queries.
    fn list_indexes(
        &self,
        _table_name: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<SqlIndexSchema>, IoError> {
        Ok(Vec::new())
    }

    /// List UNIQUE constraints declared on a table (inline or table-level),
    /// excluding `CREATE UNIQUE INDEX` indexes (those land in `list_indexes`).
    ///
    /// Per br-frankenpandas-sh4v (fd90.31). Default impl returns
    /// `Ok(Vec::new())`. rusqlite override uses
    /// `PRAGMA index_list(table)` filtered by `origin == 'u'` (the
    /// auto-created indexes that back declared UNIQUE constraints)
    /// then `PRAGMA index_info` per match. Multi-schema backends
    /// override with information_schema queries.
    fn list_unique_constraints(
        &self,
        _table_name: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<SqlUniqueConstraintSchema>, IoError> {
        Ok(Vec::new())
    }

    /// Probe the table-level comment, optionally schema-scoped.
    ///
    /// Per br-frankenpandas-yu3w (fd90.32). Default impl returns
    /// `Ok(None)` — SQLite has no native table-comment storage so it
    /// inherits the default. PostgreSQL impls should override using
    /// `pg_catalog.obj_description(...)` or
    /// `pg_catalog.pg_class.relkind` joined to `pg_description`;
    /// MySQL uses `information_schema.tables.table_comment`; MSSQL
    /// reads from `sys.extended_properties`. Aligns with
    /// `SQLAlchemy.Inspector.get_table_comment()` shape (returns
    /// `{'text': comment_or_none}`).
    fn table_comment(
        &self,
        _table_name: &str,
        _schema: Option<&str>,
    ) -> Result<Option<String>, IoError> {
        Ok(None)
    }

    /// List foreign-key constraints declared on a table, optionally
    /// schema-scoped.
    ///
    /// Per br-frankenpandas-uht8 (fd90.29). Default impl returns
    /// `Ok(Vec::new())`. rusqlite override uses
    /// `PRAGMA foreign_key_list(table)`, grouping rows by their `id`
    /// column (each id is a single FK constraint that may span multiple
    /// columns) and ordering paired columns by `seq`. Multi-schema
    /// backends override with `information_schema.referential_constraints`
    /// + `key_column_usage` joined queries.
    fn list_foreign_keys(
        &self,
        _table_name: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<SqlForeignKeySchema>, IoError> {
        Ok(Vec::new())
    }

    /// Return the primary-key column names for a table, ordered by
    /// the `primary_key_ordinal` reported by `table_schema`.
    ///
    /// Per br-frankenpandas-uw3y (fd90.25). Default impl delegates to
    /// `table_schema(table, schema)` and pulls out columns whose
    /// `primary_key_ordinal` is `Some(_)`, sorted ascending. Returns
    /// an empty vector when:
    /// - the table doesn't exist (`table_schema` returns `Ok(None)`),
    /// - the table has no primary key,
    /// - the backend can't introspect (default `table_schema`).
    ///
    /// Useful for upsert conflict-target generation, `index_label`
    /// defaulting, and schema validation. Backends can override to
    /// query their catalog directly when `table_schema` is too heavy.
    fn primary_key_columns(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<Vec<String>, IoError> {
        // Per fd90.47: defer to the shared primary_keys_from_schema
        // helper so the filter+sort logic lives in exactly one place
        // (the helper is also used by SqlInspector::reflect_table).
        let Some(meta) = self.table_schema(table_name, schema)? else {
            return Ok(Vec::new());
        };
        Ok(primary_keys_from_schema(&meta))
    }

    /// Reset a table to empty without dropping its definition.
    ///
    /// Per br-frankenpandas-phum (fd90.23). Default impl emits
    /// `DELETE FROM <table>` — universal SQL that every backend
    /// supports, but slower than TRUNCATE on large tables because the
    /// row deletes are logged in the transaction journal. PostgreSQL
    /// and MySQL backends should override with `TRUNCATE TABLE`,
    /// which is dramatically faster (DDL-style fast-path) and resets
    /// auto-increment sequences. The schema arg routes through
    /// `quote_identifier` and is silently ignored when
    /// `supports_schemas() == false`.
    fn truncate_table(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<(), IoError> {
        validate_sql_table_name(table_name)?;
        let qualified = match schema {
            Some(s) if self.supports_schemas() => {
                validate_sql_schema_name(s)?;
                format!(
                    "{}.{}",
                    self.quote_identifier(s)?,
                    self.quote_identifier(table_name)?
                )
            }
            _ => self.quote_identifier(table_name)?,
        };
        self.execute_batch(&format!("DELETE FROM {qualified}"))
    }
}

/// Map an fp-types DType to an SQLite column type declaration.
#[cfg(feature = "sql-sqlite")]
fn dtype_to_sql(dtype: DType) -> &'static str {
    match dtype {
        DType::Int64 => "INTEGER",
        DType::Float64 => "REAL",
        DType::Utf8 => "TEXT",
        DType::Categorical => "TEXT",
        DType::Bool => "INTEGER",
        DType::Null => "TEXT",
        DType::Timedelta64 => "INTEGER", // store as nanoseconds
        DType::Sparse => "TEXT",
    }
}

/// Convert an SQLite column value to a Scalar.
#[cfg(feature = "sql-sqlite")]
fn sql_value_to_scalar(value: &rusqlite::types::Value) -> Scalar {
    match value {
        rusqlite::types::Value::Null => Scalar::Null(NullKind::Null),
        rusqlite::types::Value::Integer(v) => Scalar::Int64(*v),
        rusqlite::types::Value::Real(v) => Scalar::Float64(*v),
        rusqlite::types::Value::Text(s) => Scalar::Utf8(s.clone()),
        rusqlite::types::Value::Blob(b) => Scalar::Utf8(format!("<blob:{} bytes>", b.len())),
    }
}

#[cfg(feature = "sql-sqlite")]
fn sql_value_from_scalar(scalar: &Scalar) -> rusqlite::types::Value {
    match scalar {
        Scalar::Int64(v) => rusqlite::types::Value::Integer(*v),
        Scalar::Float64(v) => {
            if v.is_nan() {
                rusqlite::types::Value::Null
            } else {
                rusqlite::types::Value::Real(*v)
            }
        }
        Scalar::Bool(b) => rusqlite::types::Value::Integer(if *b { 1 } else { 0 }),
        Scalar::Utf8(s) => rusqlite::types::Value::Text(s.clone()),
        Scalar::Null(_) => rusqlite::types::Value::Null,
        Scalar::Timedelta64(v) => {
            if *v == Timedelta::NAT {
                rusqlite::types::Value::Null
            } else {
                rusqlite::types::Value::Integer(*v)
            }
        }
    }
}

fn scalar_from_index_label(label: &IndexLabel) -> Scalar {
    match label {
        IndexLabel::Int64(v) => Scalar::Int64(*v),
        IndexLabel::Utf8(s) => Scalar::Utf8(s.clone()),
        IndexLabel::Timedelta64(v) => {
            if *v == Timedelta::NAT {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Timedelta64(*v)
            }
        }
        IndexLabel::Datetime64(v) => {
            if *v == i64::MIN {
                Scalar::Null(NullKind::Null)
            } else {
                Scalar::Utf8(format_datetime_ns(*v))
            }
        }
    }
}

#[cfg(feature = "sql-sqlite")]
impl SqlConnection for rusqlite::Connection {
    fn query(&self, query: &str, params: &[Scalar]) -> Result<SqlQueryResult, IoError> {
        let mut stmt = self
            .prepare(query)
            .map_err(|e| IoError::Sql(format!("prepare failed: {e}")))?;

        let col_count = stmt.column_count();
        let columns: Vec<String> = (0..col_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_owned())
            .collect();

        let sql_params = params.iter().map(sql_value_from_scalar).collect::<Vec<_>>();
        let mut rows = stmt
            .query(rusqlite::params_from_iter(sql_params.iter()))
            .map_err(|e| IoError::Sql(format!("query failed: {e}")))?;

        let mut out_rows = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| IoError::Sql(format!("row fetch failed: {e}")))?
        {
            let mut values = Vec::with_capacity(col_count);
            for col_idx in 0..col_count {
                let value: rusqlite::types::Value = row
                    .get(col_idx)
                    .map_err(|e| IoError::Sql(format!("cell read failed: {e}")))?;
                values.push(sql_value_to_scalar(&value));
            }
            out_rows.push(values);
        }

        Ok(SqlQueryResult {
            columns,
            rows: out_rows,
        })
    }

    fn execute_batch(&self, sql: &str) -> Result<(), IoError> {
        rusqlite::Connection::execute_batch(self, sql)
            .map_err(|e| IoError::Sql(format!("execute_batch failed: {e}")))
    }

    fn table_exists(&self, table_name: &str) -> Result<bool, IoError> {
        self.prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1")
            .and_then(|mut stmt| stmt.exists(rusqlite::params![table_name]))
            .map_err(|e| IoError::Sql(format!("existence check failed: {e}")))
    }

    fn insert_rows(&self, insert_sql: &str, rows: &[Vec<Scalar>]) -> Result<(), IoError> {
        let tx = self
            .unchecked_transaction()
            .map_err(|e| IoError::Sql(format!("begin transaction failed: {e}")))?;

        {
            let mut stmt = tx
                .prepare_cached(insert_sql)
                .map_err(|e| IoError::Sql(format!("prepare insert failed: {e}")))?;

            for (row_idx, row_values) in rows.iter().enumerate() {
                let params = row_values
                    .iter()
                    .map(sql_value_from_scalar)
                    .collect::<Vec<_>>();
                stmt.execute(rusqlite::params_from_iter(params.iter()))
                    .map_err(|e| IoError::Sql(format!("insert row {row_idx} failed: {e}")))?;
            }
        }

        tx.commit()
            .map_err(|e| IoError::Sql(format!("commit failed: {e}")))?;
        Ok(())
    }

    fn dtype_sql(&self, dtype: DType) -> &'static str {
        dtype_to_sql(dtype)
    }

    fn index_dtype_sql(&self, index: &Index) -> &'static str {
        sql_dtype_from_index(index)
    }

    // br-frankenpandas-6dtf: backend-capability + dialect probes.
    fn dialect_name(&self) -> &'static str {
        "sqlite"
    }

    fn supports_returning(&self) -> bool {
        // SQLite 3.35.0+ (released March 2021) supports INSERT ... RETURNING.
        // rusqlite ships with bundled SQLite >= 3.45 by default, so we can
        // unconditionally claim support here.
        true
    }

    fn max_param_count(&self) -> Option<usize> {
        // SQLite default SQLITE_MAX_VARIABLE_NUMBER is 32766 since 3.32.0.
        // (Older builds capped at 999.) rusqlite bundled SQLite is current,
        // so this matches.
        Some(32766)
    }

    fn with_transaction<T, F>(&self, f: F) -> Result<T, IoError>
    where
        F: FnOnce(&Self) -> Result<T, IoError>,
        Self: Sized,
    {
        // rusqlite's pure-trait `Self: Sized` constraint means we operate on
        // `&rusqlite::Connection` directly without taking the `&mut` that
        // `Connection::transaction()` requires. We emulate the same
        // BEGIN/COMMIT semantics with explicit pragmas; on closure error,
        // ROLLBACK and propagate.
        self.execute_batch("BEGIN")
            .map_err(|e| IoError::Sql(format!("begin transaction failed: {e}")))?;
        match f(self) {
            Ok(result) => {
                self.execute_batch("COMMIT")
                    .map_err(|e| IoError::Sql(format!("commit transaction failed: {e}")))?;
                Ok(result)
            }
            Err(err) => {
                // Best-effort rollback; surface the original error if rollback
                // also fails (rollback failure is logged via the Sql error
                // variant for diagnostics but the user wants the closure
                // error preserved as the primary signal).
                let _ = self.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    fn quote_identifier(&self, ident: &str) -> Result<String, IoError> {
        // SQLite accepts ANSI double-quotes for identifiers (it ALSO accepts
        // backticks for MySQL compat, but ANSI is the recommended form per
        // SQLite docs). Delegate to the existing free-fn helper to keep the
        // exact escaping policy in one place.
        quote_sql_ident(ident)
    }

    fn list_tables(&self, _schema: Option<&str>) -> Result<Vec<String>, IoError> {
        // SQLite has a single namespace; the schema arg is silently
        // ignored to match `supports_schemas() == false`. We exclude
        // SQLite's internal `sqlite_*` book-keeping tables to match
        // pandas' SQLAlchemy dialect, which never surfaces them as
        // user tables.
        // Per fd90.50: ESCAPE '\' makes the `_` in 'sqlite\_%' a literal
        // underscore instead of a SQL LIKE single-char wildcard. Without
        // the escape, a user table named e.g. `sqliteX` would be
        // incorrectly excluded because the `_` matches any single char.
        let mut stmt = self
            .prepare(
                r"SELECT name FROM sqlite_master
                 WHERE type='table' AND name NOT LIKE 'sqlite\_%' ESCAPE '\'
                 ORDER BY name",
            )
            .map_err(|e| IoError::Sql(format!("list_tables prepare failed: {e}")))?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| IoError::Sql(format!("list_tables query failed: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| IoError::Sql(format!("list_tables row read failed: {e}")))?;
        Ok(names)
    }

    fn list_views(&self, _schema: Option<&str>) -> Result<Vec<String>, IoError> {
        // Same single-namespace policy as list_tables; type='view'
        // distinguishes the two buckets in sqlite_master.
        // Per fd90.50: same ESCAPE '\' fix as list_tables to treat
        // the underscore in 'sqlite_' as a literal.
        let mut stmt = self
            .prepare(
                r"SELECT name FROM sqlite_master
                 WHERE type='view' AND name NOT LIKE 'sqlite\_%' ESCAPE '\'
                 ORDER BY name",
            )
            .map_err(|e| IoError::Sql(format!("list_views prepare failed: {e}")))?;
        let names = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| IoError::Sql(format!("list_views query failed: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| IoError::Sql(format!("list_views row read failed: {e}")))?;
        Ok(names)
    }

    fn table_schema(
        &self,
        table_name: &str,
        _schema: Option<&str>,
    ) -> Result<Option<SqlTableSchema>, IoError> {
        // Validate the table name first — PRAGMA table_info doesn't
        // accept parameter binding, so we must reject anything that
        // could break out of the identifier slot.
        validate_sql_table_name(table_name)?;
        // PRAGMA table_info returns: cid, name, type, notnull, dflt_value, pk.
        let pragma = format!("PRAGMA table_info(\"{}\")", table_name.replace('"', "\"\""));
        let mut stmt = self
            .prepare(&pragma)
            .map_err(|e| IoError::Sql(format!("table_schema prepare failed: {e}")))?;
        // PRAGMA table_info row tuple: (name, type, notnull, dflt_value, pk).
        // Type alias keeps clippy::type_complexity happy on the
        // intermediate Vec used for the two-pass autoincrement detection.
        type ColumnInfoRow = (String, Option<String>, i64, Option<String>, i64);
        let raw_rows: Vec<ColumnInfoRow> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|e| IoError::Sql(format!("table_schema query failed: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| IoError::Sql(format!("table_schema row read failed: {e}")))?;

        // Per fd90.42 (refines fd90.37): SQLite's rowid-alias rule
        // requires the column to be the SOLE primary key — i.e. exactly
        // one row in PRAGMA table_info has pk > 0. Composite PKs (where
        // multiple columns have pk > 0) never qualify, even if the
        // first column is INTEGER. So we count single-PK status across
        // the table before deciding any column's autoincrement bit.
        let pk_count = raw_rows.iter().filter(|(_, _, _, _, pk)| *pk > 0).count();
        let single_pk = pk_count == 1;

        let mut columns: Vec<SqlColumnSchema> = Vec::with_capacity(raw_rows.len());
        for (name, declared, notnull, dflt, pk) in raw_rows {
            let cleaned_type = declared.filter(|s| !s.is_empty());
            let autoincrement = single_pk
                && pk == 1
                && cleaned_type
                    .as_deref()
                    .map(|t| t.eq_ignore_ascii_case("INTEGER"))
                    .unwrap_or(false);
            columns.push(SqlColumnSchema {
                name,
                declared_type: cleaned_type,
                nullable: notnull == 0,
                default_value: dflt,
                primary_key_ordinal: if pk > 0 {
                    Some(usize::try_from(pk - 1).unwrap_or(0))
                } else {
                    None
                },
                comment: None,
                autoincrement,
            });
        }
        if columns.is_empty() {
            // PRAGMA table_info on a non-existent table returns 0 rows
            // without erroring; map that to None so callers can
            // distinguish missing tables from empty ones.
            Ok(None)
        } else {
            Ok(Some(SqlTableSchema {
                table_name: table_name.to_owned(),
                columns,
            }))
        }
    }

    fn server_version(&self) -> Result<Option<String>, IoError> {
        // sqlite_version() is a built-in scalar that returns the
        // SQLite library version string (e.g. "3.45.1").
        let version: String = self
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .map_err(|e| IoError::Sql(format!("server_version query failed: {e}")))?;
        Ok(Some(version))
    }

    fn list_indexes(
        &self,
        table_name: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<SqlIndexSchema>, IoError> {
        validate_sql_table_name(table_name)?;
        // PRAGMA index_list(table) returns: seq, name, unique, origin, partial.
        // origin is 'c' for CREATE INDEX (user), 'pk' for PRIMARY KEY auto,
        // 'u' for UNIQUE constraint auto. SQLAlchemy.Inspector surfaces
        // only the user-created ones, so we filter out 'pk' to match.
        let pragma_list = format!(
            "PRAGMA index_list(\"{}\")",
            table_name.replace('"', "\"\"")
        );
        let mut list_stmt = self
            .prepare(&pragma_list)
            .map_err(|e| IoError::Sql(format!("list_indexes prepare failed: {e}")))?;
        let index_meta = list_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?, // name
                    row.get::<_, i64>(2)?,    // unique flag
                    row.get::<_, String>(3)?, // origin
                ))
            })
            .map_err(|e| IoError::Sql(format!("list_indexes query failed: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| IoError::Sql(format!("list_indexes row read failed: {e}")))?;

        let mut indexes = Vec::new();
        for (name, uniq, origin) in index_meta {
            if origin == "pk" {
                // Auto-created PK index — pandas/SQLAlchemy hide it.
                continue;
            }
            if origin == "u" {
                // Auto-created index backing a declared UNIQUE
                // constraint — surfaced via list_unique_constraints
                // (fd90.31), not here, to match SQLAlchemy disjoint
                // bucketing between get_indexes and
                // get_unique_constraints.
                continue;
            }
            // PRAGMA index_info(idx) returns: seqno, cid, column_name (col2 may
            // be NULL for expression-based indexes — skip those rather than
            // surfacing partial column lists).
            let pragma_info = format!(
                "PRAGMA index_info(\"{}\")",
                name.replace('"', "\"\"")
            );
            let mut info_stmt = self
                .prepare(&pragma_info)
                .map_err(|e| IoError::Sql(format!("index_info prepare failed: {e}")))?;
            let cols = info_stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(2)?))
                })
                .map_err(|e| IoError::Sql(format!("index_info query failed: {e}")))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| IoError::Sql(format!("index_info row read failed: {e}")))?;
            // Skip expression-based indexes (any column_name is NULL).
            if cols.iter().any(|(_, c)| c.is_none()) {
                continue;
            }
            let mut sorted: Vec<(i64, String)> = cols
                .into_iter()
                .map(|(seq, c)| (seq, c.unwrap_or_default()))
                .collect();
            sorted.sort_by_key(|(seq, _)| *seq);
            indexes.push(SqlIndexSchema {
                name,
                columns: sorted.into_iter().map(|(_, c)| c).collect(),
                unique: uniq != 0,
            });
        }
        Ok(indexes)
    }

    fn list_unique_constraints(
        &self,
        table_name: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<SqlUniqueConstraintSchema>, IoError> {
        validate_sql_table_name(table_name)?;
        // PRAGMA index_list(table) origin column:
        //   'c' = CREATE INDEX (user) — surfaces via list_indexes
        //   'u' = UNIQUE constraint   — surfaces here
        //   'pk' = PRIMARY KEY auto   — surfaces via primary_key_columns
        let pragma_list = format!(
            "PRAGMA index_list(\"{}\")",
            table_name.replace('"', "\"\"")
        );
        let mut list_stmt = self
            .prepare(&pragma_list)
            .map_err(|e| IoError::Sql(format!("list_unique_constraints prepare failed: {e}")))?;
        let candidates = list_stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?, // index name
                    row.get::<_, String>(3)?, // origin
                ))
            })
            .map_err(|e| IoError::Sql(format!("list_unique_constraints query failed: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| IoError::Sql(format!("list_unique_constraints row read failed: {e}")))?;

        let mut constraints = Vec::new();
        for (name, origin) in candidates {
            if origin != "u" {
                continue;
            }
            let pragma_info =
                format!("PRAGMA index_info(\"{}\")", name.replace('"', "\"\""));
            let mut info_stmt = self
                .prepare(&pragma_info)
                .map_err(|e| IoError::Sql(format!("uq index_info prepare failed: {e}")))?;
            let cols = info_stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(2)?))
                })
                .map_err(|e| IoError::Sql(format!("uq index_info query failed: {e}")))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| IoError::Sql(format!("uq index_info row read failed: {e}")))?;
            // Skip expression-based unique constraints (column NULL).
            if cols.iter().any(|(_, c)| c.is_none()) {
                continue;
            }
            let mut sorted: Vec<(i64, String)> = cols
                .into_iter()
                .map(|(seq, c)| (seq, c.unwrap_or_default()))
                .collect();
            sorted.sort_by_key(|(seq, _)| *seq);
            constraints.push(SqlUniqueConstraintSchema {
                name,
                columns: sorted.into_iter().map(|(_, c)| c).collect(),
            });
        }
        Ok(constraints)
    }

    fn list_foreign_keys(
        &self,
        table_name: &str,
        _schema: Option<&str>,
    ) -> Result<Vec<SqlForeignKeySchema>, IoError> {
        // PRAGMA foreign_key_list rows: (seq, referenced_table, from_col, to_col).
        // The constraint id is the BTreeMap key; we don't repeat it inside the
        // value tuple. Type alias keeps clippy::type_complexity happy and
        // the grouping logic readable.
        type FkRow = (i64, String, String, Option<String>);

        validate_sql_table_name(table_name)?;
        // PRAGMA foreign_key_list(table) returns: id, seq, table, from, to,
        // on_update, on_delete, match. Each `id` is one FK constraint;
        // multiple rows with the same id describe a composite FK.
        let pragma = format!(
            "PRAGMA foreign_key_list(\"{}\")",
            table_name.replace('"', "\"\"")
        );
        let mut stmt = self
            .prepare(&pragma)
            .map_err(|e| IoError::Sql(format!("list_foreign_keys prepare failed: {e}")))?;
        let rows: Vec<(i64, FkRow)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?, // id
                    (
                        row.get::<_, i64>(1)?,            // seq
                        row.get::<_, String>(2)?,         // referenced table
                        row.get::<_, String>(3)?,         // from column
                        row.get::<_, Option<String>>(4)?, // to column (nullable)
                    ),
                ))
            })
            .map_err(|e| IoError::Sql(format!("list_foreign_keys query failed: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| IoError::Sql(format!("list_foreign_keys row read failed: {e}")))?;

        // Group by id; preserve discovery order across distinct ids.
        let mut order: Vec<i64> = Vec::new();
        let mut grouped: std::collections::BTreeMap<i64, Vec<FkRow>> =
            std::collections::BTreeMap::new();
        for (id, fk_row) in rows {
            let (seq, ref_table, from_col, to_col) = fk_row;
            if !grouped.contains_key(&id) {
                order.push(id);
            }
            grouped.entry(id).or_default().push((seq, ref_table, from_col, to_col));
        }

        let mut fks = Vec::with_capacity(order.len());
        for id in order {
            let mut group = grouped.remove(&id).unwrap_or_default();
            group.sort_by_key(|(seq, _, _, _)| *seq);
            let ref_table = group
                .first()
                .map(|(_, t, _, _)| t.clone())
                .unwrap_or_default();
            let mut columns = Vec::with_capacity(group.len());
            let mut referenced_columns: Vec<Option<String>> =
                Vec::with_capacity(group.len());
            for (_, _, from_col, to_col) in group {
                columns.push(from_col);
                referenced_columns.push(to_col);
            }
            // Per fd90.44: when ALL `to` columns are NULL, the user
            // declared `FOREIGN KEY (cols) REFERENCES parent` (implicit
            // reference to parent's PK). Resolve by looking up the
            // parent's primary key columns. SQLAlchemy.Inspector
            // surfaces these as resolved-to-PK references; matching
            // that behavior keeps callers from missing real FKs.
            let resolved_columns: Vec<String> = if referenced_columns
                .iter()
                .all(Option::is_none)
            {
                // Implicit-PK reference: look up parent's PK.
                let pk = self.primary_key_columns(&ref_table, None)?;
                if pk.len() == columns.len() {
                    pk
                } else {
                    // Parent PK shape doesn't match FK column count
                    // (parent has no PK, or composite mismatch). Skip
                    // — fabricating columns would mislead callers
                    // worse than hiding the FK.
                    continue;
                }
            } else if referenced_columns.iter().all(Option::is_some) {
                // Fully explicit: every column has a resolved 'to'.
                referenced_columns.into_iter().flatten().collect()
            } else {
                // Mixed Some/None: SQLite shouldn't produce this for
                // a single FK group, but if it ever does, skip rather
                // than mispair.
                continue;
            };
            fks.push(SqlForeignKeySchema {
                // SQLite PRAGMA foreign_key_list does not surface a
                // CONSTRAINT name; pandas/SQLAlchemy report None there too.
                constraint_name: None,
                columns,
                referenced_table: ref_table,
                referenced_columns: resolved_columns,
            });
        }
        Ok(fks)
    }
}

#[cfg(feature = "sql-sqlite")]
fn sql_dtype_from_index(index: &Index) -> &'static str {
    for label in index.labels() {
        match label {
            IndexLabel::Int64(_) => return "INTEGER",
            IndexLabel::Utf8(_) => return "TEXT",
            IndexLabel::Timedelta64(v) if *v != Timedelta::NAT => return "INTEGER",
            IndexLabel::Datetime64(v) if *v != i64::MIN => return "TEXT",
            _ => {}
        }
    }
    "TEXT"
}

fn resolve_sql_index_label(
    frame: &DataFrame,
    options: &SqlWriteOptions,
) -> Result<Option<String>, IoError> {
    if !options.index {
        return Ok(None);
    }

    let label = options
        .index_label
        .clone()
        .or_else(|| frame.index().name().map(str::to_owned))
        .unwrap_or_else(|| "index".to_owned());

    if frame.column(&label).is_some() {
        return Err(IoError::DuplicateColumnName(label));
    }

    Ok(Some(label))
}

// Per br-frankenpandas-ld8h (fd90.45): these helpers are only called
// from the rusqlite SqlConnection impl, which is gated behind
// `feature = "sql-sqlite"`. Mirroring the gate here keeps
// --no-default-features builds clean of dead-code warnings.
#[cfg(feature = "sql-sqlite")]
fn escape_sql_ident(name: &str) -> Result<String, IoError> {
    if name.contains('\0') {
        return Err(IoError::Sql("invalid SQL identifier: NUL byte".to_owned()));
    }
    Ok(name.replace('"', "\"\""))
}

#[cfg(feature = "sql-sqlite")]
fn quote_sql_ident(name: &str) -> Result<String, IoError> {
    Ok(format!("\"{}\"", escape_sql_ident(name)?))
}

/// Per br-frankenpandas-4l7a (fd90.55): shared identifier-shape
/// validator used by `validate_sql_table_name` and
/// `validate_sql_column_name`. `kind` is the user-facing label
/// inserted into the error message ("table", "column", ...). The
/// rule is the same for both: non-empty, ASCII-alphanumeric or
/// underscore only — defense in depth alongside `quote_identifier`
/// (which handles embedded quotes but doesn't reject other shapes).
fn validate_sql_ident(name: &str, kind: &str) -> Result<(), IoError> {
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(IoError::Sql(format!(
            "invalid {kind} name: '{name}' (must be non-empty, only alphanumeric and underscore allowed)"
        )));
    }
    Ok(())
}

fn validate_sql_table_name(table_name: &str) -> Result<(), IoError> {
    validate_sql_ident(table_name, "table")
}

/// Per br-frankenpandas-597l (fd90.56): dedicated schema-name
/// validator so error messages correctly identify the invalid
/// identifier as a schema rather than a table. Same alphanumeric+
/// underscore rule as table/column names.
fn validate_sql_schema_name(schema: &str) -> Result<(), IoError> {
    validate_sql_ident(schema, "schema")
}

/// Validate `name` against the backend's identifier-length cap.
///
/// Per br-frankenpandas-9ynk (fd90.27). When `max` is `Some(n)`, errors
/// out when `name.len() > n`. When `max` is `None`, accepts any length
/// (e.g. SQLite, where the engine has no documented limit). `kind` is
/// the user-facing label used in the error message ("table", "column",
/// "index label", ...) so misuse points cleanly back to the offending
/// identifier without callers having to format the message.
fn validate_sql_identifier_length(
    name: &str,
    max: Option<usize>,
    kind: &str,
) -> Result<(), IoError> {
    if let Some(limit) = max
        && name.len() > limit
    {
        return Err(IoError::Sql(format!(
            "invalid {kind} name '{name}': length {len} exceeds backend identifier limit ({limit})",
            len = name.len()
        )));
    }
    Ok(())
}

fn sql_select_all_query<C: SqlConnection>(
    conn: &C,
    table_name: &str,
) -> Result<String, IoError> {
    sql_select_all_query_in_schema(conn, table_name, None)
}

/// Build a `SELECT * FROM ...` statement, optionally schema-qualified.
///
/// Per br-frankenpandas-u6zn (fd90.14). When `schema` is `Some(s)` AND
/// `conn.supports_schemas()`, the FROM clause becomes `\"schema\".\"table\"`.
/// When `supports_schemas` returns false, any `Some(s)` is ignored and the
/// bare table name is used — preserves SQLite compatibility (pandas would
/// raise NotImplementedError there; we choose silent fallback so existing
/// SqlReadOptions consumers aren't broken).
fn sql_select_all_query_in_schema<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
) -> Result<String, IoError> {
    validate_sql_table_name(table_name)?;
    let qualified = match schema {
        Some(s) if conn.supports_schemas() => {
            validate_sql_schema_name(s)?;
            format!(
                "{}.{}",
                conn.quote_identifier(s)?,
                conn.quote_identifier(table_name)?
            )
        }
        _ => conn.quote_identifier(table_name)?,
    };
    Ok(format!("SELECT * FROM {qualified}"))
}

fn validate_sql_column_name(column_name: &str) -> Result<(), IoError> {
    validate_sql_ident(column_name, "column")
}

fn sql_select_columns_query<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    columns: &[&str],
) -> Result<String, IoError> {
    sql_select_columns_query_in_schema(conn, table_name, None, columns)
}

/// Build a `SELECT col1, col2, ... FROM ...` statement, optionally
/// schema-qualified.
///
/// Per br-frankenpandas-d3e9 (fd90.34). Companion to
/// `sql_select_all_query_in_schema`. Same schema rules: when
/// `schema` is `Some(s)` AND `conn.supports_schemas()`, the FROM
/// clause becomes `\"schema\".\"table\"`; otherwise the bare table
/// name is used.
fn sql_select_columns_query_in_schema<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
    columns: &[&str],
) -> Result<String, IoError> {
    validate_sql_table_name(table_name)?;
    if columns.is_empty() {
        return Err(IoError::Sql(
            "read_sql_table_columns: columns must be non-empty".to_owned(),
        ));
    }
    for name in columns {
        validate_sql_column_name(name)?;
    }

    let qualified = match schema {
        Some(s) if conn.supports_schemas() => {
            validate_sql_schema_name(s)?;
            format!(
                "{}.{}",
                conn.quote_identifier(s)?,
                conn.quote_identifier(table_name)?
            )
        }
        _ => conn.quote_identifier(table_name)?,
    };
    let projection: Vec<String> = columns
        .iter()
        .map(|name| conn.quote_identifier(name))
        .collect::<Result<_, _>>()?;
    Ok(format!(
        "SELECT {} FROM {}",
        projection.join(", "),
        qualified
    ))
}

fn sql_column_definition<C: SqlConnection>(
    conn: &C,
    column_name: &str,
    sql_type: &str,
) -> Result<String, IoError> {
    Ok(format!(
        "{} {sql_type}",
        conn.quote_identifier(column_name)?
    ))
}

#[cfg(test)]
fn sql_create_table_query<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    column_defs: &[String],
) -> Result<String, IoError> {
    sql_create_table_query_in_schema(conn, table_name, None, column_defs)
}

/// Build a `CREATE TABLE IF NOT EXISTS ...` statement, optionally
/// schema-qualified.
///
/// Per br-frankenpandas-udn6 (fd90.15). When `schema` is `Some(s)` AND
/// `conn.supports_schemas()`, the target becomes `"schema"."table"`. On
/// backends that report false, any `Some(s)` is silently ignored.
fn sql_create_table_query_in_schema<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
    column_defs: &[String],
) -> Result<String, IoError> {
    validate_sql_table_name(table_name)?;
    let qualified = match schema {
        Some(s) if conn.supports_schemas() => {
            validate_sql_schema_name(s)?;
            format!(
                "{}.{}",
                conn.quote_identifier(s)?,
                conn.quote_identifier(table_name)?
            )
        }
        _ => conn.quote_identifier(table_name)?,
    };
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {qualified} ({})",
        column_defs.join(", ")
    ))
}

#[cfg(test)]
fn sql_insert_rows_query<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    column_names: &[String],
) -> Result<String, IoError> {
    sql_insert_rows_query_in_schema(conn, table_name, None, column_names)
}

/// Build an `INSERT INTO ... VALUES (...)` statement, optionally
/// schema-qualified.
///
/// Per br-frankenpandas-udn6 (fd90.15). Same schema rules as the CREATE
/// TABLE counterpart.
fn sql_insert_rows_query_in_schema<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
    column_names: &[String],
) -> Result<String, IoError> {
    validate_sql_table_name(table_name)?;
    let qualified = match schema {
        Some(s) if conn.supports_schemas() => {
            validate_sql_schema_name(s)?;
            format!(
                "{}.{}",
                conn.quote_identifier(s)?,
                conn.quote_identifier(table_name)?
            )
        }
        _ => conn.quote_identifier(table_name)?,
    };
    let quoted_columns = column_names
        .iter()
        .map(|name| conn.quote_identifier(name))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let placeholders = (1..=column_names.len())
        .map(|ordinal| conn.parameter_marker(ordinal))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "INSERT INTO {qualified} ({quoted_columns}) VALUES ({placeholders})"
    ))
}

/// Build a multi-row `INSERT INTO ... VALUES (...), (...), ...`
/// statement, optionally schema-qualified.
///
/// Placeholder ordinals span 1..=`num_rows` * `column_names.len()` so
/// PostgreSQL-style `$N` markers stay unique across the whole statement.
/// SQLite's `?N` and the bare `?` default also work because positional
/// binding consumes ordinals in left-to-right order.
///
/// Per br-frankenpandas-i0ml (fd90.19).
fn sql_multi_row_insert_query_in_schema<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
    column_names: &[String],
    num_rows: usize,
) -> Result<String, IoError> {
    validate_sql_table_name(table_name)?;
    if num_rows == 0 || column_names.is_empty() {
        return Err(IoError::Sql(
            "multi-row insert requires at least one row and one column".to_owned(),
        ));
    }
    let qualified = match schema {
        Some(s) if conn.supports_schemas() => {
            validate_sql_schema_name(s)?;
            format!(
                "{}.{}",
                conn.quote_identifier(s)?,
                conn.quote_identifier(table_name)?
            )
        }
        _ => conn.quote_identifier(table_name)?,
    };
    let quoted_columns = column_names
        .iter()
        .map(|name| conn.quote_identifier(name))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let cols = column_names.len();
    let mut tuples = Vec::with_capacity(num_rows);
    let mut next_ord = 1usize;
    for _ in 0..num_rows {
        let row_placeholders = (0..cols)
            .map(|_| {
                let marker = conn.parameter_marker(next_ord);
                next_ord += 1;
                marker
            })
            .collect::<Vec<_>>()
            .join(", ");
        tuples.push(format!("({row_placeholders})"));
    }
    Ok(format!(
        "INSERT INTO {qualified} ({quoted_columns}) VALUES {}",
        tuples.join(", ")
    ))
}

/// Build a `DROP TABLE IF EXISTS ...` statement, optionally
/// schema-qualified.
///
/// Per br-frankenpandas-hxob (fd90.16). Companion to
/// `sql_create_table_query_in_schema`. Same schema rules: when
/// `schema` is `Some(s)` AND `conn.supports_schemas()`, the target
/// becomes `\"schema\".\"table\"`; otherwise the bare table name is
/// used. Routes through `conn.quote_identifier` so backend dialect
/// overrides (MySQL backticks etc.) take effect on the drop path.
fn sql_drop_table_query_in_schema<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
) -> Result<String, IoError> {
    validate_sql_table_name(table_name)?;
    let qualified = match schema {
        Some(s) if conn.supports_schemas() => {
            validate_sql_schema_name(s)?;
            format!(
                "{}.{}",
                conn.quote_identifier(s)?,
                conn.quote_identifier(table_name)?
            )
        }
        _ => conn.quote_identifier(table_name)?,
    };
    Ok(format!("DROP TABLE IF EXISTS {qualified}"))
}

/// Read the result of a SQL query into a DataFrame.
///
/// Matches `pd.read_sql(sql, con)`.
pub fn read_sql<C: SqlConnection>(conn: &C, query: &str) -> Result<DataFrame, IoError> {
    read_sql_with_options(conn, query, &SqlReadOptions::default())
}

/// Read the result of a SQL query into a DataFrame with read-time options.
///
/// Matches the supported subset of `pd.read_sql(sql, con, params=[...], parse_dates=...)`.
pub fn read_sql_with_options<C: SqlConnection>(
    conn: &C,
    query: &str,
    options: &SqlReadOptions,
) -> Result<DataFrame, IoError> {
    let (headers, columns) = sql_query_to_columns(conn, query, options)?;
    let frame = dataframe_from_sql_columns(headers, columns)?;
    apply_sql_index_col(frame, options.index_col.as_deref())
}

/// Per br-frankenpandas-c1h9 (fd90.36): promote `options.index_col`
/// to the DataFrame index when set, with empty-string rejection.
fn apply_sql_index_col(
    frame: DataFrame,
    index_col: Option<&str>,
) -> Result<DataFrame, IoError> {
    let Some(name) = index_col else {
        return Ok(frame);
    };
    if name.is_empty() {
        return Err(IoError::Sql(
            "index_col: empty string is not a valid column name".to_owned(),
        ));
    }
    promote_column_to_index(&frame, name)
}

/// Read the result of a SQL query into a DataFrame.
///
/// Matches `pd.read_sql_query(sql, con)`. This is the query-only spelling of
/// `read_sql`; table-name dispatch stays on `read_sql_table`.
pub fn read_sql_query<C: SqlConnection>(conn: &C, query: &str) -> Result<DataFrame, IoError> {
    read_sql(conn, query)
}

/// Read the result of a SQL query into a DataFrame with read-time options.
///
/// Matches the supported subset of
/// `pd.read_sql_query(sql, con, params=[...], parse_dates=..., coerce_float=...)`.
pub fn read_sql_query_with_options<C: SqlConnection>(
    conn: &C,
    query: &str,
    options: &SqlReadOptions,
) -> Result<DataFrame, IoError> {
    read_sql_with_options(conn, query, options)
}

/// Read the result of a SQL query as an iterator of DataFrame chunks.
///
/// Matches the supported subset of `pd.read_sql_query(sql, con, chunksize=...)`.
pub fn read_sql_query_chunks<C: SqlConnection>(
    conn: &C,
    query: &str,
    chunk_size: usize,
) -> Result<SqlChunkIterator, IoError> {
    read_sql_chunks(conn, query, chunk_size)
}

/// Read a SQL query result as chunks with one column promoted to each chunk's index.
///
/// Matches the supported subset of
/// `pd.read_sql_query(sql, con, index_col=..., chunksize=...)`.
pub fn read_sql_query_chunks_with_index_col<C: SqlConnection>(
    conn: &C,
    query: &str,
    index_col: Option<&str>,
    chunk_size: usize,
) -> Result<SqlIndexedChunkIterator, IoError> {
    read_sql_chunks_with_index_col(conn, query, index_col, chunk_size)
}

/// Read the result of a SQL query as chunks with read-time options.
///
/// Matches the supported subset of
/// `pd.read_sql_query(sql, con, params=[...], parse_dates=..., coerce_float=..., chunksize=...)`.
pub fn read_sql_query_chunks_with_options<C: SqlConnection>(
    conn: &C,
    query: &str,
    options: &SqlReadOptions,
    chunk_size: usize,
) -> Result<SqlChunkIterator, IoError> {
    read_sql_chunks_with_options(conn, query, options, chunk_size)
}

/// Read a SQL query result as chunks with read-time options and index promotion.
///
/// Matches the supported subset of
/// `pd.read_sql_query(sql, con, params=[...], parse_dates=..., coerce_float=..., index_col=..., chunksize=...)`.
pub fn read_sql_query_chunks_with_options_and_index_col<C: SqlConnection>(
    conn: &C,
    query: &str,
    options: &SqlReadOptions,
    index_col: Option<&str>,
    chunk_size: usize,
) -> Result<SqlIndexedChunkIterator, IoError> {
    read_sql_chunks_with_options_and_index_col(conn, query, options, index_col, chunk_size)
}

/// Read a SQL query result with one column promoted to the index.
///
/// Matches `pd.read_sql_query(sql, con, index_col=...)`.
pub fn read_sql_query_with_index_col<C: SqlConnection>(
    conn: &C,
    query: &str,
    index_col: Option<&str>,
) -> Result<DataFrame, IoError> {
    read_sql_with_index_col(conn, query, index_col)
}

fn sql_query_to_columns<C: SqlConnection>(
    conn: &C,
    query: &str,
    options: &SqlReadOptions,
) -> Result<(Vec<String>, Vec<Vec<Scalar>>), IoError> {
    let SqlQueryResult {
        columns: headers,
        rows,
    } = conn.query(query, options.params.as_deref().unwrap_or(&[]))?;
    reject_duplicate_headers(&headers)?;
    let mut columns: Vec<Vec<Scalar>> = (0..headers.len()).map(|_| Vec::new()).collect();

    for row in rows {
        for (col_idx, value) in row.into_iter().enumerate() {
            if let Some(col_vec) = columns.get_mut(col_idx) {
                col_vec.push(value);
            }
        }
    }

    if let Some(ref parse_dates) = options.parse_dates {
        apply_parse_dates(&headers, &mut columns, parse_dates)?;
    }
    if options.coerce_float {
        apply_sql_coerce_float(&mut columns);
    }
    if let Some(ref dtype_map) = options.dtype {
        apply_sql_dtype_overrides(
            &headers,
            &mut columns,
            dtype_map,
            options.parse_dates.as_deref().unwrap_or(&[]),
        )?;
    }

    Ok((headers, columns))
}

/// Apply pandas-style `dtype={'col': dtype}` overrides to materialized
/// SQL result columns. Skips columns also listed in `parse_dates` to
/// avoid double-cast errors. Per br-frankenpandas-l9pt (fd90.11).
fn apply_sql_dtype_overrides(
    headers: &[String],
    columns: &mut [Vec<Scalar>],
    dtype_map: &BTreeMap<String, DType>,
    parse_dates: &[String],
) -> Result<(), IoError> {
    for (idx, header) in headers.iter().enumerate() {
        let Some(target_dtype) = dtype_map.get(header) else {
            continue;
        };
        if parse_dates.iter().any(|d| d == header) {
            // parse_dates wins; skip dtype override for this column.
            continue;
        }
        let Some(col) = columns.get_mut(idx) else {
            continue;
        };
        for value in col.iter_mut() {
            // Take ownership of the scalar, cast, and write back. NaT/Null
            // pass through cast_scalar_owned unchanged so missingness is
            // preserved across the override.
            let taken = std::mem::replace(value, Scalar::Null(NullKind::Null));
            *value = cast_scalar_owned(taken, *target_dtype).map_err(|e| {
                IoError::Sql(format!(
                    "dtype override on column '{header}' to {target_dtype:?} failed: {e}"
                ))
            })?;
        }
    }
    Ok(())
}

fn dataframe_from_sql_columns(
    headers: Vec<String>,
    columns: Vec<Vec<Scalar>>,
) -> Result<DataFrame, IoError> {
    let row_count = columns.first().map_or(0, Vec::len);
    let mut out_columns = BTreeMap::new();
    let mut column_order = Vec::new();

    for (name, values) in headers.into_iter().zip(columns) {
        out_columns.insert(name.clone(), Column::from_values(values)?);
        column_order.push(name);
    }

    let index = Index::from_i64((0..row_count as i64).collect());
    Ok(DataFrame::new_with_column_order(
        index,
        out_columns,
        column_order,
    )?)
}

/// Read a SQL query result as an iterator of DataFrame chunks.
///
/// Matches the supported subset of `pd.read_sql(sql, con, chunksize=...)`.
/// Each chunk receives a fresh zero-based RangeIndex, matching pandas'
/// SQLite chunk iterator behavior.
pub fn read_sql_chunks<C: SqlConnection>(
    conn: &C,
    query: &str,
    chunk_size: usize,
) -> Result<SqlChunkIterator, IoError> {
    read_sql_chunks_with_options(conn, query, &SqlReadOptions::default(), chunk_size)
}

/// Read a SQL query result as DataFrame chunks with read-time options.
///
/// `params`, `parse_dates`, and `coerce_float` are applied before chunking so
/// chunked reads match the corresponding full-frame `read_sql_with_options`
/// result when concatenated row-wise.
pub fn read_sql_chunks_with_options<C: SqlConnection>(
    conn: &C,
    query: &str,
    options: &SqlReadOptions,
    chunk_size: usize,
) -> Result<SqlChunkIterator, IoError> {
    if chunk_size == 0 {
        return Err(IoError::Sql(
            "read_sql chunksize must be greater than zero".to_owned(),
        ));
    }

    let (headers, columns) = sql_query_to_columns(conn, query, options)?;
    Ok(SqlChunkIterator {
        headers,
        columns,
        chunk_size,
        next_row: 0,
    })
}

/// Read a SQL query result as DataFrame chunks with read-time options and optional index promotion.
///
/// `params`, `parse_dates`, and `coerce_float` are applied before chunking and
/// before index promotion so indexed chunk output matches the corresponding
/// full-frame `read_sql_with_options` result when concatenated row-wise.
pub fn read_sql_chunks_with_options_and_index_col<C: SqlConnection>(
    conn: &C,
    query: &str,
    options: &SqlReadOptions,
    index_col: Option<&str>,
    chunk_size: usize,
) -> Result<SqlIndexedChunkIterator, IoError> {
    let inner = read_sql_chunks_with_options(conn, query, options, chunk_size)?;
    sql_indexed_chunks(inner, index_col)
}

/// Read a SQL query result as DataFrame chunks with optional index promotion.
///
/// Matches the supported subset of `pd.read_sql(sql, con, index_col=..., chunksize=...)`.
pub fn read_sql_chunks_with_index_col<C: SqlConnection>(
    conn: &C,
    query: &str,
    index_col: Option<&str>,
    chunk_size: usize,
) -> Result<SqlIndexedChunkIterator, IoError> {
    let inner = read_sql_chunks(conn, query, chunk_size)?;
    sql_indexed_chunks(inner, index_col)
}

/// Read a SQL query result with one column promoted to the index.
///
/// Matches `pd.read_sql(sql, con, index_col=...)`. When
/// `index_col=Some(name)` the named column is removed from the data
/// columns and its values become the DataFrame's row index. Returns
/// `IoError::Sql` if the named column is absent from the result set.
pub fn read_sql_with_index_col<C: SqlConnection>(
    conn: &C,
    query: &str,
    index_col: Option<&str>,
) -> Result<DataFrame, IoError> {
    let frame = read_sql(conn, query)?;
    let Some(col_name) = index_col else {
        return Ok(frame);
    };
    promote_column_to_index(&frame, col_name)
}

/// Read an entire SQL table with one column promoted to the index.
///
/// Matches `pd.read_sql_table(table, con, index_col=...)`.
pub fn read_sql_table_with_index_col<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    index_col: Option<&str>,
) -> Result<DataFrame, IoError> {
    let frame = read_sql_table(conn, table_name)?;
    let Some(col_name) = index_col else {
        return Ok(frame);
    };
    promote_column_to_index(&frame, col_name)
}

fn promote_column_to_index(frame: &DataFrame, col_name: &str) -> Result<DataFrame, IoError> {
    let column = frame.column(col_name).ok_or_else(|| {
        IoError::Sql(format!(
            "index_col {col_name:?} not present in result columns"
        ))
    })?;
    let labels: Vec<IndexLabel> = column
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Int64(i) => IndexLabel::Int64(*i),
            Scalar::Utf8(s) => IndexLabel::Utf8(s.clone()),
            Scalar::Float64(f) if !f.is_nan() => IndexLabel::Utf8(f.to_string()),
            Scalar::Bool(b) => IndexLabel::Utf8(b.to_string()),
            Scalar::Timedelta64(ns) => IndexLabel::Timedelta64(*ns),
            _ => IndexLabel::Utf8("NaN".to_owned()),
        })
        .collect();
    let new_index = Index::new(labels).set_name(col_name);

    let mut new_columns = std::collections::BTreeMap::new();
    let mut new_order = Vec::new();
    for name in frame.column_names() {
        if name == col_name {
            continue;
        }
        if let Some(col) = frame.column(name) {
            new_columns.insert(name.clone(), col.clone());
            new_order.push(name.clone());
        }
    }

    Ok(DataFrame::new_with_column_order(
        new_index,
        new_columns,
        new_order,
    )?)
}

/// Read an entire SQL table into a DataFrame.
///
/// Matches `pd.read_sql_table(table_name, con)`.
pub fn read_sql_table<C: SqlConnection>(conn: &C, table_name: &str) -> Result<DataFrame, IoError> {
    read_sql(conn, &sql_select_all_query(conn, table_name)?)
}

/// List user-visible table names known to the SQL backend.
///
/// Matches the supported subset of
/// `pd.io.sql.SQLDatabase.list_tables(schema=...)`. When the backend
/// reports `supports_schemas() == false` (SQLite), `schema` is ignored
/// and all tables in the single namespace are returned. When the
/// backend supports schemas (PostgreSQL, MySQL, MSSQL), `Some(s)`
/// scopes the listing. `None` passes through to the backend
/// unchanged — backends MAY consult `default_schema()` for their own
/// fallback logic if desired (per fd90.57: this wrapper does NOT
/// apply the fallback automatically).
///
/// Per br-frankenpandas-vhq2 (fd90.20).
pub fn list_sql_tables<C: SqlConnection>(
    conn: &C,
    schema: Option<&str>,
) -> Result<Vec<String>, IoError> {
    conn.list_tables(schema)
}

/// Introspect a SQL table's column metadata, optionally schema-scoped.
///
/// Matches the supported subset of
/// `pd.io.sql.SQLDatabase.has_table` + `SQLAlchemy.MetaData.reflect`
/// for column-level details. Returns `Ok(None)` when the table does
/// not exist. Schema arg is silently ignored when the backend reports
/// `supports_schemas() == false` (SQLite).
///
/// Per br-frankenpandas-w43q (fd90.21).
pub fn sql_table_schema<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Option<SqlTableSchema>, IoError> {
    conn.table_schema(table_name, schema)
}

/// List user-visible schemas exposed by the SQL backend.
///
/// Matches `SQLAlchemy.Inspector.get_schema_names()` shape. Single
/// namespace backends (SQLite) return an empty vector. Multi-schema
/// backends return the schemas the connection's role can see, with
/// internal/system schemas filtered out.
///
/// Per br-frankenpandas-lxhi (fd90.22).
pub fn list_sql_schemas<C: SqlConnection>(conn: &C) -> Result<Vec<String>, IoError> {
    conn.list_schemas()
}

/// Reset a SQL table to empty without dropping its definition.
///
/// On backends that override the default (PostgreSQL, MySQL), this
/// uses `TRUNCATE TABLE` for a DDL-style fast-path reset. On backends
/// that inherit the default (SQLite), this emits `DELETE FROM <table>`,
/// which is universal but slower on large tables. Schema arg is
/// silently ignored when `supports_schemas() == false`.
///
/// Per br-frankenpandas-phum (fd90.23).
pub fn truncate_sql_table<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
) -> Result<(), IoError> {
    conn.truncate_table(table_name, schema)
}

/// Probe the SQL backend's server version string.
///
/// Returns `Ok(None)` for backends that can't (or don't) introspect
/// their version. SQLite returns `Some("3.x.y")`. PostgreSQL/MySQL
/// impls return their respective `SHOW server_version` /
/// `SELECT VERSION()` payloads. Useful for dialect-version gating
/// (RETURNING, JSON ops, generated columns) and diagnostics.
///
/// Per br-frankenpandas-e23k (fd90.24).
pub fn sql_server_version<C: SqlConnection>(conn: &C) -> Result<Option<String>, IoError> {
    conn.server_version()
}

/// Return the primary-key column names for a SQL table, ordered by
/// the table's primary-key ordinal.
///
/// Returns an empty vector when the table doesn't exist, has no
/// primary key, or the backend can't introspect column metadata.
/// Useful for upsert conflict-target generation and `index_label`
/// defaulting.
///
/// Per br-frankenpandas-uw3y (fd90.25).
pub fn sql_primary_key_columns<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Vec<String>, IoError> {
    conn.primary_key_columns(table_name, schema)
}

/// List user-defined indexes on a SQL table, optionally schema-scoped.
///
/// Matches `SQLAlchemy.Inspector.get_indexes()` shape. Returns an
/// empty vector when the table doesn't exist, has no user-created
/// indexes, or the backend can't introspect. Auto-created PRIMARY-KEY
/// indexes are filtered out (they're surfaced via primary_key_columns
/// instead).
///
/// Per br-frankenpandas-bgv9 (fd90.28).
pub fn list_sql_indexes<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Vec<SqlIndexSchema>, IoError> {
    conn.list_indexes(table_name, schema)
}

/// List user-visible view names known to the SQL backend.
///
/// Matches `SQLAlchemy.Inspector.get_view_names()` shape. Companion
/// to `list_sql_tables` — pandas/SQLAlchemy keep tables and views in
/// distinct buckets so `pd.read_sql_table` can distinguish them.
/// Schema arg is silently ignored when `supports_schemas() == false`.
///
/// Per br-frankenpandas-gm3r (fd90.30).
pub fn list_sql_views<C: SqlConnection>(
    conn: &C,
    schema: Option<&str>,
) -> Result<Vec<String>, IoError> {
    conn.list_views(schema)
}

/// List foreign-key constraints declared on a SQL table, optionally
/// schema-scoped.
///
/// Matches `SQLAlchemy.Inspector.get_foreign_keys()` shape. Returns
/// an empty vector when the table has no FKs or the backend can't
/// introspect. Composite FKs are returned as a single entry with
/// paired `columns` / `referenced_columns` ordered by declaration
/// position. SQLite does not expose constraint names via PRAGMA, so
/// `constraint_name` is `None` there.
///
/// Per br-frankenpandas-uht8 (fd90.29).
pub fn list_sql_foreign_keys<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Vec<SqlForeignKeySchema>, IoError> {
    conn.list_foreign_keys(table_name, schema)
}

/// Probe the table-level comment for a SQL table, optionally
/// schema-scoped.
///
/// Matches `SQLAlchemy.Inspector.get_table_comment()` shape — returns
/// `Ok(Some(text))` when a comment exists, `Ok(None)` otherwise.
/// SQLite has no native table-comment storage and returns `None`.
/// PostgreSQL/MySQL/MSSQL impls override with their respective
/// catalog queries.
///
/// Per br-frankenpandas-yu3w (fd90.32).
pub fn sql_table_comment<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Option<String>, IoError> {
    conn.table_comment(table_name, schema)
}

/// List UNIQUE constraints declared on a SQL table.
///
/// Matches `SQLAlchemy.Inspector.get_unique_constraints()` shape.
/// Surfaces only inline `UNIQUE` declarations and `UNIQUE (...)`
/// table constraints. User-created `CREATE UNIQUE INDEX` indexes
/// remain in `list_sql_indexes` (with `unique == true`). The two
/// listings are intentionally disjoint to match SQLAlchemy.
///
/// Per br-frankenpandas-sh4v (fd90.31).
pub fn list_sql_unique_constraints<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    schema: Option<&str>,
) -> Result<Vec<SqlUniqueConstraintSchema>, IoError> {
    conn.list_unique_constraints(table_name, schema)
}

/// Maximum identifier length supported by the SQL backend, or `None`
/// when no documented limit exists.
///
/// Useful for to_sql validation: backends that override this report
/// their cap (PostgreSQL=63, MySQL=64, MSSQL=128) so auto-generated
/// index / constraint / column names can be truncated or rejected
/// before round-tripping through DDL that would silently truncate.
///
/// Per br-frankenpandas-cs81 (fd90.26).
pub fn sql_max_identifier_length<C: SqlConnection>(conn: &C) -> Option<usize> {
    conn.max_identifier_length()
}

/// Backend-agnostic introspection facade matching the
/// `SQLAlchemy.Inspector` shape.
///
/// Per br-frankenpandas-szs9 (fd90.38). Wraps a `&C: SqlConnection`
/// and exposes the full fd90.20-37 introspection surface as methods
/// on a single bundle so callers don't have to remember which
/// `list_sql_*` / `sql_*` free-fn to use. Pure delegation — no new
/// behavior, just API ergonomics.
///
/// ```rust,ignore
/// use frankenpandas::SqlInspector;
/// let inspector = SqlInspector::new(&conn);
/// for table in inspector.tables(None)? {
///     for col in inspector.columns(&table, None)?
///         .map(|s| s.columns)
///         .unwrap_or_default()
///     {
///         println!("{}: {:?}", col.name, col.declared_type);
///     }
/// }
/// ```
#[derive(Debug)]
pub struct SqlInspector<'a, C: SqlConnection> {
    conn: &'a C,
}

impl<'a, C: SqlConnection> SqlInspector<'a, C> {
    /// Create a new inspector bound to the given connection.
    #[must_use]
    pub fn new(conn: &'a C) -> Self {
        Self { conn }
    }

    /// List user-visible table names. See `list_sql_tables`.
    pub fn tables(&self, schema: Option<&str>) -> Result<Vec<String>, IoError> {
        self.conn.list_tables(schema)
    }

    /// List user-visible view names. See `list_sql_views`.
    pub fn views(&self, schema: Option<&str>) -> Result<Vec<String>, IoError> {
        self.conn.list_views(schema)
    }

    /// List user-visible schemas. See `list_sql_schemas`.
    pub fn schemas(&self) -> Result<Vec<String>, IoError> {
        self.conn.list_schemas()
    }

    /// Introspect a table's columns. See `sql_table_schema`.
    pub fn columns(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<Option<SqlTableSchema>, IoError> {
        self.conn.table_schema(table_name, schema)
    }

    /// List user-defined indexes. See `list_sql_indexes`.
    pub fn indexes(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<Vec<SqlIndexSchema>, IoError> {
        self.conn.list_indexes(table_name, schema)
    }

    /// List foreign-key constraints. See `list_sql_foreign_keys`.
    pub fn foreign_keys(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<Vec<SqlForeignKeySchema>, IoError> {
        self.conn.list_foreign_keys(table_name, schema)
    }

    /// List UNIQUE constraints. See `list_sql_unique_constraints`.
    pub fn unique_constraints(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<Vec<SqlUniqueConstraintSchema>, IoError> {
        self.conn.list_unique_constraints(table_name, schema)
    }

    /// Return primary-key columns sorted by ordinal.
    /// See `sql_primary_key_columns`.
    pub fn primary_key_columns(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<Vec<String>, IoError> {
        self.conn.primary_key_columns(table_name, schema)
    }

    /// Probe the table-level comment. See `sql_table_comment`.
    pub fn table_comment(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<Option<String>, IoError> {
        self.conn.table_comment(table_name, schema)
    }

    /// Schema-aware existence check. Routes to
    /// `SqlConnection::table_exists_in_schema`.
    pub fn table_exists(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<bool, IoError> {
        self.conn.table_exists_in_schema(table_name, schema)
    }

    /// Probe the backend server version. See `sql_server_version`.
    pub fn server_version(&self) -> Result<Option<String>, IoError> {
        self.conn.server_version()
    }

    /// Maximum identifier length, when the backend exposes one.
    /// See `sql_max_identifier_length`.
    #[must_use]
    pub fn max_identifier_length(&self) -> Option<usize> {
        self.conn.max_identifier_length()
    }

    /// Backend dialect name (`"sqlite"`, `"postgresql"`, etc.).
    #[must_use]
    pub fn dialect_name(&self) -> &'static str {
        self.conn.dialect_name()
    }

    /// Check whether a specific column exists on a table.
    ///
    /// Per br-frankenpandas-ppry (fd90.39). Returns `Ok(false)` when
    /// the table doesn't exist (i.e. `columns` returns `None`), or
    /// when the table exists but has no column matching `column_name`.
    /// Returns `Ok(true)` only when the named column is present.
    /// Mirrors `SQLAlchemy.Inspector.has_column()` semantics.
    pub fn has_column(
        &self,
        table_name: &str,
        column_name: &str,
        schema: Option<&str>,
    ) -> Result<bool, IoError> {
        let Some(meta) = self.conn.table_schema(table_name, schema)? else {
            return Ok(false);
        };
        Ok(meta.column(column_name).is_some())
    }

    /// Look up the metadata bundle for a single column.
    ///
    /// Per br-frankenpandas-ppry (fd90.39). Returns `Ok(None)` when the
    /// table doesn't exist or the column isn't present. The returned
    /// `SqlColumnSchema` carries the full set of fields populated by
    /// the underlying `table_schema` impl (declared_type, nullable,
    /// default_value, primary_key_ordinal, comment, autoincrement).
    pub fn column(
        &self,
        table_name: &str,
        column_name: &str,
        schema: Option<&str>,
    ) -> Result<Option<SqlColumnSchema>, IoError> {
        let Some(meta) = self.conn.table_schema(table_name, schema)? else {
            return Ok(None);
        };
        Ok(meta.column(column_name).cloned())
    }

    /// Reflect a full table's metadata in one call: columns, primary
    /// key, indexes, foreign keys, unique constraints, and comment.
    ///
    /// Per br-frankenpandas-76mw (fd90.40). Mirrors
    /// `SQLAlchemy.MetaData.reflect_table` shape — gives callers a
    /// single bundle instead of 5 separate fetches. Returns `Ok(None)`
    /// when the table doesn't exist (matched via `table_schema`
    /// returning `None`); otherwise all derived calls run and any
    /// missing pieces (e.g. SQLite's always-None `table_comment`)
    /// are simply preserved as their natural empty values.
    ///
    /// Per br-frankenpandas-2kzv (fd90.43): primary-key columns are
    /// derived directly from the `SqlTableSchema` we already fetched
    /// rather than dispatching `primary_key_columns()` again — that
    /// trait method's default impl calls `table_schema()` internally,
    /// which would double the round-trip count for backends where
    /// each call is a real network hop.
    pub fn reflect_table(
        &self,
        table_name: &str,
        schema: Option<&str>,
    ) -> Result<Option<SqlReflectedTable>, IoError> {
        let Some(meta) = self.conn.table_schema(table_name, schema)? else {
            return Ok(None);
        };
        let primary_key_columns = primary_keys_from_schema(&meta);
        let indexes = self.conn.list_indexes(table_name, schema)?;
        let foreign_keys = self.conn.list_foreign_keys(table_name, schema)?;
        let unique_constraints =
            self.conn.list_unique_constraints(table_name, schema)?;
        let comment = self.conn.table_comment(table_name, schema)?;
        Ok(Some(SqlReflectedTable {
            table_name: meta.table_name,
            columns: meta.columns,
            primary_key_columns,
            indexes,
            foreign_keys,
            unique_constraints,
            comment,
        }))
    }

    /// Reflect every user-visible table in `schema` into a vector of
    /// bundles, one per table.
    ///
    /// Per br-frankenpandas-jmmo (fd90.53). Iterates `tables(schema)`
    /// then calls `reflect_table` on each. Skips any table that
    /// `reflect_table` returns `Ok(None)` for — covers the race
    /// condition where a table existed at list time but not at
    /// reflect time (e.g. concurrent DROP). Useful for whole-database
    /// introspection in one call.
    pub fn reflect_all_tables(
        &self,
        schema: Option<&str>,
    ) -> Result<Vec<SqlReflectedTable>, IoError> {
        let table_names = self.conn.list_tables(schema)?;
        let mut bundles = Vec::with_capacity(table_names.len());
        for name in table_names {
            if let Some(bundle) = self.reflect_table(&name, schema)? {
                bundles.push(bundle);
            }
        }
        Ok(bundles)
    }

    /// Reflect every user-visible view in `schema` into a vector of
    /// bundles, one per view.
    ///
    /// Per br-frankenpandas-zuqt (fd90.54). View-side parity with
    /// `reflect_all_tables`: iterates `views(schema)` then calls
    /// `reflect_table` on each (PRAGMA table_info works on views too,
    /// returning the view's column shape). PK/FK/UC/index lists in
    /// the bundle will typically be empty for views since views don't
    /// carry constraints — only the column metadata + comment are
    /// meaningful. Same disappearing-entity skip semantics as
    /// `reflect_all_tables`.
    pub fn reflect_all_views(
        &self,
        schema: Option<&str>,
    ) -> Result<Vec<SqlReflectedTable>, IoError> {
        let view_names = self.conn.list_views(schema)?;
        let mut bundles = Vec::with_capacity(view_names.len());
        for name in view_names {
            if let Some(bundle) = self.reflect_table(&name, schema)? {
                bundles.push(bundle);
            }
        }
        Ok(bundles)
    }
}

/// Derive the primary-key column names from an already-fetched
/// `SqlTableSchema`, sorted ascending by `primary_key_ordinal`.
///
/// Per br-frankenpandas-2kzv (fd90.43) / fd90.47: this is the
/// canonical filter+sort impl shared by both
/// `SqlConnection::primary_key_columns` (the trait default) and
/// `SqlInspector::reflect_table` (which uses already-fetched
/// metadata to avoid a redundant `table_schema()` round-trip).
fn primary_keys_from_schema(meta: &SqlTableSchema) -> Vec<String> {
    let mut pk: Vec<(usize, String)> = meta
        .columns
        .iter()
        .filter_map(|c| {
            c.primary_key_ordinal.map(|ord| (ord, c.name.clone()))
        })
        .collect();
    pk.sort_by_key(|(ord, _)| *ord);
    pk.into_iter().map(|(_, name)| name).collect()
}

/// Convenience constructor for `SqlInspector`.
///
/// `let inspector = inspect(&conn);` reads more naturally than
/// `SqlInspector::new(&conn)` for one-shot uses. Per br-frankenpandas-szs9 (fd90.38).
#[must_use]
pub fn inspect<C: SqlConnection>(conn: &C) -> SqlInspector<'_, C> {
    SqlInspector::new(conn)
}

/// Read an entire SQL table into a DataFrame with read-time options.
///
/// Matches the supported subset of
/// `pd.read_sql_table(table_name, con, parse_dates=..., coerce_float=...)`.
pub fn read_sql_table_with_options<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    options: &SqlReadOptions,
) -> Result<DataFrame, IoError> {
    // Per br-frankenpandas-d3e9 (fd90.34): when options.columns is
    // Some(list), project only those columns instead of SELECT *.
    let query = match options.columns.as_deref() {
        Some(cols) => {
            let refs: Vec<&str> = cols.iter().map(String::as_str).collect();
            sql_select_columns_query_in_schema(
                conn,
                table_name,
                options.schema.as_deref(),
                &refs,
            )?
        }
        None => sql_select_all_query_in_schema(conn, table_name, options.schema.as_deref())?,
    };
    read_sql_with_options(conn, &query, options)
}

/// Read an entire SQL table with read-time options and optional index promotion.
///
/// Matches the supported subset of
/// `pd.read_sql_table(table_name, con, parse_dates=..., coerce_float=..., index_col=...)`.
pub fn read_sql_table_with_options_and_index_col<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    options: &SqlReadOptions,
    index_col: Option<&str>,
) -> Result<DataFrame, IoError> {
    // Per br-frankenpandas-c1h9 (fd90.36): explicit `index_col` arg
    // always wins over `options.index_col`. Avoid double-promotion by
    // clearing the option-struct copy when the explicit arg is set.
    if let Some(col_name) = index_col {
        let cleared = SqlReadOptions {
            index_col: None,
            ..options.clone()
        };
        let frame = read_sql_table_with_options(conn, table_name, &cleared)?;
        return promote_column_to_index(&frame, col_name);
    }
    read_sql_table_with_options(conn, table_name, options)
}

/// Read an entire SQL table as an iterator of DataFrame chunks.
///
/// Matches the supported subset of `pd.read_sql_table(table_name, con, chunksize=...)`.
pub fn read_sql_table_chunks<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    chunk_size: usize,
) -> Result<SqlChunkIterator, IoError> {
    read_sql_chunks(conn, &sql_select_all_query(conn, table_name)?, chunk_size)
}

/// Read an entire SQL table as DataFrame chunks with read-time options.
///
/// Matches the supported subset of
/// `pd.read_sql_table(table_name, con, parse_dates=..., coerce_float=..., chunksize=...)`.
pub fn read_sql_table_chunks_with_options<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    options: &SqlReadOptions,
    chunk_size: usize,
) -> Result<SqlChunkIterator, IoError> {
    let query =
        sql_select_all_query_in_schema(conn, table_name, options.schema.as_deref())?;
    read_sql_chunks_with_options(conn, &query, options, chunk_size)
}

/// Read an entire SQL table as chunks with read-time options and optional index promotion.
///
/// Matches the supported subset of
/// `pd.read_sql_table(table_name, con, parse_dates=..., coerce_float=..., index_col=..., chunksize=...)`.
pub fn read_sql_table_chunks_with_options_and_index_col<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    options: &SqlReadOptions,
    index_col: Option<&str>,
    chunk_size: usize,
) -> Result<SqlIndexedChunkIterator, IoError> {
    let inner = read_sql_table_chunks_with_options(conn, table_name, options, chunk_size)?;
    sql_indexed_chunks(inner, index_col)
}

/// Read an entire SQL table as chunks with one column promoted to each chunk's index.
///
/// Matches the supported subset of
/// `pd.read_sql_table(table_name, con, index_col=..., chunksize=...)`.
pub fn read_sql_table_chunks_with_index_col<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    index_col: Option<&str>,
    chunk_size: usize,
) -> Result<SqlIndexedChunkIterator, IoError> {
    let inner = read_sql_table_chunks(conn, table_name, chunk_size)?;
    sql_indexed_chunks(inner, index_col)
}

/// Read a subset of columns from a SQL table.
///
/// Matches `pd.read_sql_table(table, con, columns=[...])`. The named
/// columns are emitted in the requested order. Each column name must
/// satisfy the same alphanumeric+underscore rule as `table_name` to
/// keep the projection injection-safe; mismatched names return
/// `IoError::Sql`. Empty `columns` is rejected (pandas raises in
/// the same case rather than producing an empty SELECT).
pub fn read_sql_table_columns<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    columns: &[&str],
) -> Result<DataFrame, IoError> {
    read_sql(conn, &sql_select_columns_query(conn, table_name, columns)?)
}

/// Read a subset of columns from a SQL table with optional index promotion.
///
/// Matches the supported subset of
/// `pd.read_sql_table(table, con, columns=[...], index_col=...)`. When
/// `index_col` is set it must be included in `columns`; the promoted column is
/// removed from the data columns after projection.
pub fn read_sql_table_columns_with_index_col<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    columns: &[&str],
    index_col: Option<&str>,
) -> Result<DataFrame, IoError> {
    let frame = read_sql_table_columns(conn, table_name, columns)?;
    let Some(col_name) = index_col else {
        return Ok(frame);
    };
    promote_column_to_index(&frame, col_name)
}

/// Read a subset of columns from a SQL table as DataFrame chunks.
///
/// Matches the supported subset of
/// `pd.read_sql_table(table, con, columns=[...], chunksize=...)`. The named
/// columns are emitted in the requested order and each chunk receives a fresh
/// zero-based RangeIndex.
pub fn read_sql_table_columns_chunks<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    columns: &[&str],
    chunk_size: usize,
) -> Result<SqlChunkIterator, IoError> {
    read_sql_chunks(
        conn,
        &sql_select_columns_query(conn, table_name, columns)?,
        chunk_size,
    )
}

/// Read a subset of columns from a SQL table as chunks with optional index promotion.
///
/// Matches the supported subset of
/// `pd.read_sql_table(table, con, columns=[...], index_col=..., chunksize=...)`.
/// When `index_col` is set it must be included in `columns`; the promoted
/// column is removed from each chunk after projection.
pub fn read_sql_table_columns_chunks_with_index_col<C: SqlConnection>(
    conn: &C,
    table_name: &str,
    columns: &[&str],
    index_col: Option<&str>,
    chunk_size: usize,
) -> Result<SqlIndexedChunkIterator, IoError> {
    let inner = read_sql_table_columns_chunks(conn, table_name, columns, chunk_size)?;
    sql_indexed_chunks(inner, index_col)
}

/// Write a DataFrame to a SQL table.
///
/// Matches `pd.DataFrame.to_sql(name, con)`.
pub fn write_sql<C: SqlConnection>(
    frame: &DataFrame,
    conn: &C,
    table_name: &str,
    if_exists: SqlIfExists,
) -> Result<(), IoError> {
    write_sql_with_options(
        frame,
        conn,
        table_name,
        &SqlWriteOptions {
            if_exists,
            index: false,
            index_label: None,
            schema: None,
            dtype: None,
            method: SqlInsertMethod::Single,
            chunksize: None,
        },
    )
}

/// Write a DataFrame to a SQLite table with pandas-style index options.
///
/// Matches the supported subset of
/// `pd.DataFrame.to_sql(name, con, index=..., index_label=...)`.
pub fn write_sql_with_options<C: SqlConnection>(
    frame: &DataFrame,
    conn: &C,
    table_name: &str,
    options: &SqlWriteOptions,
) -> Result<(), IoError> {
    // Validate table name to prevent SQL injection (only allow alphanumeric + underscore, non-empty).
    if table_name.is_empty() || !table_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(IoError::Sql(format!(
            "invalid table name: '{table_name}' (must be non-empty, only alphanumeric and underscore allowed)"
        )));
    }

    let col_names: Vec<String> = frame.column_names().into_iter().cloned().collect();
    let index_label = resolve_sql_index_label(frame, options)?;
    let mut sql_col_names =
        Vec::with_capacity(col_names.len() + usize::from(index_label.is_some()));
    if let Some(ref label) = index_label {
        sql_col_names.push(label.clone());
    }
    sql_col_names.extend(col_names.iter().cloned());

    // Per fd90.27: when the backend reports an identifier-length cap
    // (PG=63, MySQL=64, MSSQL=128), reject any identifier that exceeds
    // it before emitting DDL. SQLite (None) is unaffected.
    let max_ident = conn.max_identifier_length();
    validate_sql_identifier_length(table_name, max_ident, "table")?;
    if let Some(ref label) = index_label {
        validate_sql_identifier_length(label, max_ident, "index label")?;
    }
    for name in &col_names {
        validate_sql_identifier_length(name, max_ident, "column")?;
    }
    if let Some(s) = options.schema.as_deref() {
        validate_sql_identifier_length(s, max_ident, "schema")?;
    }

    // Handle if_exists policy.
    let schema = options.schema.as_deref();
    match options.if_exists {
        SqlIfExists::Fail => {
            let exists = conn.table_exists_in_schema(table_name, schema)?;
            if exists {
                return Err(IoError::Sql(format!("table '{table_name}' already exists")));
            }
        }
        SqlIfExists::Replace => {
            let drop_sql = sql_drop_table_query_in_schema(conn, table_name, schema)?;
            conn.execute_batch(&drop_sql)?;
        }
        SqlIfExists::Append => {
            // Table may or may not exist; CREATE TABLE IF NOT EXISTS handles both.
        }
    }

    // Build CREATE TABLE statement.
    let mut col_defs = Vec::with_capacity(sql_col_names.len());
    if let Some(ref label) = index_label {
        col_defs.push(sql_column_definition(
            conn,
            label,
            conn.index_dtype_sql(frame.index()),
        )?);
    }
    let dtype_overrides = options.dtype.as_ref();
    col_defs.extend(
        col_names
            .iter()
            .map(|name| {
                // Per br-frankenpandas-ev2s (fd90.18): explicit per-column
                // SQL-type override wins over the inferred conn.dtype_sql.
                let override_sql = dtype_overrides.and_then(|m| m.get(name)).map(|s| s.as_str());
                let sql_type = match override_sql {
                    Some(s) => s,
                    None => {
                        let dt = frame.column(name).map_or(DType::Utf8, |c| c.dtype());
                        conn.dtype_sql(dt)
                    }
                };
                sql_column_definition(conn, name, sql_type)
            })
            .collect::<Result<Vec<_>, IoError>>()?,
    );

    let create_sql = sql_create_table_query_in_schema(conn, table_name, schema, &col_defs)?;
    conn.execute_batch(&create_sql)?;

    let nrows = frame.index().len();
    let ncols = sql_col_names.len();
    let mut rows = Vec::with_capacity(nrows);
    for row_idx in 0..nrows {
        let mut row = Vec::with_capacity(ncols);
        if options.index {
            row.push(scalar_from_index_label(&frame.index().labels()[row_idx]));
        }
        row.extend(col_names.iter().map(|name| {
            frame
                .column(name)
                .and_then(|col| col.value(row_idx))
                .cloned()
                .unwrap_or(Scalar::Null(NullKind::Null))
        }));
        rows.push(row);
    }

    if rows.is_empty() {
        // Empty frame: still emit CREATE TABLE (already done) but skip INSERT.
        return Ok(());
    }

    // Per fd90.33: pandas-style chunksize. None preserves prior
    // single-transaction semantics; Some(0) is rejected (matches pandas).
    if let Some(0) = options.chunksize {
        return Err(IoError::Sql(
            "invalid chunksize: 0 (must be > 0 if Some)".to_owned(),
        ));
    }

    match options.method {
        SqlInsertMethod::Single => {
            let insert_sql =
                sql_insert_rows_query_in_schema(conn, table_name, schema, &sql_col_names)?;
            match options.chunksize {
                None => {
                    conn.insert_rows(&insert_sql, &rows)?;
                }
                Some(n) => {
                    for chunk in rows.chunks(n) {
                        conn.insert_rows(&insert_sql, chunk)?;
                    }
                }
            }
        }
        SqlInsertMethod::Multi => {
            // Per fd90.19: chunk rows to fit `max_param_count`. When the
            // backend reports None, send the whole frame in one statement.
            // Per fd90.33: when chunksize is also Some(n), the effective
            // chunk row count is min(n, max_param_count / num_cols).
            let param_chunk = match conn.max_param_count() {
                Some(max) if ncols > 0 => {
                    let per_chunk = max / ncols;
                    if per_chunk == 0 {
                        return Err(IoError::Sql(format!(
                            "multi-row insert: ncols={ncols} exceeds backend max_param_count={max}"
                        )));
                    }
                    per_chunk
                }
                _ => rows.len(),
            };
            let chunk_rows = options
                .chunksize
                .map(|cs| cs.min(param_chunk))
                .unwrap_or(param_chunk);
            for chunk in rows.chunks(chunk_rows) {
                let chunk_sql = sql_multi_row_insert_query_in_schema(
                    conn,
                    table_name,
                    schema,
                    &sql_col_names,
                    chunk.len(),
                )?;
                let mut flat = Vec::with_capacity(chunk.len() * ncols);
                for row in chunk {
                    flat.extend(row.iter().cloned());
                }
                conn.insert_rows(&chunk_sql, &[flat])?;
            }
        }
    }

    Ok(())
}

// ── Extension trait for DataFrame IO convenience methods ─────────────

/// Extension trait that adds IO convenience methods to `DataFrame`.
///
/// Import this trait to call `df.to_parquet(path)`, `df.to_parquet_bytes()`,
/// `DataFrame::from_parquet(path)`, etc. directly on DataFrame values.
pub trait DataFrameIoExt {
    /// Write this DataFrame to a Parquet file.
    ///
    /// Matches `pd.DataFrame.to_parquet(path)`.
    fn to_parquet(&self, path: &Path) -> Result<(), IoError>;

    /// Serialize this DataFrame to Parquet bytes in memory.
    ///
    /// Matches `pd.DataFrame.to_parquet()` with no path (returns bytes).
    fn to_parquet_bytes(&self) -> Result<Vec<u8>, IoError>;

    /// Write this DataFrame to a CSV file.
    ///
    /// Matches `pd.DataFrame.to_csv(path)`.
    fn to_csv_file(&self, path: &Path) -> Result<(), IoError>;

    /// Write this DataFrame to a JSON file.
    ///
    /// Matches `pd.DataFrame.to_json(path)`.
    fn to_json_file(&self, path: &Path, orient: JsonOrient) -> Result<(), IoError>;

    /// Write this DataFrame to an Excel (.xlsx) file.
    ///
    /// Matches `pd.DataFrame.to_excel(path)`.
    fn to_excel_file(&self, path: &Path) -> Result<(), IoError>;

    /// Serialize this DataFrame to Excel (.xlsx) bytes in memory.
    fn to_excel_bytes(&self) -> Result<Vec<u8>, IoError>;

    /// Write this DataFrame to a JSONL file (one JSON object per line).
    ///
    /// Matches `pd.DataFrame.to_json(path, orient='records', lines=True)`.
    fn to_jsonl_file(&self, path: &Path) -> Result<(), IoError>;

    /// Write this DataFrame to an Arrow IPC (Feather v2) file.
    ///
    /// Matches `pd.DataFrame.to_feather(path)`.
    fn to_feather_file(&self, path: &Path) -> Result<(), IoError>;

    /// Serialize this DataFrame to Arrow IPC (Feather v2) bytes.
    fn to_feather_bytes(&self) -> Result<Vec<u8>, IoError>;

    /// Write this DataFrame to a SQL table.
    ///
    /// Matches `pd.DataFrame.to_sql(name, con)`.
    fn to_sql<C: SqlConnection>(
        &self,
        conn: &C,
        table_name: &str,
        if_exists: SqlIfExists,
    ) -> Result<(), IoError>;

    /// Write this DataFrame to a SQL table with pandas-style SQL write options.
    fn to_sql_with_options<C: SqlConnection>(
        &self,
        conn: &C,
        table_name: &str,
        options: &SqlWriteOptions,
    ) -> Result<(), IoError>;
}

impl DataFrameIoExt for DataFrame {
    fn to_parquet(&self, path: &Path) -> Result<(), IoError> {
        write_parquet(self, path)
    }

    fn to_parquet_bytes(&self) -> Result<Vec<u8>, IoError> {
        write_parquet_bytes(self)
    }

    fn to_csv_file(&self, path: &Path) -> Result<(), IoError> {
        write_csv(self, path)
    }

    fn to_json_file(&self, path: &Path, orient: JsonOrient) -> Result<(), IoError> {
        write_json(self, path, orient)
    }

    fn to_excel_file(&self, path: &Path) -> Result<(), IoError> {
        write_excel(self, path)
    }

    fn to_excel_bytes(&self) -> Result<Vec<u8>, IoError> {
        write_excel_bytes(self)
    }

    fn to_jsonl_file(&self, path: &Path) -> Result<(), IoError> {
        write_jsonl(self, path)
    }

    fn to_feather_file(&self, path: &Path) -> Result<(), IoError> {
        write_feather(self, path)
    }

    fn to_feather_bytes(&self) -> Result<Vec<u8>, IoError> {
        write_feather_bytes(self)
    }

    fn to_sql<C: SqlConnection>(
        &self,
        conn: &C,
        table_name: &str,
        if_exists: SqlIfExists,
    ) -> Result<(), IoError> {
        write_sql(self, conn, table_name, if_exists)
    }

    fn to_sql_with_options<C: SqlConnection>(
        &self,
        conn: &C,
        table_name: &str,
        options: &SqlWriteOptions,
    ) -> Result<(), IoError> {
        write_sql_with_options(self, conn, table_name, options)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use arrow::{
        array::{Array, Int64Array},
        datatypes::DataType as ArrowDataType,
    };
    use fp_columnar::Column;
    use fp_frame::{DataFrame, Series};
    use fp_index::{Index, IndexLabel};
    use fp_types::{DType, NullKind, Scalar};

    use super::{
        CsvWriteOptions, IoError, read_csv_str, read_csv_with_index_cols, write_csv_string,
        write_csv_string_with_options,
    };

    #[test]
    fn csv_round_trip_preserves_null_and_numeric_shape() {
        let input = "id,value\n1,10\n2,\n3,3.5\n";
        let frame = read_csv_str(input).expect("read");
        let value_col = frame.column("value").expect("value");

        assert_eq!(value_col.values()[1], Scalar::Null(NullKind::NaN));

        let out = write_csv_string(&frame).expect("write");
        assert!(out.contains("id,value"));
        assert!(out.contains("3,3.5"));
    }

    #[test]
    fn csv_parses_boolean_true_false_case_insensitive() {
        let input = "flag\nTrue\nFALSE\ntrue\nfalse\n";
        let frame = read_csv_str(input).expect("read");
        let flag_col = frame.column("flag").expect("flag");
        assert_eq!(flag_col.values()[0], Scalar::Bool(true));
        assert_eq!(flag_col.values()[1], Scalar::Bool(false));
        assert_eq!(flag_col.values()[2], Scalar::Bool(true));
        assert_eq!(flag_col.values()[3], Scalar::Bool(false));
    }

    #[test]
    fn csv_duplicate_headers_error() {
        let input = "a,a\n1,2\n";
        let err = read_csv_str(input).expect_err("duplicate header");
        assert!(matches!(err, IoError::DuplicateColumnName(name) if name == "a"));
    }

    // === AG-07-T: CSV Parser Optimization Tests ===

    #[test]
    fn test_csv_vec_based_column_order() {
        // Verify Vec-based parser preserves header-to-data mapping exactly.
        // BTreeMap sorts alphabetically, so we use alpha-ordered headers.
        let input = "alpha,bravo,charlie\n1,2,3\n4,5,6\n";
        let frame = read_csv_str(input).expect("parse");
        let keys: Vec<&String> = frame.columns().keys().collect();
        assert_eq!(keys, &["alpha", "bravo", "charlie"]);
        assert_eq!(frame.column("alpha").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(frame.column("bravo").unwrap().values()[0], Scalar::Int64(2));
        assert_eq!(
            frame.column("charlie").unwrap().values()[1],
            Scalar::Int64(6)
        );
        eprintln!("[TEST] test_csv_vec_based_column_order | rows=2 cols=3 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_capacity_hint_reasonable() {
        // Generate a ~1MB CSV and verify it parses correctly.
        // The capacity hint (input.len / (cols*8)) should avoid excessive reallocs.
        let mut csv = String::with_capacity(1_100_000);
        csv.push_str("a,b,c,d,e\n");
        let target_rows = 50_000; // ~20 bytes/row * 50k ≈ 1MB
        for i in 0..target_rows {
            csv.push_str(&format!("{},{},{},{},{}\n", i, i * 2, i * 3, i * 4, i * 5));
        }
        assert!(csv.len() > 500_000, "CSV should be large");

        let frame = read_csv_str(&csv).expect("parse large CSV");
        assert_eq!(frame.index().len(), target_rows);
        assert_eq!(frame.columns().len(), 5);
        // Spot-check last row
        assert_eq!(
            frame.column("a").unwrap().values()[target_rows - 1],
            Scalar::Int64((target_rows - 1) as i64)
        );
        eprintln!(
            "[TEST] test_csv_capacity_hint_reasonable | rows={target_rows} cols=5 parse_ok=true | PASS"
        );
    }

    #[test]
    fn test_csv_empty_columns() {
        // CSV with headers but no data rows -> empty DataFrame with correct column names.
        let input = "x,y,z\n";
        let frame = read_csv_str(input).expect("parse");
        assert_eq!(frame.index().len(), 0);
        let keys: Vec<&String> = frame.columns().keys().collect();
        assert_eq!(keys, &["x", "y", "z"]);
        for col in frame.columns().values() {
            assert!(col.is_empty());
        }
        eprintln!("[TEST] test_csv_empty_columns | rows=0 cols=3 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_comment_skips_lines() {
        let input = "# header comment\nname,age\n# inline comment\nalice,30\nbob,25\n";
        let options = CsvReadOptions {
            comment: Some(b'#'),
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(frame.index().len(), 2);
        let names: Vec<&String> = frame.column_names().into_iter().collect();
        assert_eq!(names, vec!["name", "age"]);
        assert_eq!(
            frame.column("name").unwrap().values()[0],
            Scalar::Utf8("alice".to_string())
        );
        assert_eq!(frame.column("age").unwrap().values()[1], Scalar::Int64(25));
    }

    #[test]
    fn test_csv_comment_none_preserves_comment_lines() {
        // Without comment set, a leading "#"-line should become part of parsing
        // (and fail as duplicate/missing-headers or be treated as data).
        let input = "name,age\nalice,30\n";
        let frame = read_csv_with_options(input, &CsvReadOptions::default()).expect("parse");
        assert_eq!(frame.index().len(), 1);
    }

    #[test]
    fn test_csv_comment_custom_char() {
        let input = "% this is ignored\nname,age\nalice,30\n";
        let options = CsvReadOptions {
            comment: Some(b'%'),
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(frame.index().len(), 1);
        assert_eq!(
            frame.column("name").unwrap().values()[0],
            Scalar::Utf8("alice".to_string())
        );
    }

    #[test]
    fn test_csv_thousands_strips_int_separator() {
        let input = "amount\n\"1,234,567\"\n\"42\"\n";
        let options = CsvReadOptions {
            thousands: Some(b','),
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(
            frame.column("amount").unwrap().values()[0],
            Scalar::Int64(1234567)
        );
        assert_eq!(
            frame.column("amount").unwrap().values()[1],
            Scalar::Int64(42)
        );
    }

    #[test]
    fn test_csv_thousands_strips_float_with_custom_decimal() {
        // European convention: '.' as thousands, ',' as decimal.
        let input = "price\n\"1.234,56\"\n";
        let options = CsvReadOptions {
            thousands: Some(b'.'),
            decimal: b',',
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        let v = frame.column("price").unwrap().values()[0].clone();
        assert!(matches!(v, Scalar::Float64(_)), "expected Float64");
        let Scalar::Float64(f) = v else { return };
        assert!((f - 1234.56).abs() < 1e-9);
    }

    #[test]
    fn test_csv_thousands_none_keeps_separator_as_string() {
        // Without thousands set, "1,234" in a single field stays Utf8.
        let input = "amount\n\"1,234\"\n";
        let frame = read_csv_with_options(input, &CsvReadOptions::default()).expect("parse");
        assert_eq!(
            frame.column("amount").unwrap().values()[0],
            Scalar::Utf8("1,234".to_string())
        );
    }

    #[test]
    fn test_csv_thousands_equal_to_decimal_is_ignored() {
        // pandas silently ignores thousands if it equals decimal.
        let input = "v\n\"1.234\"\n";
        let options = CsvReadOptions {
            thousands: Some(b'.'),
            decimal: b'.',
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        let v = frame.column("v").unwrap().values()[0].clone();
        // thousands ignored → "1.234" parses as float 1.234
        assert!(matches!(v, Scalar::Float64(_)), "expected Float64");
        let Scalar::Float64(f) = v else { return };
        assert!((f - 1.234).abs() < 1e-9);
    }

    #[test]
    fn test_csv_thousands_does_not_affect_non_numeric() {
        let input = "name\n\"a,b\"\n";
        let options = CsvReadOptions {
            thousands: Some(b','),
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(
            frame.column("name").unwrap().values()[0],
            Scalar::Utf8("a,b".to_string())
        );
    }

    #[test]
    fn test_csv_quotechar_custom_single_quote() {
        let input = "name,remark\n'alice','loves, cats'\n";
        let options = CsvReadOptions {
            quotechar: b'\'',
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(
            frame.column("name").unwrap().values()[0],
            Scalar::Utf8("alice".to_string())
        );
        assert_eq!(
            frame.column("remark").unwrap().values()[0],
            Scalar::Utf8("loves, cats".to_string())
        );
    }

    #[test]
    fn test_csv_doublequote_true_collapses_doubled_quotes() {
        // The field is `she said ""hi""` with doubled inner quotes.
        let input = "text\n\"she said \"\"hi\"\"\"\n";
        let frame = read_csv_with_options(input, &CsvReadOptions::default()).expect("parse");
        assert_eq!(
            frame.column("text").unwrap().values()[0],
            Scalar::Utf8("she said \"hi\"".to_string())
        );
    }

    #[test]
    fn test_csv_doublequote_false_requires_escapechar() {
        // With doublequote=false and escapechar=\, \" escapes the quote.
        let input = "text\n\"hi\\\"there\"\n";
        let options = CsvReadOptions {
            doublequote: false,
            escapechar: Some(b'\\'),
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(
            frame.column("text").unwrap().values()[0],
            Scalar::Utf8("hi\"there".to_string())
        );
    }

    #[test]
    fn test_csv_lineterminator_semicolon() {
        // Single-byte record separator '|'. No newlines in the data.
        let input = "a,b|1,x|2,y|3,z";
        let options = CsvReadOptions {
            lineterminator: Some(b'|'),
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(frame.index().len(), 3);
        assert_eq!(frame.column_names(), vec!["a", "b"]);
        assert_eq!(frame.column("a").unwrap().values()[2], Scalar::Int64(3));
    }

    #[test]
    fn test_csv_lineterminator_default_none_accepts_crlf() {
        let input = "a,b\r\n1,x\r\n2,y\r\n";
        let frame = read_csv_with_options(input, &CsvReadOptions::default()).expect("parse");
        assert_eq!(frame.index().len(), 2);
    }

    #[test]
    fn test_csv_lineterminator_interacts_with_skipfooter() {
        let input = "a|1|2|3|4|FOOTER";
        let options = CsvReadOptions {
            lineterminator: Some(b'|'),
            skipfooter: 1,
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        // 5 data rows after header, footer drops 1 → 4 rows.
        assert_eq!(frame.index().len(), 4);
    }

    #[test]
    fn test_csv_skipfooter_drops_trailing_rows() {
        let input = "a,b\n1,x\n2,y\n3,z\nTOTAL,summary\n";
        let options = CsvReadOptions {
            skipfooter: 1,
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(frame.index().len(), 3);
        assert_eq!(frame.column("a").unwrap().values()[2], Scalar::Int64(3));
    }

    #[test]
    fn test_csv_skipfooter_zero_is_noop() {
        let input = "a,b\n1,x\n2,y\n";
        let frame_default =
            read_csv_with_options(input, &CsvReadOptions::default()).expect("parse");
        let options = CsvReadOptions {
            skipfooter: 0,
            ..CsvReadOptions::default()
        };
        let frame_zero = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(frame_default.index().len(), frame_zero.index().len());
    }

    #[test]
    fn test_csv_skipfooter_larger_than_data_clears_rows() {
        let input = "a,b\n1,x\n2,y\n";
        let options = CsvReadOptions {
            skipfooter: 10,
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(frame.index().len(), 0);
        // Columns and headers are still preserved.
        assert_eq!(frame.column_names().len(), 2);
    }

    #[test]
    fn test_csv_skipfooter_with_nrows() {
        // nrows caps read to 4, then skipfooter drops last 1 → 3 rows.
        let input = "a\n1\n2\n3\n4\n5\n";
        let options = CsvReadOptions {
            nrows: Some(4),
            skipfooter: 1,
            ..CsvReadOptions::default()
        };
        let frame = read_csv_with_options(input, &options).expect("parse");
        assert_eq!(frame.index().len(), 3);
    }

    #[test]
    fn test_csv_escapechar_none_default_keeps_backslash_literal() {
        // Without escapechar set, backslash is just a normal character.
        let input = "text\n\"foo\\bar\"\n";
        let frame = read_csv_with_options(input, &CsvReadOptions::default()).expect("parse");
        assert_eq!(
            frame.column("text").unwrap().values()[0],
            Scalar::Utf8("foo\\bar".to_string())
        );
    }

    #[test]
    fn test_csv_single_column() {
        // CSV with one column, many rows -> correct parsing.
        let mut csv = String::from("value\n");
        for i in 0..500 {
            csv.push_str(&format!("{}\n", i));
        }
        let frame = read_csv_str(&csv).expect("parse");
        assert_eq!(frame.index().len(), 500);
        assert_eq!(frame.columns().len(), 1);
        assert_eq!(
            frame.column("value").unwrap().values()[499],
            Scalar::Int64(499)
        );
        eprintln!("[TEST] test_csv_single_column | rows=500 cols=1 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_many_columns() {
        // CSV with 100+ columns -> all columns present, correct values.
        let col_count = 120;
        let headers: Vec<String> = (0..col_count).map(|i| format!("c{i:03}")).collect();
        let mut csv = headers.join(",");
        csv.push('\n');
        // 3 data rows
        for row in 0..3 {
            let vals: Vec<String> = (0..col_count)
                .map(|c| format!("{}", row * 1000 + c))
                .collect();
            csv.push_str(&vals.join(","));
            csv.push('\n');
        }
        let frame = read_csv_str(&csv).expect("parse");
        assert_eq!(frame.columns().len(), col_count);
        assert_eq!(frame.index().len(), 3);
        // Spot-check: c000 row 0 = 0, c119 row 2 = 2119
        assert_eq!(frame.column("c000").unwrap().values()[0], Scalar::Int64(0));
        assert_eq!(
            frame.column("c119").unwrap().values()[2],
            Scalar::Int64(2119)
        );
        eprintln!("[TEST] test_csv_many_columns | rows=3 cols={col_count} parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_mixed_dtypes() {
        // Columns with uniform int/float/string/bool/null -> correct type inference.
        let input = "ints,floats,strings,bools,nulls\n\
                     1,1.5,hello,true,\n\
                     2,2.7,world,false,\n\
                     3,3.14,foo,true,\n";
        let frame = read_csv_str(input).expect("parse");

        let ints = frame.column("ints").unwrap();
        assert_eq!(ints.values()[0], Scalar::Int64(1));

        let floats = frame.column("floats").unwrap();
        assert_eq!(floats.values()[1], Scalar::Float64(2.7));

        let strings = frame.column("strings").unwrap();
        assert_eq!(strings.values()[2], Scalar::Utf8("foo".to_owned()));

        let bools = frame.column("bools").unwrap();
        assert_eq!(bools.values()[0], Scalar::Bool(true));
        assert_eq!(bools.values()[1], Scalar::Bool(false));

        // "nulls" column is all empty -> all null/NaN
        let nulls = frame.column("nulls").unwrap();
        for v in nulls.values() {
            assert!(v.is_missing(), "null column values should be missing");
        }
        eprintln!(
            "[TEST] test_csv_mixed_dtypes | rows=3 cols=5 parse_ok=true | dtype_per_col=[int64,float64,utf8,bool,null] | PASS"
        );
    }

    #[test]
    fn test_csv_unicode_headers() {
        // CSV with unicode header names -> correct column names.
        let input = "名前,Größe,café\nAlice,170,latte\nBob,180,espresso\n";
        let frame = read_csv_str(input).expect("parse");
        assert!(frame.column("名前").is_some());
        assert!(frame.column("Größe").is_some());
        assert!(frame.column("café").is_some());
        assert_eq!(
            frame.column("名前").unwrap().values()[0],
            Scalar::Utf8("Alice".to_owned())
        );
        eprintln!("[TEST] test_csv_unicode_headers | rows=2 cols=3 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_quoted_fields() {
        // CSV with quoted fields containing commas and newlines -> correct parsing.
        let input =
            "name,address\n\"Smith, John\",\"123 Main St\nApt 4\"\nJane,\"456 Oak, Suite 1\"\n";
        let frame = read_csv_str(input).expect("parse");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(
            frame.column("name").unwrap().values()[0],
            Scalar::Utf8("Smith, John".to_owned())
        );
        // Quoted field with embedded newline
        let addr0 = &frame.column("address").unwrap().values()[0];
        assert!(
            matches!(addr0, Scalar::Utf8(s) if s.contains('\n')),
            "expected Utf8 containing embedded newline, got {addr0:?}"
        );
        eprintln!("[TEST] test_csv_quoted_fields | rows=2 cols=2 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_trailing_newline() {
        // CSV with/without trailing newline -> identical DataFrame.
        let with = "a,b\n1,2\n3,4\n";
        let without = "a,b\n1,2\n3,4";
        let f1 = read_csv_str(with).expect("with newline");
        let f2 = read_csv_str(without).expect("without newline");

        assert_eq!(f1.index().len(), f2.index().len());
        assert_eq!(f1.columns().len(), f2.columns().len());
        for key in f1.columns().keys() {
            let c1 = f1.column(key).unwrap();
            let c2 = f2.column(key).unwrap();
            assert_eq!(c1.values(), c2.values(), "column {key} mismatch");
        }
        eprintln!("[TEST] test_csv_trailing_newline | rows=2 cols=2 parse_ok=true | PASS");
    }

    #[test]
    fn test_csv_round_trip_unchanged() {
        // read_csv_str then write_csv_string produces semantically equivalent output.
        let input = "id,name,score\n1,Alice,95.5\n2,Bob,87\n3,,100\n";
        let frame = read_csv_str(input).expect("read");
        let output = write_csv_string(&frame).expect("write");
        // Re-parse the output and compare
        let frame2 = read_csv_str(&output).expect("re-read");
        assert_eq!(frame.index().len(), frame2.index().len());
        for key in frame.columns().keys() {
            let c1 = frame.column(key).unwrap();
            let c2 = frame2.column(key).unwrap();
            assert!(
                c1.semantic_eq(c2),
                "column {key} not semantically equal after round-trip"
            );
        }
        eprintln!("[TEST] test_csv_round_trip_unchanged | rows=3 cols=3 parse_ok=true | PASS");
    }

    #[test]
    fn test_write_csv_options_custom_delimiter() {
        let input = "a,b\n1,x\n2,y\n";
        let frame = read_csv_str(input).expect("read");
        let output = write_csv_string_with_options(
            &frame,
            &CsvWriteOptions {
                delimiter: b';',
                ..CsvWriteOptions::default()
            },
        )
        .expect("write");
        assert!(output.starts_with("a;b\n"));
        assert!(output.contains("1;x\n"));
        assert!(output.contains("2;y\n"));
    }

    #[test]
    fn test_write_csv_options_na_rep_replaces_nulls() {
        let input = "id,name\n1,Alice\n2,\n";
        let frame = read_csv_str(input).expect("read");
        let output = write_csv_string_with_options(
            &frame,
            &CsvWriteOptions {
                na_rep: "NA".to_string(),
                ..CsvWriteOptions::default()
            },
        )
        .expect("write");
        // Second data row's name should render as NA, not empty.
        assert!(output.contains("2,NA\n"));
        assert!(!output.contains("2,\n"));
    }

    #[test]
    fn test_write_csv_options_header_false_omits_header_row() {
        let input = "a,b\n1,2\n";
        let frame = read_csv_str(input).expect("read");
        let output = write_csv_string_with_options(
            &frame,
            &CsvWriteOptions {
                header: false,
                ..CsvWriteOptions::default()
            },
        )
        .expect("write");
        assert_eq!(output, "1,2\n");
    }

    #[test]
    fn test_write_csv_options_include_index_and_index_label() {
        let input = "a,b\n1,2\n3,4\n";
        let frame = read_csv_str(input).expect("read");
        let output = write_csv_string_with_options(
            &frame,
            &CsvWriteOptions {
                include_index: true,
                index_label: Some("row_id".to_string()),
                ..CsvWriteOptions::default()
            },
        )
        .expect("write");

        assert_eq!(output, "row_id,a,b\n0,1,2\n1,3,4\n");
    }

    #[test]
    fn test_write_csv_options_include_index_uses_named_index_when_label_omitted() {
        let mut cols = std::collections::BTreeMap::new();
        cols.insert(
            "a".to_string(),
            Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20)]).unwrap(),
        );
        let frame = DataFrame::new_with_column_order(
            Index::from_i64(vec![100, 200]).set_name("sample_id"),
            cols,
            vec!["a".to_string()],
        )
        .unwrap();

        let output = write_csv_string_with_options(
            &frame,
            &CsvWriteOptions {
                include_index: true,
                ..CsvWriteOptions::default()
            },
        )
        .expect("write");

        assert_eq!(output, "sample_id,a\n100,10\n200,20\n");
    }

    #[test]
    fn test_write_csv_options_include_index_label_overrides_index_name() {
        let mut cols = std::collections::BTreeMap::new();
        cols.insert(
            "a".to_string(),
            Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20)]).unwrap(),
        );
        let frame = DataFrame::new_with_column_order(
            Index::from_i64(vec![100, 200]).set_name("sample_id"),
            cols,
            vec!["a".to_string()],
        )
        .unwrap();

        let output = write_csv_string_with_options(
            &frame,
            &CsvWriteOptions {
                include_index: true,
                index_label: Some("row".to_string()),
                ..CsvWriteOptions::default()
            },
        )
        .expect("write");

        assert_eq!(output, "row,a\n100,10\n200,20\n");
    }

    #[test]
    fn test_csv_multiindex_roundtrip_with_explicit_index_cols() {
        let frame = make_row_multiindex_test_dataframe();
        let csv = write_csv_string_with_options(
            &frame,
            &CsvWriteOptions {
                include_index: true,
                ..CsvWriteOptions::default()
            },
        )
        .expect("write");

        let roundtrip = read_csv_with_index_cols(
            &csv,
            &CsvReadOptions::default(),
            &["region", "product", "year"],
        )
        .expect("read");

        assert!(roundtrip.equals(&frame));
        assert_eq!(roundtrip.row_multiindex(), frame.row_multiindex());
    }

    #[test]
    fn test_write_csv_options_default_matches_write_csv_string() {
        let input = "a,b\n1,2\n3,4\n";
        let frame = read_csv_str(input).expect("read");
        let default_output = write_csv_string(&frame).expect("write");
        let options_output =
            write_csv_string_with_options(&frame, &CsvWriteOptions::default()).expect("write");
        assert_eq!(default_output, options_output);
    }

    #[test]
    fn test_write_csv_options_na_rep_with_float_nan() {
        // Generate a frame with an explicit NaN float.
        use fp_columnar::Column;
        let mut cols = std::collections::BTreeMap::new();
        cols.insert(
            "score".to_string(),
            Column::from_values(vec![Scalar::Float64(1.5), Scalar::Float64(f64::NAN)]).unwrap(),
        );
        let frame = DataFrame::new_with_column_order(
            Index::from_i64(vec![0, 1]),
            cols,
            vec!["score".to_string()],
        )
        .unwrap();
        let output = write_csv_string_with_options(
            &frame,
            &CsvWriteOptions {
                na_rep: "NaN".to_string(),
                ..CsvWriteOptions::default()
            },
        )
        .expect("write");
        assert!(output.contains("NaN"));
    }

    #[test]
    fn test_csv_large_file_perf() {
        // 100K-row, 10-column CSV -> parse completes, correct row/column counts.
        let col_count = 10;
        let row_count = 100_000;
        let headers: Vec<String> = (0..col_count).map(|i| format!("col{i}")).collect();
        let mut csv = String::with_capacity(row_count * 50);
        csv.push_str(&headers.join(","));
        csv.push('\n');
        for r in 0..row_count {
            for c in 0..col_count {
                if c > 0 {
                    csv.push(',');
                }
                csv.push_str(&(r * col_count + c).to_string());
            }
            csv.push('\n');
        }

        let frame = read_csv_str(&csv).expect("parse 100K rows");
        assert_eq!(frame.index().len(), row_count);
        assert_eq!(frame.columns().len(), col_count);
        // Spot-check first and last rows
        assert_eq!(frame.column("col0").unwrap().values()[0], Scalar::Int64(0));
        assert_eq!(
            frame.column("col9").unwrap().values()[row_count - 1],
            Scalar::Int64(((row_count - 1) * col_count + 9) as i64)
        );
        eprintln!(
            "[TEST] test_csv_large_file_perf | rows={row_count} cols={col_count} parse_ok=true | PASS"
        );
    }

    #[test]
    fn test_csv_golden_output() {
        // Fixed CSV input -> write_csv_string output matches golden reference exactly.
        let input = "a,b,c\n1,hello,3.14\n2,,true\n3,world,\n";
        let frame = read_csv_str(input).expect("parse");
        let output = write_csv_string(&frame).expect("write");

        // Golden reference: columns in BTreeMap order; Bool(true) coerced to Float64
        // in column c (which has Float64 + Bool → Float64), so true → 1.
        let expected = "a,b,c\n1,hello,3.14\n2,,1\n3,world,\n";
        assert_eq!(
            output, expected,
            "output does not match golden reference.\nGot:\n{output}\nExpected:\n{expected}"
        );
        eprintln!("[TEST] test_csv_golden_output | golden_match=true | PASS");
    }

    // === bd-2gi.19: IO Complete Contract Tests ===

    use super::{
        CsvOnBadLines, CsvReadOptions, JsonOrient, read_csv_with_options, read_json_str,
        write_json_string,
    };

    #[test]
    fn csv_with_custom_delimiter() {
        let input = "a\tb\tc\n1\t2\t3\n4\t5\t6\n";
        let opts = CsvReadOptions {
            delimiter: b'\t',
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse tsv");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(frame.column("a").unwrap().values()[0], Scalar::Int64(1));
    }

    #[test]
    fn csv_without_headers_generates_default_names_and_keeps_first_row() {
        let input = "1,2\n3,4\n";
        let opts = CsvReadOptions {
            has_headers: false,
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(
            frame.column("column_0").unwrap().values()[0],
            Scalar::Int64(1)
        );
        assert_eq!(
            frame.column("column_1").unwrap().values()[0],
            Scalar::Int64(2)
        );
        assert_eq!(
            frame.column("column_0").unwrap().values()[1],
            Scalar::Int64(3)
        );
        assert_eq!(
            frame.column("column_1").unwrap().values()[1],
            Scalar::Int64(4)
        );
    }

    #[test]
    fn csv_usecols_missing_column_errors() {
        let input = "a,b\n1,2\n";
        let opts = CsvReadOptions {
            usecols: Some(vec!["c".to_string()]),
            ..Default::default()
        };
        let err = read_csv_with_options(input, &opts).expect_err("missing usecols");
        assert!(
            matches!(err, IoError::MissingUsecols(missing) if missing == vec!["c".to_string()])
        );
    }

    #[test]
    fn csv_without_headers_supports_generated_index_col_name() {
        let input = "10,alpha\n20,beta\n";
        let opts = CsvReadOptions {
            has_headers: false,
            index_col: Some("column_0".into()),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(frame.index().labels()[0], IndexLabel::Int64(10));
        assert_eq!(frame.index().labels()[1], IndexLabel::Int64(20));
        assert!(frame.column("column_0").is_none());
        assert_eq!(
            frame.column("column_1").unwrap().values()[0],
            Scalar::Utf8("alpha".into())
        );
        assert_eq!(
            frame.column("column_1").unwrap().values()[1],
            Scalar::Utf8("beta".into())
        );
    }

    #[test]
    fn csv_with_na_values() {
        let input = "a,b\n1,NA\n2,n/a\n3,valid\n";
        let opts = CsvReadOptions {
            na_values: vec!["NA".into(), "n/a".into()],
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        let b = frame.column("b").unwrap();
        assert!(b.values()[0].is_missing());
        assert!(b.values()[1].is_missing());
        assert_eq!(b.values()[2], Scalar::Utf8("valid".into()));
    }

    #[test]
    fn csv_none_is_default_na() {
        // "None" is a pandas default NA value (Python's None)
        let input = "a,b\n1,None\n2,valid\n";
        let frame = read_csv_str(input).expect("parse");
        let b = frame.column("b").unwrap();
        assert!(b.values()[0].is_missing(), "None should be parsed as NA");
        assert_eq!(b.values()[1], Scalar::Utf8("valid".into()));
    }

    #[test]
    fn csv_keep_default_na_false() {
        // With keep_default_na=false, only custom na_values are recognized
        let input = "a,b\n1,NA\n2,CUSTOM\n3,valid\n";
        let opts = CsvReadOptions {
            na_values: vec!["CUSTOM".into()],
            keep_default_na: false,
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        let b = frame.column("b").unwrap();
        // "NA" should NOT be missing because keep_default_na=false
        assert_eq!(b.values()[0], Scalar::Utf8("NA".into()));
        // "CUSTOM" should be missing because it's in na_values
        assert!(b.values()[1].is_missing());
        assert_eq!(b.values()[2], Scalar::Utf8("valid".into()));
    }

    #[test]
    fn csv_na_filter_false() {
        // With na_filter=false, no NA detection at all (for performance)
        let input = "a,b\n1,NA\n2,\n3,None\n";
        let opts = CsvReadOptions {
            na_filter: false,
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        let b = frame.column("b").unwrap();
        // All values should be kept as strings, no NA detection
        assert_eq!(b.values()[0], Scalar::Utf8("NA".into()));
        assert_eq!(b.values()[1], Scalar::Utf8("".into()));
        assert_eq!(b.values()[2], Scalar::Utf8("None".into()));
    }

    #[test]
    fn csv_with_index_col() {
        let input = "id,val\na,10\nb,20\nc,30\n";
        let opts = CsvReadOptions {
            index_col: Some("id".into()),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(frame.index().len(), 3);
        assert_eq!(
            frame.index().labels()[0],
            fp_index::IndexLabel::Utf8("a".into())
        );
        assert!(frame.column("id").is_none());
        assert_eq!(frame.column("val").unwrap().values()[0], Scalar::Int64(10));
    }

    #[test]
    fn csv_with_missing_index_col_errors() {
        let input = "id,val\na,10\nb,20\n";
        let opts = CsvReadOptions {
            index_col: Some("missing".into()),
            ..Default::default()
        };

        let err = read_csv_with_options(input, &opts).expect_err("missing index_col should error");
        assert!(
            matches!(&err, IoError::MissingIndexColumn(name) if name == "missing"),
            "expected MissingIndexColumn(\"missing\"), got {err:?}"
        );
    }

    #[test]
    fn csv_with_malformed_row_errors() {
        let input = "a,b\n1,2\n3\n";
        let opts = CsvReadOptions::default();

        let err = read_csv_with_options(input, &opts).expect_err("malformed CSV row should error");
        assert!(
            matches!(&err, IoError::Csv(_)),
            "expected CSV parser error for ragged row, got {err:?}"
        );
    }

    #[test]
    fn csv_on_bad_lines_skip_skips_extra_field_rows() {
        let input = "a,b\n1,2\n3,4,5\n6,7\n";
        let opts = CsvReadOptions {
            on_bad_lines: CsvOnBadLines::Skip,
            ..Default::default()
        };

        let frame = read_csv_with_options(input, &opts).expect("parse with skipped bad line");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(frame.column("a").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(frame.column("b").unwrap().values()[0], Scalar::Int64(2));
        assert_eq!(frame.column("a").unwrap().values()[1], Scalar::Int64(6));
        assert_eq!(frame.column("b").unwrap().values()[1], Scalar::Int64(7));
    }

    #[test]
    fn csv_on_bad_lines_warn_skips_extra_field_rows() {
        let input = "a,b\n1,2\n3,4,5\n6,7\n";
        let opts = CsvReadOptions {
            on_bad_lines: CsvOnBadLines::Warn,
            ..Default::default()
        };

        let frame = read_csv_with_options(input, &opts).expect("parse with warned bad line");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(frame.column("a").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(frame.column("b").unwrap().values()[1], Scalar::Int64(7));
    }

    #[test]
    fn csv_on_bad_lines_skip_preserves_short_rows_as_missing() {
        let input = "a,b\n1,2\n3\n6,7\n";
        let opts = CsvReadOptions {
            on_bad_lines: CsvOnBadLines::Skip,
            ..Default::default()
        };

        let frame = read_csv_with_options(input, &opts).expect("parse short row");
        assert_eq!(frame.index().len(), 3);
        assert_eq!(frame.column("a").unwrap().values()[1], Scalar::Int64(3));
        assert_eq!(
            frame.column("b").unwrap().values()[1],
            Scalar::Null(NullKind::NaN)
        );
    }

    #[test]
    fn json_records_read_write_roundtrip() {
        let input = r#"[{"name":"Alice","age":30},{"name":"Bob","age":25}]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("read json records");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(
            frame.column("name").unwrap().values()[0],
            Scalar::Utf8("Alice".into())
        );
        assert_eq!(frame.column("age").unwrap().values()[1], Scalar::Int64(25));

        let output = write_json_string(&frame, JsonOrient::Records).expect("write");
        let frame2 = read_json_str(&output, JsonOrient::Records).expect("re-read");
        assert_eq!(frame2.index().len(), 2);
    }

    #[test]
    fn json_records_preserves_column_order() {
        let input = r#"[{"b":1,"a":2},{"c":3}]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("read json records");
        let order: Vec<&str> = frame
            .column_names()
            .iter()
            .map(|name| name.as_str())
            .collect();
        assert_eq!(order, vec!["b", "a", "c"]);
    }

    #[test]
    fn json_columns_read_write_roundtrip() {
        let input = r#"{"name":{"row_a":"Alice","row_b":"Bob"},"age":{"row_a":30,"row_b":25}}"#;
        let frame = read_json_str(input, JsonOrient::Columns).expect("read json columns");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(frame.index().labels()[0], IndexLabel::Utf8("row_a".into()));

        let output = write_json_string(&frame, JsonOrient::Columns).expect("write");
        let frame2 = read_json_str(&output, JsonOrient::Columns).expect("re-read");
        assert_eq!(frame2.index().labels(), frame.index().labels());
    }

    #[test]
    fn json_columns_write_duplicate_index_rejects() {
        let index = Index::new(vec![IndexLabel::Int64(1), IndexLabel::Utf8("1".into())]);
        let mut columns = BTreeMap::new();
        columns.insert(
            "v".into(),
            Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20)]).expect("col"),
        );
        let frame = DataFrame::new(index, columns).expect("frame");

        let err = write_json_string(&frame, JsonOrient::Columns)
            .expect_err("duplicate JSON object keys should reject");
        assert!(
            matches!(&err, IoError::JsonFormat(msg) if msg.contains("duplicate index label key")),
            "expected duplicate-index-key JsonFormat, got {err:?}"
        );
    }

    #[test]
    fn json_split_read_write_roundtrip() {
        let input = r#"{"columns":["x","y"],"index":["r1","r2","r3"],"data":[[1,4],[2,5],[3,6]]}"#;
        let frame = read_json_str(input, JsonOrient::Split).expect("read json split");
        assert_eq!(frame.index().len(), 3);
        assert_eq!(
            frame.index().labels()[0],
            fp_index::IndexLabel::Utf8("r1".into())
        );
        assert_eq!(frame.column("x").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(frame.column("y").unwrap().values()[2], Scalar::Int64(6));

        let output = write_json_string(&frame, JsonOrient::Split).expect("write");
        let frame2 = read_json_str(&output, JsonOrient::Split).expect("re-read");
        assert_eq!(frame2.index().len(), 3);
        assert_eq!(frame2.index().labels(), frame.index().labels());
    }

    #[test]
    fn json_records_multiindex_roundtrip_restores_logical_row_axis() {
        let frame = make_row_multiindex_test_dataframe();
        let json = write_json_string(&frame, JsonOrient::Records).expect("write");
        let roundtrip = read_json_str(&json, JsonOrient::Records).expect("read");

        assert!(roundtrip.equals(&frame));
        assert!(roundtrip.row_multiindex().is_some());
        assert!(roundtrip.column("__index_level_0__").is_none());
    }

    #[test]
    fn json_split_multiindex_roundtrip_restores_logical_row_axis() {
        let frame = make_row_multiindex_test_dataframe();
        let json = write_json_string(&frame, JsonOrient::Split).expect("write");
        let roundtrip = read_json_str(&json, JsonOrient::Split).expect("read");

        assert!(roundtrip.equals(&frame));
        assert!(roundtrip.row_multiindex().is_some());
        assert!(roundtrip.column("__index_level_0__").is_none());
    }

    #[test]
    fn json_split_without_index_defaults_to_range_index() {
        let input = r#"{"columns":["x"],"data":[[10],[20]]}"#;
        let frame = read_json_str(input, JsonOrient::Split).expect("read json split");
        assert_eq!(frame.index().labels()[0], fp_index::IndexLabel::Int64(0));
        assert_eq!(frame.index().labels()[1], fp_index::IndexLabel::Int64(1));
    }

    #[test]
    fn json_split_index_length_mismatch_errors() {
        let input = r#"{"columns":["x"],"index":[0],"data":[[1],[2]]}"#;
        let err = read_json_str(input, JsonOrient::Split)
            .expect_err("split orient index/data length mismatch should error");
        assert!(
            matches!(&err, IoError::JsonFormat(msg) if msg.contains("index length")),
            "expected split index length error, got {err:?}"
        );
    }

    #[test]
    fn json_split_row_length_mismatch_errors() {
        let input = r#"{"columns":["x","y"],"data":[[1],[2,3]]}"#;
        let err = read_json_str(input, JsonOrient::Split)
            .expect_err("split orient row length mismatch should error");
        assert!(
            matches!(&err, IoError::JsonFormat(msg) if msg.contains("row 0 length")),
            "expected split row length error, got {err:?}"
        );
    }

    #[test]
    fn json_split_non_string_columns_are_stringified() {
        let input = r#"{"columns":[1,true,null,"name"],"data":[[10,20,30,40]]}"#;
        let frame = read_json_str(input, JsonOrient::Split).expect("read json split");
        assert_eq!(frame.column("1").unwrap().values()[0], Scalar::Int64(10));
        assert_eq!(frame.column("true").unwrap().values()[0], Scalar::Int64(20));
        assert_eq!(frame.column("null").unwrap().values()[0], Scalar::Int64(30));
        assert_eq!(frame.column("name").unwrap().values()[0], Scalar::Int64(40));
    }

    #[test]
    fn json_split_duplicate_column_names_error() {
        let input = r#"{"columns":[1,"1"],"data":[[10,20]]}"#;
        let err = read_json_str(input, JsonOrient::Split).expect_err("dup columns");
        assert!(matches!(err, IoError::DuplicateColumnName(name) if name == "1"));
    }

    #[test]
    fn json_index_read_write_roundtrip() {
        let input = r#"{"row_a":{"name":"Alice","age":30},"row_b":{"name":"Bob","age":25}}"#;
        let frame = read_json_str(input, JsonOrient::Index).expect("read json index");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(frame.index().labels()[0], IndexLabel::Utf8("row_a".into()));
        assert_eq!(
            frame.column("name").unwrap().values()[1],
            Scalar::Utf8("Bob".into())
        );

        let output = write_json_string(&frame, JsonOrient::Index).expect("write");
        let frame2 = read_json_str(&output, JsonOrient::Index).expect("re-read");
        assert_eq!(frame2.index().labels(), frame.index().labels());
        assert_eq!(frame2.column("age").unwrap().values()[0], Scalar::Int64(30));
    }

    #[test]
    fn json_index_preserves_column_order() {
        let input = r#"{"r1":{"b":1,"a":2},"r2":{"c":3}}"#;
        let frame = read_json_str(input, JsonOrient::Index).expect("parse");
        let order: Vec<&str> = frame
            .column_names()
            .iter()
            .map(|name| name.as_str())
            .collect();
        assert_eq!(order, vec!["b", "a", "c"]);
    }

    #[test]
    fn json_index_missing_columns_null_fill() {
        let input = r#"{"r1":{"a":1},"r2":{"b":2}}"#;
        let frame = read_json_str(input, JsonOrient::Index).expect("parse");
        let a = frame.column("a").expect("a");
        let b = frame.column("b").expect("b");

        assert_eq!(a.values()[0], Scalar::Int64(1));
        assert!(a.values()[1].is_missing());
        assert!(b.values()[0].is_missing());
        assert_eq!(b.values()[1], Scalar::Int64(2));
    }

    #[test]
    fn json_index_write_duplicate_index_rejects() {
        let index = Index::new(vec![IndexLabel::Int64(1), IndexLabel::Utf8("1".into())]);
        let mut columns = BTreeMap::new();
        columns.insert(
            "v".into(),
            Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20)]).expect("col"),
        );
        let frame = DataFrame::new(index, columns).expect("frame");

        let err = write_json_string(&frame, JsonOrient::Index)
            .expect_err("duplicate JSON object keys should reject");
        assert!(
            matches!(&err, IoError::JsonFormat(msg) if msg.contains("duplicate index label key")),
            "expected duplicate-index-key JsonFormat, got {err:?}"
        );
    }

    #[test]
    fn json_index_read_non_object_row_rejects() {
        let input = r#"{"r1":{"a":1},"r2":[1,2]}"#;
        let err = read_json_str(input, JsonOrient::Index)
            .expect_err("index orient rows must be JSON objects");
        assert!(
            matches!(&err, IoError::JsonFormat(msg) if msg.contains("rows must be objects")),
            "expected row-object error, got {err:?}"
        );
    }

    #[test]
    fn json_values_read_write_roundtrip() {
        let input = r#"[[1,"Alice"],[null,"Bob"]]"#;
        let frame = read_json_str(input, JsonOrient::Values).expect("read json values");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(frame.column_names(), vec!["0", "1"]);
        assert_eq!(frame.column("0").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(
            frame.column("1").unwrap().values()[1],
            Scalar::Utf8("Bob".into())
        );

        let output = write_json_string(&frame, JsonOrient::Values).expect("write");
        let frame2 = read_json_str(&output, JsonOrient::Values).expect("re-read");
        assert_eq!(frame2.index().len(), 2);
        assert_eq!(frame2.column_names(), frame.column_names());
        assert_eq!(
            frame2.column("0").unwrap().values(),
            frame.column("0").unwrap().values()
        );
        assert_eq!(
            frame2.column("1").unwrap().values(),
            frame.column("1").unwrap().values()
        );
    }

    #[test]
    fn json_records_with_nulls() {
        let input = r#"[{"a":1,"b":null},{"a":null,"b":"hello"}]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("parse");
        assert!(frame.column("a").unwrap().values()[1].is_missing());
        assert!(frame.column("b").unwrap().values()[0].is_missing());
    }

    #[test]
    fn json_records_write_promotes_nullable_int_column_to_float() {
        let frame = DataFrame::from_dict_with_index(
            vec![("a", vec![Scalar::Int64(1), Scalar::Null(NullKind::Null)])],
            vec!["row".into(), "row".into()],
        )
        .unwrap();
        let json = write_json_string(&frame, JsonOrient::Records).expect("write");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, serde_json::json!([{"a": 1.0}, {"a": null}]));
    }

    #[test]
    fn json_records_empty_array() {
        let input = r#"[]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("parse");
        assert_eq!(frame.index().len(), 0);
    }

    #[test]
    fn json_records_mixed_numeric_coerces() {
        let input = r#"[{"v":1},{"v":2.5},{"v":true}]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("parse");
        // Int64 + Float64 + Bool all coerce to Float64
        assert_eq!(frame.column("v").unwrap().values()[0], Scalar::Float64(1.0));
        assert_eq!(frame.column("v").unwrap().values()[1], Scalar::Float64(2.5));
        assert_eq!(frame.column("v").unwrap().values()[2], Scalar::Float64(1.0));
    }

    #[test]
    fn json_records_mixed_utf8_numeric_preserves_object_values() {
        let input = r#"[{"v":1},{"v":"text"}]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("parse");
        assert_eq!(
            frame.column("v").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Utf8("text".into())]
        );
    }

    #[test]
    fn file_csv_roundtrip() {
        let input = "a,b\n1,2\n3,4\n";
        let frame = read_csv_str(input).expect("parse");

        let dir = std::env::temp_dir();
        let path = dir.join("fp_io_test_roundtrip.csv");
        super::write_csv(&frame, &path).expect("write file");
        let frame2 = super::read_csv(&path).expect("read file");
        assert_eq!(frame2.index().len(), 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_csv_with_options_path() {
        let input = "id\tval\na\tNA\nb\t2\n";
        let dir = std::env::temp_dir();
        let path = dir.join("fp_io_test_options.csv");
        std::fs::write(&path, input).expect("write fixture");

        let options = CsvReadOptions {
            delimiter: b'\t',
            na_values: vec!["NA".into()],
            index_col: Some("id".into()),
            ..Default::default()
        };

        let frame = super::read_csv_with_options_path(&path, &options).expect("read with options");
        assert_eq!(
            frame.index().labels()[0],
            fp_index::IndexLabel::Utf8("a".into())
        );
        assert!(frame.column("id").is_none());
        assert!(frame.column("val").unwrap().values()[0].is_missing());
        assert_eq!(
            frame.column("val").unwrap().values()[1],
            Scalar::Float64(2.0)
        );

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_json_roundtrip() {
        let input = r#"[{"x":1},{"x":2}]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("parse");

        let dir = std::env::temp_dir();
        let path = dir.join("fp_io_test_roundtrip.json");
        super::write_json(&frame, &path, JsonOrient::Records).expect("write file");
        let frame2 = super::read_json(&path, JsonOrient::Records).expect("read file");
        assert_eq!(frame2.index().len(), 2);
        std::fs::remove_file(&path).ok();
    }

    // ── Parquet I/O tests ──────────────────────────────────────────────

    fn make_test_dataframe() -> DataFrame {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "ints".to_string(),
            Column::new(
                DType::Int64,
                vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
            )
            .unwrap(),
        );
        columns.insert(
            "floats".to_string(),
            Column::new(
                DType::Float64,
                vec![
                    Scalar::Float64(1.5),
                    Scalar::Float64(2.5),
                    Scalar::Float64(3.5),
                ],
            )
            .unwrap(),
        );
        columns.insert(
            "names".to_string(),
            Column::from_values(vec![
                Scalar::Utf8("alice".into()),
                Scalar::Utf8("bob".into()),
                Scalar::Utf8("carol".into()),
            ])
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        DataFrame::new_with_column_order(
            Index::new(labels),
            columns,
            vec![
                "ints".to_string(),
                "floats".to_string(),
                "names".to_string(),
            ],
        )
        .unwrap()
    }

    fn make_row_multiindex_test_dataframe() -> DataFrame {
        let df = DataFrame::from_dict(
            &["region", "product", "year", "sales", "cost"],
            vec![
                (
                    "region",
                    vec![
                        Scalar::Utf8("north".into()),
                        Scalar::Utf8("north".into()),
                        Scalar::Utf8("south".into()),
                    ],
                ),
                (
                    "product",
                    vec![
                        Scalar::Utf8("apple".into()),
                        Scalar::Utf8("pear".into()),
                        Scalar::Utf8("apple".into()),
                    ],
                ),
                (
                    "year",
                    vec![
                        Scalar::Int64(2023),
                        Scalar::Int64(2024),
                        Scalar::Int64(2023),
                    ],
                ),
                (
                    "sales",
                    vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
                ),
                (
                    "cost",
                    vec![Scalar::Int64(4), Scalar::Int64(7), Scalar::Int64(12)],
                ),
            ],
        )
        .unwrap();
        df.set_index_multi(&["region", "product", "year"], true, "|")
            .unwrap()
    }

    #[test]
    fn parquet_bytes_roundtrip() {
        let frame = make_test_dataframe();
        let bytes = super::write_parquet_bytes(&frame).expect("write parquet");
        assert!(!bytes.is_empty());

        let frame2 = super::read_parquet_bytes(&bytes).expect("read parquet");
        assert_eq!(frame2.index().len(), 3);
        assert_eq!(
            frame2
                .column_names()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            vec!["ints", "floats", "names"]
        );

        // Check values round-tripped correctly
        let ints = frame2.column("ints").unwrap();
        assert_eq!(ints.values()[0], Scalar::Int64(10));
        assert_eq!(ints.values()[1], Scalar::Int64(20));
        assert_eq!(ints.values()[2], Scalar::Int64(30));

        let floats = frame2.column("floats").unwrap();
        assert_eq!(floats.values()[0], Scalar::Float64(1.5));
        assert_eq!(floats.values()[1], Scalar::Float64(2.5));
        assert_eq!(floats.values()[2], Scalar::Float64(3.5));

        let names = frame2.column("names").unwrap();
        assert_eq!(names.values()[0], Scalar::Utf8("alice".into()));
        assert_eq!(names.values()[1], Scalar::Utf8("bob".into()));
        assert_eq!(names.values()[2], Scalar::Utf8("carol".into()));
    }

    #[test]
    fn parquet_row_multiindex_roundtrip_restores_logical_row_axis() {
        let frame = make_row_multiindex_test_dataframe();
        let bytes = super::write_parquet_bytes(&frame).expect("write parquet");
        let roundtrip = super::read_parquet_bytes(&bytes).expect("read parquet");

        assert!(roundtrip.equals(&frame));
        assert!(roundtrip.column("__index_level_0__").is_none());
        assert_eq!(
            roundtrip
                .row_multiindex()
                .expect("row multiindex should be restored")
                .get_level_values(0)
                .unwrap()
                .labels(),
            frame
                .row_multiindex()
                .expect("source row multiindex")
                .get_level_values(0)
                .unwrap()
                .labels()
        );
    }

    #[test]
    fn parquet_file_roundtrip() {
        let frame = make_test_dataframe();
        let dir = std::env::temp_dir();
        let path = dir.join("fp_io_test_parquet_roundtrip.parquet");

        super::write_parquet(&frame, &path).expect("write parquet file");
        let frame2 = super::read_parquet(&path).expect("read parquet file");
        assert_eq!(frame2.index().len(), 3);
        assert_eq!(
            frame2.column("ints").unwrap().values()[0],
            Scalar::Int64(10)
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn parquet_with_nulls() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "vals".to_string(),
            Column::new(
                DType::Float64,
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
        );
        columns.insert(
            "strs".to_string(),
            Column::from_values(vec![
                Scalar::Utf8("a".into()),
                Scalar::Null(NullKind::Null),
                Scalar::Utf8("c".into()),
            ])
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame = DataFrame::new_with_column_order(
            Index::new(labels),
            columns,
            vec!["vals".to_string(), "strs".to_string()],
        )
        .unwrap();

        let bytes = super::write_parquet_bytes(&frame).expect("write");
        let frame2 = super::read_parquet_bytes(&bytes).expect("read");

        assert_eq!(
            frame2.column("vals").unwrap().values()[0],
            Scalar::Float64(1.0)
        );
        assert!(frame2.column("vals").unwrap().values()[1].is_missing());
        assert_eq!(
            frame2.column("vals").unwrap().values()[2],
            Scalar::Float64(3.0)
        );

        assert_eq!(
            frame2.column("strs").unwrap().values()[0],
            Scalar::Utf8("a".into())
        );
        assert!(frame2.column("strs").unwrap().values()[1].is_missing());
        assert_eq!(
            frame2.column("strs").unwrap().values()[2],
            Scalar::Utf8("c".into())
        );
    }

    #[test]
    fn parquet_bool_column() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "flags".to_string(),
            Column::new(
                DType::Bool,
                vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
            )
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame = DataFrame::new_with_column_order(
            Index::new(labels),
            columns,
            vec!["flags".to_string()],
        )
        .unwrap();

        let bytes = super::write_parquet_bytes(&frame).expect("write");
        let frame2 = super::read_parquet_bytes(&bytes).expect("read");

        assert_eq!(
            frame2.column("flags").unwrap().values()[0],
            Scalar::Bool(true)
        );
        assert_eq!(
            frame2.column("flags").unwrap().values()[1],
            Scalar::Bool(false)
        );
        assert_eq!(
            frame2.column("flags").unwrap().values()[2],
            Scalar::Bool(true)
        );
    }

    #[test]
    fn parquet_empty_dataframe_errors() {
        // Parquet format requires at least one column — empty DataFrames
        // cannot be represented, matching pandas behavior where
        // pd.DataFrame().to_parquet() also fails.
        let frame =
            DataFrame::new_with_column_order(Index::new(vec![]), BTreeMap::new(), vec![]).unwrap();

        let result = super::write_parquet_bytes(&frame);
        assert!(result.is_err());
    }

    // ── Excel I/O tests ──────────────────────────────────────────────

    #[test]
    fn write_excel_with_options_custom_sheet_name_survives_round_trip() {
        let frame = make_test_dataframe();
        let bytes = super::write_excel_bytes_with_options(
            &frame,
            &super::ExcelWriteOptions {
                sheet_name: "Results".to_string(),
                ..super::ExcelWriteOptions::default()
            },
        )
        .expect("write");
        let sheets =
            super::read_excel_sheets_bytes(&bytes, None, &super::ExcelReadOptions::default())
                .expect("read");
        assert_eq!(sheets.len(), 1);
        assert!(sheets.contains_key("Results"));
    }

    #[test]
    fn write_excel_with_options_index_false_omits_index_column() {
        let frame = make_test_dataframe();
        let bytes = super::write_excel_bytes_with_options(
            &frame,
            &super::ExcelWriteOptions {
                index: false,
                ..super::ExcelWriteOptions::default()
            },
        )
        .expect("write");
        let frame2 =
            super::read_excel_bytes(&bytes, &super::ExcelReadOptions::default()).expect("read");
        // With index=false the first column is "ints" directly (no
        // anonymous leading index column).
        let names = frame2.column_names();
        assert_eq!(
            names.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            vec!["ints", "floats", "names"]
        );
    }

    #[test]
    fn write_excel_with_options_index_label_overrides_header() {
        let frame = make_test_dataframe();
        let bytes = super::write_excel_bytes_with_options(
            &frame,
            &super::ExcelWriteOptions {
                index_label: Some("row_id".to_string()),
                ..super::ExcelWriteOptions::default()
            },
        )
        .expect("write");
        let frame2 =
            super::read_excel_bytes(&bytes, &super::ExcelReadOptions::default()).expect("read");
        // The index column now shows up as "row_id" before the data columns.
        let names = frame2.column_names();
        assert_eq!(names[0], "row_id");
    }

    #[test]
    fn write_excel_with_options_header_false_omits_header_row() {
        let frame = make_test_dataframe();
        let bytes = super::write_excel_bytes_with_options(
            &frame,
            &super::ExcelWriteOptions {
                header: false,
                index: false,
                ..super::ExcelWriteOptions::default()
            },
        )
        .expect("write");
        // Without header, the reader treats row 0 as headers. We
        // expect the first data row to become the column names
        // instead of literal "ints"/"floats"/"names".
        let frame2 =
            super::read_excel_bytes(&bytes, &super::ExcelReadOptions::default()).expect("read");
        let names = frame2.column_names();
        let name_strs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        assert!(!name_strs.contains(&"ints"));
    }

    #[test]
    fn write_excel_with_options_default_matches_write_excel_bytes() {
        let frame = make_test_dataframe();
        let default_bytes = super::write_excel_bytes(&frame).expect("default");
        let options_bytes =
            super::write_excel_bytes_with_options(&frame, &super::ExcelWriteOptions::default())
                .expect("options");
        assert_eq!(default_bytes, options_bytes);
    }

    #[test]
    fn excel_multiindex_roundtrip_with_explicit_index_cols() {
        let frame = make_row_multiindex_test_dataframe();
        let bytes =
            super::write_excel_bytes_with_options(&frame, &super::ExcelWriteOptions::default())
                .expect("write");
        let roundtrip = super::read_excel_bytes_with_index_cols(
            &bytes,
            &super::ExcelReadOptions::default(),
            &["region", "product", "year"],
        )
        .expect("read");

        assert!(roundtrip.equals(&frame));
        assert_eq!(roundtrip.row_multiindex(), frame.row_multiindex());
    }

    fn build_two_sheet_workbook_bytes() -> Vec<u8> {
        use rust_xlsxwriter::Workbook;
        let mut workbook = Workbook::new();
        let sheet1 = workbook.add_worksheet();
        sheet1.set_name("Alpha").expect("sheet name");
        sheet1.write_string(0, 0, "a").expect("header");
        sheet1.write_string(0, 1, "b").expect("header");
        sheet1.write_number(1, 0, 1.0).expect("data");
        sheet1.write_number(1, 1, 10.0).expect("data");
        sheet1.write_number(2, 0, 2.0).expect("data");
        sheet1.write_number(2, 1, 20.0).expect("data");

        let sheet2 = workbook.add_worksheet();
        sheet2.set_name("Bravo").expect("sheet name");
        sheet2.write_string(0, 0, "name").expect("header");
        sheet2.write_string(1, 0, "alice").expect("data");
        sheet2.write_string(2, 0, "bob").expect("data");

        let sheet3 = workbook.add_worksheet();
        sheet3.set_name("Charlie").expect("sheet name");
        sheet3.write_string(0, 0, "x").expect("header");
        sheet3.write_number(1, 0, 99.0).expect("data");

        workbook.save_to_buffer().expect("save")
    }

    #[test]
    fn read_excel_sheets_ordered_bytes_preserves_workbook_order() {
        // Workbook sheet order: Alpha, Bravo, Charlie. A sorted map
        // would still give Alpha/Bravo/Charlie alphabetically — but
        // pandas guarantees workbook order regardless of alphabetic
        // relationship, so this test uses a fixture where the ordered
        // result differs from sorted order.
        use rust_xlsxwriter::Workbook;
        let mut workbook = Workbook::new();
        let s1 = workbook.add_worksheet();
        s1.set_name("Zulu").expect("name");
        s1.write_string(0, 0, "v").expect("header");
        s1.write_number(1, 0, 1.0).expect("data");
        let s2 = workbook.add_worksheet();
        s2.set_name("Alpha").expect("name");
        s2.write_string(0, 0, "v").expect("header");
        s2.write_number(1, 0, 2.0).expect("data");
        let s3 = workbook.add_worksheet();
        s3.set_name("Mike").expect("name");
        s3.write_string(0, 0, "v").expect("header");
        s3.write_number(1, 0, 3.0).expect("data");
        let bytes = workbook.save_to_buffer().expect("save");

        let ordered = super::read_excel_sheets_ordered_bytes(
            &bytes,
            None,
            &super::ExcelReadOptions::default(),
        )
        .expect("read ordered");
        assert_eq!(
            ordered.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            vec!["Zulu", "Alpha", "Mike"],
            "ordered form preserves workbook order"
        );

        // Sorted form alphabetizes (existing contract for BTreeMap).
        let sorted =
            super::read_excel_sheets_bytes(&bytes, None, &super::ExcelReadOptions::default())
                .expect("read sorted");
        assert_eq!(
            sorted.keys().map(String::as_str).collect::<Vec<_>>(),
            vec!["Alpha", "Mike", "Zulu"],
            "BTreeMap form alphabetizes"
        );
    }

    #[test]
    fn read_excel_sheets_ordered_bytes_selected_subset_keeps_caller_order() {
        let bytes = build_two_sheet_workbook_bytes();
        // Caller-specified order: Charlie, Alpha — deliberately reversed
        // from workbook order. Pandas docs say sheet_name=[list] returns
        // a dict whose iteration reflects the argument order; we match.
        let req = vec!["Charlie".to_string(), "Alpha".to_string()];
        let ordered = super::read_excel_sheets_ordered_bytes(
            &bytes,
            Some(&req),
            &super::ExcelReadOptions::default(),
        )
        .expect("ordered subset");
        assert_eq!(
            ordered.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            vec!["Charlie", "Alpha"]
        );
    }

    #[test]
    fn read_excel_sheets_ordered_path_matches_bytes() {
        let bytes = build_two_sheet_workbook_bytes();
        let temp = std::env::temp_dir().join("fp_io_wrt3_ordered.xlsx");
        std::fs::write(&temp, &bytes).expect("write temp");
        let via_path =
            super::read_excel_sheets_ordered(&temp, None, &super::ExcelReadOptions::default())
                .expect("read path");
        let via_bytes = super::read_excel_sheets_ordered_bytes(
            &bytes,
            None,
            &super::ExcelReadOptions::default(),
        )
        .expect("read bytes");
        assert_eq!(
            via_path.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>(),
            via_bytes.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn read_excel_sheets_bytes_all_sheets_returns_map() {
        let bytes = build_two_sheet_workbook_bytes();
        let sheets =
            super::read_excel_sheets_bytes(&bytes, None, &super::ExcelReadOptions::default())
                .expect("read sheets");
        assert_eq!(sheets.len(), 3);
        assert!(sheets.contains_key("Alpha"));
        assert!(sheets.contains_key("Bravo"));
        assert!(sheets.contains_key("Charlie"));

        let alpha = &sheets["Alpha"];
        assert_eq!(alpha.index().len(), 2);
        assert_eq!(alpha.column_names().len(), 2);

        let bravo = &sheets["Bravo"];
        assert_eq!(bravo.index().len(), 2);
        assert_eq!(
            bravo.column("name").unwrap().values()[0],
            Scalar::Utf8("alice".into())
        );
    }

    #[test]
    fn read_excel_sheets_bytes_selects_subset() {
        let bytes = build_two_sheet_workbook_bytes();
        let selected = vec!["Alpha".to_string(), "Charlie".to_string()];
        let sheets = super::read_excel_sheets_bytes(
            &bytes,
            Some(&selected),
            &super::ExcelReadOptions::default(),
        )
        .expect("read subset");
        assert_eq!(sheets.len(), 2);
        assert!(sheets.contains_key("Alpha"));
        assert!(sheets.contains_key("Charlie"));
        assert!(!sheets.contains_key("Bravo"));
    }

    #[test]
    fn read_excel_sheets_bytes_unknown_sheet_errors() {
        let bytes = build_two_sheet_workbook_bytes();
        let bogus = vec!["Zeta".to_string()];
        let err = super::read_excel_sheets_bytes(
            &bytes,
            Some(&bogus),
            &super::ExcelReadOptions::default(),
        )
        .unwrap_err();
        assert!(matches!(err, super::IoError::Excel(_)));
    }

    #[test]
    fn read_excel_sheets_path_matches_bytes() {
        let bytes = build_two_sheet_workbook_bytes();
        let temp = std::env::temp_dir().join("fp_io_9my2_multisheet.xlsx");
        std::fs::write(&temp, &bytes).expect("write temp");
        let via_path = super::read_excel_sheets(&temp, None, &super::ExcelReadOptions::default())
            .expect("read path");
        let via_bytes =
            super::read_excel_sheets_bytes(&bytes, None, &super::ExcelReadOptions::default())
                .expect("read bytes");
        assert_eq!(
            via_path.keys().collect::<Vec<_>>(),
            via_bytes.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn excel_bytes_roundtrip() {
        let frame = make_test_dataframe();
        let bytes = super::write_excel_bytes(&frame).expect("write excel");
        assert!(!bytes.is_empty());

        let frame2 = super::read_excel_bytes(
            &bytes,
            &super::ExcelReadOptions {
                index_col: Some("column_0".into()),
                ..Default::default()
            },
        )
        .expect("read excel");
        assert_eq!(frame2.index().len(), 3);
        assert_eq!(frame2.index().labels(), frame.index().labels());
        assert_eq!(frame2.index().name(), None);
        // Excel preserves the write-time column order (ints, floats, names).
        assert_eq!(
            frame2
                .column_names()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            vec!["ints", "floats", "names"]
        );

        // Int values survive round-trip (Excel stores as f64, we recover Int64).
        let ints = frame2.column("ints").unwrap();
        assert_eq!(ints.values()[0], Scalar::Int64(10));
        assert_eq!(ints.values()[1], Scalar::Int64(20));
        assert_eq!(ints.values()[2], Scalar::Int64(30));

        // Float values survive round-trip.
        let floats = frame2.column("floats").unwrap();
        assert_eq!(floats.values()[0], Scalar::Float64(1.5));
        assert_eq!(floats.values()[1], Scalar::Float64(2.5));
        assert_eq!(floats.values()[2], Scalar::Float64(3.5));

        // String values survive round-trip.
        let names = frame2.column("names").unwrap();
        assert_eq!(names.values()[0], Scalar::Utf8("alice".into()));
        assert_eq!(names.values()[1], Scalar::Utf8("bob".into()));
        assert_eq!(names.values()[2], Scalar::Utf8("carol".into()));
    }

    #[test]
    fn excel_file_roundtrip() {
        let frame = make_test_dataframe();
        let dir = std::env::temp_dir();
        let path = dir.join("fp_io_test_excel_roundtrip.xlsx");

        super::write_excel(&frame, &path).expect("write excel file");
        let frame2 = super::read_excel(
            &path,
            &super::ExcelReadOptions {
                index_col: Some("column_0".into()),
                ..Default::default()
            },
        )
        .expect("read excel file");
        assert_eq!(frame2.index().len(), 3);
        assert_eq!(frame2.index().labels(), frame.index().labels());
        assert_eq!(
            frame2.column("ints").unwrap().values()[0],
            Scalar::Int64(10)
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn excel_with_nulls() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "vals".to_string(),
            Column::new(
                DType::Float64,
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame =
            DataFrame::new_with_column_order(Index::new(labels), columns, vec!["vals".to_string()])
                .unwrap();

        let bytes = super::write_excel_bytes(&frame).expect("write");
        let frame2 = super::read_excel_bytes(
            &bytes,
            &super::ExcelReadOptions {
                index_col: Some("column_0".into()),
                ..Default::default()
            },
        )
        .expect("read");

        // Non-null values round-trip.
        assert_eq!(frame2.column("vals").unwrap().values()[0], Scalar::Int64(1));
        // NaN written as empty cell, read back as Null.
        assert!(frame2.column("vals").unwrap().values()[1].is_missing());
        assert_eq!(frame2.column("vals").unwrap().values()[2], Scalar::Int64(3));
    }

    #[test]
    fn excel_bool_column() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "flags".to_string(),
            Column::new(
                DType::Bool,
                vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
            )
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame = DataFrame::new_with_column_order(
            Index::new(labels),
            columns,
            vec!["flags".to_string()],
        )
        .unwrap();

        let bytes = super::write_excel_bytes(&frame).expect("write");
        let frame2 = super::read_excel_bytes(
            &bytes,
            &super::ExcelReadOptions {
                index_col: Some("column_0".into()),
                ..Default::default()
            },
        )
        .expect("read");

        assert_eq!(
            frame2.column("flags").unwrap().values()[0],
            Scalar::Bool(true)
        );
        assert_eq!(
            frame2.column("flags").unwrap().values()[1],
            Scalar::Bool(false)
        );
        assert_eq!(
            frame2.column("flags").unwrap().values()[2],
            Scalar::Bool(true)
        );
    }

    #[test]
    fn excel_skip_rows() {
        // Build an xlsx with 5 data rows, then read with skip_rows=2 to skip
        // 2 rows before the header.
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "x".to_string(),
            Column::new(DType::Int64, vec![Scalar::Int64(1), Scalar::Int64(2)]).unwrap(),
        );
        let labels = vec![IndexLabel::Int64(0), IndexLabel::Int64(1)];
        let frame =
            DataFrame::new_with_column_order(Index::new(labels), columns, vec!["x".to_string()])
                .unwrap();

        let bytes = super::write_excel_bytes(&frame).expect("write");
        let frame2 = super::read_excel_bytes(
            &bytes,
            &super::ExcelReadOptions {
                skip_rows: 1,
                has_headers: false,
                ..Default::default()
            },
        )
        .expect("read with skip");

        // Skipped the header row, so first data row becomes first row.
        // With has_headers=false, column names are auto-generated.
        assert_eq!(frame2.index().len(), 2);
        assert!(frame2.column("column_0").is_some());
    }

    #[test]
    fn excel_header_none_with_explicit_names_uses_names_and_keeps_first_row() {
        let rows = vec![
            vec![
                calamine::Data::Int(1),
                calamine::Data::String("alpha".to_owned()),
            ],
            vec![
                calamine::Data::Int(2),
                calamine::Data::String("beta".to_owned()),
            ],
        ];

        let frame = super::parse_excel_rows(
            rows,
            &super::ExcelReadOptions {
                has_headers: false,
                names: Some(vec!["id".to_owned(), "label".to_owned()]),
                ..Default::default()
            },
        )
        .expect("parse excel rows with explicit names");

        assert_eq!(frame.column_names(), vec!["id", "label"]);
        assert_eq!(frame.index().len(), 2);
        assert_eq!(frame.column("id").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(
            frame.column("label").unwrap().values()[0],
            Scalar::Utf8("alpha".into())
        );
        assert_eq!(frame.column("id").unwrap().values()[1], Scalar::Int64(2));
    }

    #[test]
    fn excel_header_none_with_explicit_names_preserves_index_name() {
        let rows = vec![
            vec![
                calamine::Data::Int(10),
                calamine::Data::String("alpha".to_owned()),
            ],
            vec![
                calamine::Data::Int(20),
                calamine::Data::String("beta".to_owned()),
            ],
        ];

        let frame = super::parse_excel_rows(
            rows,
            &super::ExcelReadOptions {
                has_headers: false,
                names: Some(vec!["row_id".to_owned(), "value".to_owned()]),
                index_col: Some("row_id".to_owned()),
                ..Default::default()
            },
        )
        .expect("parse excel rows with named index column");

        assert_eq!(frame.index().name(), Some("row_id"));
        assert_eq!(frame.index().labels()[0], IndexLabel::Int64(10));
        assert_eq!(frame.index().labels()[1], IndexLabel::Int64(20));
        assert!(frame.column("row_id").is_none());
        assert_eq!(
            frame.column("value").unwrap().values(),
            &[Scalar::Utf8("alpha".into()), Scalar::Utf8("beta".into())]
        );
    }

    #[test]
    fn excel_explicit_names_width_mismatch_errors() {
        let rows = vec![vec![calamine::Data::Int(1), calamine::Data::Int(2)]];

        let err = super::parse_excel_rows(
            rows,
            &super::ExcelReadOptions {
                has_headers: false,
                names: Some(vec!["only_one".to_owned()]),
                ..Default::default()
            },
        )
        .expect_err("names width mismatch should error");

        assert!(
            matches!(err, IoError::Excel(message) if message.contains("expected 2 column names, got 1"))
        );
    }

    #[test]
    fn excel_usecols_selects_subset_in_sheet_order() {
        let rows = vec![
            vec![
                calamine::Data::String("a".to_owned()),
                calamine::Data::String("b".to_owned()),
                calamine::Data::String("c".to_owned()),
            ],
            vec![
                calamine::Data::Int(1),
                calamine::Data::Int(2),
                calamine::Data::Int(3),
            ],
        ];

        let frame = super::parse_excel_rows(
            rows,
            &super::ExcelReadOptions {
                usecols: Some(vec!["c".to_owned(), "a".to_owned()]),
                ..Default::default()
            },
        )
        .expect("parse excel rows with usecols");

        assert_eq!(frame.column_names(), vec!["a", "c"]);
        assert_eq!(frame.column("a").unwrap().values(), &[Scalar::Int64(1)]);
        assert_eq!(frame.column("c").unwrap().values(), &[Scalar::Int64(3)]);
        assert!(frame.column("b").is_none());
    }

    #[test]
    fn excel_usecols_with_explicit_names_filters_renamed_columns() {
        let rows = vec![
            vec![
                calamine::Data::Int(1),
                calamine::Data::String("alpha".to_owned()),
            ],
            vec![
                calamine::Data::Int(2),
                calamine::Data::String("beta".to_owned()),
            ],
        ];

        let frame = super::parse_excel_rows(
            rows,
            &super::ExcelReadOptions {
                has_headers: false,
                names: Some(vec!["id".to_owned(), "label".to_owned()]),
                usecols: Some(vec!["label".to_owned()]),
                ..Default::default()
            },
        )
        .expect("parse headerless excel rows with names and usecols");

        assert_eq!(frame.column_names(), vec!["label"]);
        assert_eq!(
            frame.column("label").unwrap().values(),
            &[Scalar::Utf8("alpha".into()), Scalar::Utf8("beta".into())]
        );
        assert!(frame.column("id").is_none());
    }

    #[test]
    fn excel_usecols_missing_column_errors() {
        let rows = vec![
            vec![
                calamine::Data::String("a".to_owned()),
                calamine::Data::String("b".to_owned()),
            ],
            vec![calamine::Data::Int(1), calamine::Data::Int(2)],
        ];

        let err = super::parse_excel_rows(
            rows,
            &super::ExcelReadOptions {
                usecols: Some(vec!["missing".to_owned()]),
                ..Default::default()
            },
        )
        .expect_err("missing excel usecols should error");

        assert!(
            matches!(err, IoError::MissingUsecols(missing) if missing == vec!["missing".to_owned()])
        );
    }

    #[test]
    fn excel_default_read_promotes_writer_range_index_back_to_index() {
        let frame = make_test_dataframe();
        let bytes = super::write_excel_bytes(&frame).expect("write excel");

        let frame2 = super::read_excel_bytes(&bytes, &super::ExcelReadOptions::default())
            .expect("read excel");

        assert_eq!(frame2.index().labels(), frame.index().labels());
        assert_eq!(frame2.index().name(), None);
        assert_eq!(frame2.column_names(), vec!["ints", "floats", "names"],);
        assert!(frame2.column("column_0").is_none());
    }

    #[test]
    fn excel_default_read_keeps_non_range_generated_leading_column_as_data() {
        let rows = vec![
            vec![
                calamine::Data::Empty,
                calamine::Data::String("value".to_owned()),
            ],
            vec![calamine::Data::Int(10), calamine::Data::Int(1)],
            vec![calamine::Data::Int(20), calamine::Data::Int(2)],
        ];

        let frame = super::parse_excel_rows(rows, &super::ExcelReadOptions::default())
            .expect("parse excel rows");

        assert_eq!(
            frame.index().labels(),
            &[IndexLabel::Int64(0), IndexLabel::Int64(1)]
        );
        assert_eq!(frame.column_names(), vec!["column_0", "value"]);
        assert_eq!(
            frame.column("column_0").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20)],
        );
    }

    #[test]
    fn excel_named_index_roundtrip_preserves_index_name() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "vals".to_string(),
            Column::new(DType::Int64, vec![Scalar::Int64(10), Scalar::Int64(20)]).unwrap(),
        );

        let frame = DataFrame::new_with_column_order(
            Index::new(vec![IndexLabel::Int64(10), IndexLabel::Int64(20)]).set_name("row_id"),
            columns,
            vec!["vals".to_string()],
        )
        .unwrap();

        let bytes = super::write_excel_bytes(&frame).expect("write excel");
        let frame2 = super::read_excel_bytes(
            &bytes,
            &super::ExcelReadOptions {
                index_col: Some("row_id".into()),
                ..Default::default()
            },
        )
        .expect("read excel");

        assert_eq!(frame2.index().labels(), frame.index().labels());
        assert_eq!(frame2.index().name(), Some("row_id"));
        assert!(frame2.column("row_id").is_none());
        assert_eq!(
            frame2.column("vals").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20)]
        );
    }

    #[test]
    fn excel_duplicate_headers_error() {
        let rows = vec![
            vec![
                calamine::Data::String("dup".to_owned()),
                calamine::Data::String("dup".to_owned()),
            ],
            vec![calamine::Data::Int(1), calamine::Data::Int(2)],
        ];

        let err = super::parse_excel_rows(rows, &super::ExcelReadOptions::default())
            .expect_err("duplicate headers should error");
        assert!(matches!(err, IoError::DuplicateColumnName(_)));
    }

    // ── SQL I/O tests ──────────────────────────────────────────────
    //
    // Per br-frankenpandas-7a49 (fd90.48): keep the import block
    // unconditional so stub-backend tests (which only need types and
    // free-fns, not rusqlite) compile under --no-default-features.
    // `#[allow(unused_imports)]` covers the few free fns that are
    // exclusively used inside the cfg-gated SQLite-backed tests.

    // Per fd90.48: TYPE imports + introspection-helper free fns are
    // used by both stub-backend tests (which compile under
    // --no-default-features) and SQLite-backed tests. The
    // read_sql_* / write_sql_* row-materialization free fns are only
    // exercised inside SQLite-backed tests (cfg-gated below) so they
    // get their own gated import group to avoid unused warnings.
    // Per fd90.48: TYPE imports + introspection-helper free fns +
    // write_sql / write_sql_with_options are used by both stub-backend
    // tests (which compile under --no-default-features) and
    // SQLite-backed tests. The read_sql_* row-materialization free fns
    // are only exercised inside SQLite-backed tests (cfg-gated below).
    use super::{
        SqlColumnSchema, SqlForeignKeySchema, SqlIfExists, SqlIndexSchema, SqlInsertMethod,
        SqlInspector, SqlQueryResult, SqlReadOptions, SqlReflectedTable, SqlTableSchema,
        SqlUniqueConstraintSchema, SqlWriteOptions, list_sql_foreign_keys,
        list_sql_indexes, list_sql_schemas, list_sql_tables, list_sql_unique_constraints,
        list_sql_views, sql_max_identifier_length, sql_primary_key_columns,
        sql_server_version, sql_table_comment, sql_table_schema, truncate_sql_table,
        write_sql, write_sql_with_options,
    };
    #[cfg(feature = "sql-sqlite")]
    use super::{
        read_sql, read_sql_chunks, read_sql_chunks_with_index_col,
        read_sql_chunks_with_options, read_sql_chunks_with_options_and_index_col,
        read_sql_query, read_sql_query_chunks, read_sql_query_chunks_with_index_col,
        read_sql_query_chunks_with_options,
        read_sql_query_chunks_with_options_and_index_col, read_sql_query_with_index_col,
        read_sql_query_with_options, read_sql_table, read_sql_table_chunks,
        read_sql_table_chunks_with_index_col, read_sql_table_chunks_with_options,
        read_sql_table_chunks_with_options_and_index_col, read_sql_table_columns,
        read_sql_table_columns_chunks, read_sql_table_columns_chunks_with_index_col,
        read_sql_table_columns_with_index_col, read_sql_table_with_index_col,
        read_sql_table_with_options, read_sql_table_with_options_and_index_col,
        read_sql_with_index_col, read_sql_with_options,
    };

    // Per br-frankenpandas-7a49 (fd90.48): the helper itself only
    // exists when sql-sqlite is on, since it directly references
    // rusqlite::Connection. All tests that call this are also
    // cfg-gated on the same feature.
    #[cfg(feature = "sql-sqlite")]
    fn make_sql_test_conn() -> rusqlite::Connection {
        rusqlite::Connection::open_in_memory().expect("in-memory sqlite")
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_with_index_col_promotes_named_column() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "indexed_tbl", SqlIfExists::Fail).expect("write");

        // Promote the "ints" column to the row index. The data
        // columns should drop ints and the index labels should be the
        // ints values.
        let result = read_sql_table_with_index_col(&conn, "indexed_tbl", Some("ints"))
            .expect("read with index");
        assert_eq!(result.index().name(), Some("ints"));
        assert_eq!(result.index().labels()[0], crate::IndexLabel::Int64(10));
        assert_eq!(result.index().labels()[1], crate::IndexLabel::Int64(20));
        assert_eq!(result.index().labels()[2], crate::IndexLabel::Int64(30));
        // Data columns: only the non-index columns remain.
        let names: Vec<&str> = result.column_names().iter().map(|s| s.as_str()).collect();
        assert!(!names.contains(&"ints"));
        assert!(names.contains(&"floats"));
        assert!(names.contains(&"names"));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_with_index_col_none_is_unchanged() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "noindex_tbl", SqlIfExists::Fail).expect("write");
        let baseline = read_sql_table(&conn, "noindex_tbl").expect("baseline");
        let result =
            read_sql_table_with_index_col(&conn, "noindex_tbl", None).expect("noop variant");
        assert_eq!(result.index().labels(), baseline.index().labels());
        assert_eq!(result.column_names(), baseline.column_names());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_with_index_col_unknown_column_errors() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "missing_tbl", SqlIfExists::Fail).expect("write");
        let err = read_sql_table_with_index_col(&conn, "missing_tbl", Some("nope")).unwrap_err();
        assert!(matches!(err, crate::IoError::Sql(_)));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_returns_requested_projection_in_order() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "proj_tbl", SqlIfExists::Fail).expect("write");

        let result = read_sql_table_columns(&conn, "proj_tbl", &["names", "ints"])
            .expect("subset projection");
        let names: Vec<&str> = result.column_names().iter().map(|s| s.as_str()).collect();
        assert_eq!(names, vec!["names", "ints"]);
        assert_eq!(result.index().len(), 3);
        assert_eq!(
            result.column("ints").unwrap().values()[0],
            Scalar::Int64(10)
        );
        assert_eq!(
            result.column("names").unwrap().values()[2],
            Scalar::Utf8("carol".into())
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_single_column_projection() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "single_tbl", SqlIfExists::Fail).expect("write");

        let result =
            read_sql_table_columns(&conn, "single_tbl", &["floats"]).expect("single projection");
        let names: Vec<&str> = result.column_names().iter().map(|s| s.as_str()).collect();
        assert_eq!(names, vec!["floats"]);
        assert_eq!(
            result.column("floats").unwrap().values()[1],
            Scalar::Float64(2.5)
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_rejects_empty_columns() {
        let conn = make_sql_test_conn();
        let err = read_sql_table_columns(&conn, "any_tbl", &[]).unwrap_err();
        assert!(matches!(err, crate::IoError::Sql(_)));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_rejects_invalid_column_name() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "valid_tbl", SqlIfExists::Fail).expect("write");
        let err = read_sql_table_columns(&conn, "valid_tbl", &["ints; DROP TABLE valid_tbl"])
            .unwrap_err();
        assert!(matches!(err, crate::IoError::Sql(_)));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_rejects_invalid_table_name() {
        let conn = make_sql_test_conn();
        let err = read_sql_table_columns(&conn, "bad table", &["ints"]).unwrap_err();
        assert!(matches!(err, crate::IoError::Sql(_)));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_chunks_returns_requested_projection_in_order() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "proj_chunk_tbl", SqlIfExists::Fail).expect("write");

        let chunks = read_sql_table_columns_chunks(&conn, "proj_chunk_tbl", &["names", "ints"], 2)
            .expect("projection chunk iterator")
            .collect::<Result<Vec<_>, _>>()
            .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].column_names(), vec!["names", "ints"]);
        assert_eq!(
            chunks[0].column("names").unwrap().values(),
            &[
                Scalar::Utf8("alice".to_owned()),
                Scalar::Utf8("bob".to_owned())
            ]
        );
        assert_eq!(
            chunks[1].column("ints").unwrap().values(),
            &[Scalar::Int64(30)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_chunks_rejects_zero_chunksize() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "proj_zero_chunk_tbl", SqlIfExists::Fail).expect("write");

        let err = read_sql_table_columns_chunks(&conn, "proj_zero_chunk_tbl", &["names"], 0)
            .expect_err("zero projection chunksize should be rejected");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("chunksize")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_chunks_rejects_invalid_projection_inputs() {
        let conn = make_sql_test_conn();

        let empty = read_sql_table_columns_chunks(&conn, "proj_chunk_tbl", &[], 1)
            .expect_err("empty projection should be rejected");
        assert!(matches!(empty, IoError::Sql(msg) if msg.contains("columns must be non-empty")));

        let invalid = read_sql_table_columns_chunks(&conn, "proj_chunk_tbl", &["bad column"], 1)
            .expect_err("invalid projection name should be rejected");
        assert!(matches!(invalid, IoError::Sql(msg) if msg.contains("invalid column name")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_with_index_col_promotes_projected_column() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "proj_index_tbl", SqlIfExists::Fail).expect("write");

        let result = read_sql_table_columns_with_index_col(
            &conn,
            "proj_index_tbl",
            &["names", "ints"],
            Some("ints"),
        )
        .expect("projection with index_col");

        assert_eq!(result.index().name(), Some("ints"));
        assert_eq!(
            result.index().labels(),
            &[
                IndexLabel::Int64(10),
                IndexLabel::Int64(20),
                IndexLabel::Int64(30)
            ]
        );
        assert_eq!(result.column_names(), vec!["names"]);
        assert_eq!(
            result.column("names").unwrap().values(),
            &[
                Scalar::Utf8("alice".to_owned()),
                Scalar::Utf8("bob".to_owned()),
                Scalar::Utf8("carol".to_owned())
            ]
        );
        assert!(result.column("ints").is_none());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_with_index_col_none_keeps_projection_and_range_index() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "proj_no_index_tbl", SqlIfExists::Fail).expect("write");

        let result = read_sql_table_columns_with_index_col(
            &conn,
            "proj_no_index_tbl",
            &["floats", "names"],
            None,
        )
        .expect("projection without index_col");

        assert_eq!(
            result.index().labels(),
            &[
                IndexLabel::Int64(0),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2)
            ]
        );
        assert_eq!(result.column_names(), vec!["floats", "names"]);
        assert_eq!(
            result.column("floats").unwrap().values()[1],
            Scalar::Float64(2.5)
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_chunks_with_index_col_promotes_each_chunk_index() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "proj_index_chunk_tbl", SqlIfExists::Fail).expect("write");

        let chunks = read_sql_table_columns_chunks_with_index_col(
            &conn,
            "proj_index_chunk_tbl",
            &["ints", "names"],
            Some("ints"),
            2,
        )
        .expect("indexed projection chunk iterator")
        .collect::<Result<Vec<_>, _>>()
        .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].index().name(), Some("ints"));
        assert_eq!(
            chunks[0].index().labels(),
            &[IndexLabel::Int64(10), IndexLabel::Int64(20)]
        );
        assert_eq!(chunks[0].column_names(), vec!["names"]);
        assert_eq!(
            chunks[0].column("names").unwrap().values(),
            &[
                Scalar::Utf8("alice".to_owned()),
                Scalar::Utf8("bob".to_owned())
            ]
        );
        assert_eq!(chunks[1].index().labels(), &[IndexLabel::Int64(30)]);
        assert_eq!(
            chunks[1].column("names").unwrap().values(),
            &[Scalar::Utf8("carol".to_owned())]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_columns_chunks_with_index_col_validates_projection_and_index() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "proj_index_error_tbl", SqlIfExists::Fail).expect("write");

        let missing = read_sql_table_columns_chunks_with_index_col(
            &conn,
            "proj_index_error_tbl",
            &["names"],
            Some("ints"),
            1,
        )
        .expect_err("missing index_col should error during iterator construction");
        assert!(matches!(missing, IoError::Sql(msg) if msg.contains("index_col")));

        let empty = read_sql_table_columns_chunks_with_index_col(
            &conn,
            "proj_index_error_tbl",
            &[],
            Some("ints"),
            1,
        )
        .expect_err("empty projection should be rejected");
        assert!(matches!(empty, IoError::Sql(msg) if msg.contains("columns must be non-empty")));

        let invalid = read_sql_table_columns_with_index_col(
            &conn,
            "proj_index_error_tbl",
            &["bad column"],
            Some("ints"),
        )
        .expect_err("invalid projection name should be rejected");
        assert!(matches!(invalid, IoError::Sql(msg) if msg.contains("invalid column name")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_with_index_col_works_on_arbitrary_select() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "queried_tbl", SqlIfExists::Fail).expect("write");
        let result = read_sql_with_index_col(
            &conn,
            "SELECT names AS label, ints, floats FROM queried_tbl ORDER BY ints DESC",
            Some("label"),
        )
        .expect("read query with index");
        assert_eq!(result.index().name(), Some("label"));
        // Order respected by the SELECT (ints DESC) → index labels in
        // reversed name order.
        assert_eq!(
            result.index().labels()[0],
            crate::IndexLabel::Utf8("carol".into())
        );
        assert_eq!(
            result.index().labels()[2],
            crate::IndexLabel::Utf8("alice".into())
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_write_read_roundtrip() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();

        write_sql(&frame, &conn, "test_table", SqlIfExists::Fail).expect("write sql");

        let frame2 = read_sql_table(&conn, "test_table").expect("read sql");
        assert_eq!(frame2.index().len(), 3);

        // Int values survive round-trip.
        let ints = frame2.column("ints").unwrap();
        assert_eq!(ints.values()[0], Scalar::Int64(10));
        assert_eq!(ints.values()[1], Scalar::Int64(20));
        assert_eq!(ints.values()[2], Scalar::Int64(30));

        // Float values survive round-trip.
        let floats = frame2.column("floats").unwrap();
        assert_eq!(floats.values()[0], Scalar::Float64(1.5));
        assert_eq!(floats.values()[1], Scalar::Float64(2.5));
        assert_eq!(floats.values()[2], Scalar::Float64(3.5));

        // String values survive round-trip.
        let names = frame2.column("names").unwrap();
        assert_eq!(names.values()[0], Scalar::Utf8("alice".into()));
        assert_eq!(names.values()[1], Scalar::Utf8("bob".into()));
        assert_eq!(names.values()[2], Scalar::Utf8("carol".into()));
    }

    #[derive(Default)]
    struct DollarMarkerSqlConn {
        insert_sql: std::cell::RefCell<Vec<String>>,
        inserted_rows: std::cell::RefCell<Vec<Vec<Vec<Scalar>>>>,
    }

    impl super::SqlConnection for DollarMarkerSqlConn {
        fn query(
            &self,
            _query: &str,
            _params: &[Scalar],
        ) -> Result<super::SqlQueryResult, IoError> {
            Err(IoError::Sql("mock connection does not read".to_owned()))
        }

        fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
            Ok(())
        }

        fn table_exists(&self, _table_name: &str) -> Result<bool, IoError> {
            Ok(false)
        }

        fn insert_rows(&self, insert_sql: &str, rows: &[Vec<Scalar>]) -> Result<(), IoError> {
            self.insert_sql.borrow_mut().push(insert_sql.to_owned());
            self.inserted_rows.borrow_mut().push(rows.to_vec());
            Ok(())
        }

        fn dtype_sql(&self, dtype: DType) -> &'static str {
            match dtype {
                DType::Int64 | DType::Bool | DType::Timedelta64 => "BIGINT",
                DType::Float64 => "DOUBLE PRECISION",
                DType::Utf8 | DType::Categorical | DType::Null | DType::Sparse => "TEXT",
            }
        }

        fn index_dtype_sql(&self, _index: &Index) -> &'static str {
            "TEXT"
        }

        fn parameter_marker(&self, ordinal: usize) -> String {
            format!("${ordinal}")
        }
    }

    #[test]
    fn sql_query_builders_quote_select_and_projection_identifiers() {
        let conn = DollarMarkerSqlConn::default();
        assert_eq!(
            super::sql_select_all_query(&conn, "portable_tbl").expect("select all query"),
            "SELECT * FROM \"portable_tbl\""
        );
        assert_eq!(
            super::sql_select_columns_query(&conn, "portable_tbl", &["names", "ints"])
                .expect("projection query"),
            "SELECT \"names\", \"ints\" FROM \"portable_tbl\""
        );

        let err = super::sql_select_columns_query(&conn, "portable_tbl", &["bad column"])
            .expect_err("projection identifiers stay validated");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid column name")));
    }

    #[test]
    fn sql_query_builders_create_and_insert_use_backend_contracts() {
        let conn = DollarMarkerSqlConn::default();
        let column_defs = vec![
            super::sql_column_definition(&conn, "row id", "TEXT")
                .expect("index column definition"),
            super::sql_column_definition(&conn, "value\"raw", "BIGINT")
                .expect("value column definition"),
        ];

        assert_eq!(
            super::sql_create_table_query(&conn, "typed_tbl", &column_defs)
                .expect("create table query"),
            "CREATE TABLE IF NOT EXISTS \"typed_tbl\" (\"row id\" TEXT, \"value\"\"raw\" BIGINT)"
        );

        let insert_columns = vec!["row id".to_owned(), "value\"raw".to_owned()];
        assert_eq!(
            super::sql_insert_rows_query(&conn, "typed_tbl", &insert_columns)
                .expect("insert row query"),
            "INSERT INTO \"typed_tbl\" (\"row id\", \"value\"\"raw\") VALUES ($1, $2)"
        );
    }

    /// Verify that quote_identifier overrides on a custom backend ACTUALLY
    /// flow through the helper functions (br-frankenpandas-cx2x / fd90.12).
    /// A MySQL-style backend that returns backticks must produce backticked
    /// identifiers in CREATE / SELECT / INSERT statements without any
    /// further plumbing.
    #[test]
    fn sql_query_builders_use_backend_quote_identifier_override() {
        #[derive(Default)]
        struct BacktickSqlConn;
        impl super::SqlConnection for BacktickSqlConn {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn quote_identifier(&self, ident: &str) -> Result<String, IoError> {
                if ident.contains('\0') {
                    return Err(IoError::Sql(
                        "invalid SQL identifier: NUL byte".to_owned(),
                    ));
                }
                // MySQL-style backtick quoting; embedded backticks doubled.
                Ok(format!("`{}`", ident.replace('`', "``")))
            }
        }

        let conn = BacktickSqlConn;
        // SELECT / projection helpers flow through quote_identifier.
        assert_eq!(
            super::sql_select_all_query(&conn, "users").expect("select all"),
            "SELECT * FROM `users`"
        );
        assert_eq!(
            super::sql_select_columns_query(&conn, "users", &["id", "name"]).expect("projection"),
            "SELECT `id`, `name` FROM `users`"
        );
        // CREATE / INSERT helpers flow through quote_identifier.
        let col_defs =
            vec![super::sql_column_definition(&conn, "id", "INTEGER").expect("col def")];
        assert_eq!(
            super::sql_create_table_query(&conn, "users", &col_defs).expect("create"),
            "CREATE TABLE IF NOT EXISTS `users` (`id` INTEGER)"
        );
        let insert_cols = vec!["id".to_owned(), "name".to_owned()];
        assert_eq!(
            super::sql_insert_rows_query(&conn, "users", &insert_cols).expect("insert"),
            "INSERT INTO `users` (`id`, `name`) VALUES (?, ?)"
        );
    }

    #[test]
    fn sql_write_uses_backend_parameter_markers() {
        let frame = make_test_dataframe();
        let conn = DollarMarkerSqlConn::default();

        write_sql(&frame, &conn, "portable_tbl", SqlIfExists::Fail)
            .expect("write through marker-aware mock backend");

        let insert_sql = conn.insert_sql.borrow();
        assert_eq!(
            insert_sql.as_slice(),
            &["INSERT INTO \"portable_tbl\" (\"ints\", \"floats\", \"names\") VALUES ($1, $2, $3)"
                .to_owned()]
        );
        let inserted_rows = conn.inserted_rows.borrow();
        assert_eq!(inserted_rows[0].len(), frame.index().len());
        assert_eq!(inserted_rows[0][0][0], Scalar::Int64(10));
        assert_eq!(inserted_rows[0][2][2], Scalar::Utf8("carol".into()));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_write_with_options_includes_named_index_column() {
        let mut columns = BTreeMap::new();
        columns.insert(
            "vals".to_string(),
            Column::new(DType::Int64, vec![Scalar::Int64(10), Scalar::Int64(20)]).unwrap(),
        );

        let frame = DataFrame::new_with_column_order(
            Index::new(vec![IndexLabel::Int64(101), IndexLabel::Int64(102)]).set_name("row_id"),
            columns,
            vec!["vals".to_string()],
        )
        .unwrap();
        let conn = make_sql_test_conn();

        write_sql_with_options(
            &frame,
            &conn,
            "indexed_write_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: true,
                index_label: None,
            schema: None,
            dtype: None,
            method: SqlInsertMethod::Single,
            chunksize: None,
            },
        )
        .expect("write with named index");

        let roundtrip = read_sql_table_with_index_col(&conn, "indexed_write_tbl", Some("row_id"))
            .expect("read with promoted index");
        assert_eq!(roundtrip.index().name(), Some("row_id"));
        assert_eq!(roundtrip.index().labels(), frame.index().labels());
        assert!(roundtrip.column("row_id").is_none());
        assert_eq!(
            roundtrip.column("vals").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_write_with_options_unnamed_index_defaults_to_index_column_name() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();

        write_sql_with_options(
            &frame,
            &conn,
            "default_index_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: true,
                index_label: None,
            schema: None,
            dtype: None,
            method: SqlInsertMethod::Single,
            chunksize: None,
            },
        )
        .expect("write with unnamed index");

        let raw = read_sql_table(&conn, "default_index_tbl").expect("read raw table");
        assert!(raw.column("index").is_some());
        assert_eq!(raw.column("index").unwrap().values()[0], Scalar::Int64(0));
        assert_eq!(raw.column("index").unwrap().values()[2], Scalar::Int64(2));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_write_with_options_index_label_overrides_name() {
        let mut columns = BTreeMap::new();
        columns.insert(
            "vals".to_string(),
            Column::new(DType::Int64, vec![Scalar::Int64(7), Scalar::Int64(8)]).unwrap(),
        );

        let frame = DataFrame::new_with_column_order(
            Index::new(vec![IndexLabel::Int64(1), IndexLabel::Int64(2)]).set_name("row_id"),
            columns,
            vec!["vals".to_string()],
        )
        .unwrap();
        let conn = make_sql_test_conn();

        write_sql_with_options(
            &frame,
            &conn,
            "override_index_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: true,
                index_label: Some("custom_id".to_string()),
            schema: None,
            dtype: None,
            method: SqlInsertMethod::Single,
            chunksize: None,
            },
        )
        .expect("write with custom index label");

        let raw = read_sql_table(&conn, "override_index_tbl").expect("read raw table");
        assert!(raw.column("custom_id").is_some());
        assert!(raw.column("row_id").is_none());
        assert_eq!(
            raw.column("custom_id").unwrap().values()[0],
            Scalar::Int64(1)
        );
        assert_eq!(
            raw.column("custom_id").unwrap().values()[1],
            Scalar::Int64(2)
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_write_with_options_index_false_omits_index_column() {
        let mut columns = BTreeMap::new();
        columns.insert(
            "vals".to_string(),
            Column::new(DType::Int64, vec![Scalar::Int64(5), Scalar::Int64(6)]).unwrap(),
        );

        let frame = DataFrame::new_with_column_order(
            Index::new(vec![IndexLabel::Int64(9), IndexLabel::Int64(10)]).set_name("row_id"),
            columns,
            vec!["vals".to_string()],
        )
        .unwrap();
        let conn = make_sql_test_conn();

        write_sql_with_options(
            &frame,
            &conn,
            "no_index_write_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: Some("custom_id".to_string()),
            schema: None,
            dtype: None,
            method: SqlInsertMethod::Single,
            chunksize: None,
            },
        )
        .expect("write without index");

        let raw = read_sql_table(&conn, "no_index_write_tbl").expect("read raw table");
        assert!(raw.column("row_id").is_none());
        assert!(raw.column("custom_id").is_none());
        let names: Vec<&str> = raw
            .column_names()
            .iter()
            .map(|name| name.as_str())
            .collect();
        assert_eq!(names, vec!["vals"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_with_query() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "data", SqlIfExists::Fail).unwrap();

        let filtered = read_sql(&conn, "SELECT ints, names FROM data WHERE ints > 15").unwrap();
        assert_eq!(filtered.index().len(), 2); // rows with ints=20,30
        assert_eq!(
            filtered.column("ints").unwrap().values()[0],
            Scalar::Int64(20)
        );
        assert_eq!(
            filtered.column("names").unwrap().values()[1],
            Scalar::Utf8("carol".into())
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_alias_matches_read_sql_query_path() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "data", SqlIfExists::Fail).unwrap();

        let queried = read_sql_query(
            &conn,
            "SELECT names, ints FROM data WHERE ints >= 20 ORDER BY ints",
        )
        .unwrap();

        assert_eq!(queried.column_names(), vec!["names", "ints"]);
        assert_eq!(queried.index().len(), 2);
        assert_eq!(
            queried.column("names").unwrap().values(),
            &[
                Scalar::Utf8("bob".to_owned()),
                Scalar::Utf8("carol".to_owned())
            ]
        );
        assert_eq!(
            queried.column("ints").unwrap().values(),
            &[Scalar::Int64(20), Scalar::Int64(30)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_with_options_applies_params_and_parse_dates() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE events (ts TEXT, value INTEGER);
             INSERT INTO events (ts, value) VALUES
                ('2024-01-15', 1),
                ('2024-02-01 05:06:07', 2),
                ('2024-03-03', 3);",
        )
        .expect("create events table");

        let frame = read_sql_query_with_options(
            &conn,
            "SELECT ts, value FROM events WHERE value > ? ORDER BY value",
            &SqlReadOptions {
                params: Some(vec![Scalar::Int64(1)]),
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: false,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect("read_sql_query with options");

        assert_eq!(frame.column_names(), vec!["ts", "value"]);
        assert_eq!(
            frame.column("ts").unwrap().values(),
            &[
                Scalar::Utf8("2024-02-01 05:06:07".to_owned()),
                Scalar::Utf8("2024-03-03 00:00:00".to_owned())
            ]
        );
        assert_eq!(
            frame.column("value").unwrap().values(),
            &[Scalar::Int64(2), Scalar::Int64(3)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_with_index_col_promotes_named_column() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "data", SqlIfExists::Fail).unwrap();

        let indexed = read_sql_query_with_index_col(
            &conn,
            "SELECT names, ints FROM data ORDER BY ints",
            Some("names"),
        )
        .unwrap();

        assert_eq!(
            indexed.index().labels(),
            &[
                IndexLabel::Utf8("alice".to_owned()),
                IndexLabel::Utf8("bob".to_owned()),
                IndexLabel::Utf8("carol".to_owned())
            ]
        );
        assert_eq!(indexed.index().name(), Some("names"));
        assert!(indexed.column("names").is_none());
        assert_eq!(
            indexed.column("ints").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_chunks_alias_batches_rows() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE query_chunked (id INTEGER, name TEXT);
             INSERT INTO query_chunked (id, name) VALUES
                (1, 'alpha'),
                (2, 'beta'),
                (3, 'gamma');",
        )
        .expect("create query_chunked table");

        let chunks =
            read_sql_query_chunks(&conn, "SELECT id, name FROM query_chunked ORDER BY id", 2)
                .expect("query chunk iterator")
                .collect::<Result<Vec<_>, _>>()
                .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].column_names(), vec!["id", "name"]);
        assert_eq!(
            chunks[0].column("id").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
        assert_eq!(
            chunks[1].column("name").unwrap().values(),
            &[Scalar::Utf8("gamma".to_owned())]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_chunks_with_options_applies_params_parse_dates_and_coerce_float() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE query_events (ts TEXT, amount TEXT, keep INTEGER);
             INSERT INTO query_events (ts, amount, keep) VALUES
                ('2024-01-15', '12.50', 0),
                ('2024-02-01 05:06:07', '$1,234.50', 1),
                ('2024-03-03', '-3.25', 1);",
        )
        .expect("create query_events table");

        let chunks = read_sql_query_chunks_with_options(
            &conn,
            "SELECT ts, amount FROM query_events WHERE keep = ? ORDER BY ts",
            &SqlReadOptions {
                params: Some(vec![Scalar::Int64(1)]),
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            1,
        )
        .expect("query chunk iterator")
        .collect::<Result<Vec<_>, _>>()
        .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(
            chunks[0].column("ts").unwrap().values(),
            &[Scalar::Utf8("2024-02-01 05:06:07".to_owned())]
        );
        assert_eq!(
            chunks[0].column("amount").unwrap().values(),
            &[Scalar::Float64(1234.5)]
        );
        assert_eq!(
            chunks[1].column("ts").unwrap().values(),
            &[Scalar::Utf8("2024-03-03 00:00:00".to_owned())]
        );
        assert_eq!(
            chunks[1].column("amount").unwrap().values(),
            &[Scalar::Float64(-3.25)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_chunks_with_options_and_index_col_applies_options_before_indexing() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE query_index_events (ts TEXT, amount TEXT, keep INTEGER);
             INSERT INTO query_index_events (ts, amount, keep) VALUES
                ('2024-01-15', '12.50', 0),
                ('2024-02-01 05:06:07', '$1,234.50', 1),
                ('2024-03-03', '-3.25', 1);",
        )
        .expect("create query_index_events table");

        let chunks = read_sql_query_chunks_with_options_and_index_col(
            &conn,
            "SELECT ts, amount FROM query_index_events WHERE keep = ? ORDER BY ts",
            &SqlReadOptions {
                params: Some(vec![Scalar::Int64(1)]),
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            Some("ts"),
            1,
        )
        .expect("indexed query chunk iterator")
        .collect::<Result<Vec<_>, _>>()
        .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].index().name(), Some("ts"));
        assert_eq!(
            chunks[0].index().labels(),
            &[IndexLabel::Utf8("2024-02-01 05:06:07".to_owned())]
        );
        assert!(chunks[0].column("ts").is_none());
        assert_eq!(
            chunks[0].column("amount").unwrap().values(),
            &[Scalar::Float64(1234.5)]
        );
        assert_eq!(
            chunks[1].index().labels(),
            &[IndexLabel::Utf8("2024-03-03 00:00:00".to_owned())]
        );
        assert_eq!(
            chunks[1].column("amount").unwrap().values(),
            &[Scalar::Float64(-3.25)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_chunks_with_options_and_index_col_none_keeps_options_and_range_index() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE query_options_no_index (id INTEGER, amount TEXT);
             INSERT INTO query_options_no_index (id, amount) VALUES
                (1, '$10.50'),
                (2, '11.25');",
        )
        .expect("create query_options_no_index table");

        let chunks = read_sql_chunks_with_options_and_index_col(
            &conn,
            "SELECT id, amount FROM query_options_no_index ORDER BY id",
            &SqlReadOptions {
                params: None,
                parse_dates: None,
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            None,
            1,
        )
        .expect("query chunk iterator")
        .collect::<Result<Vec<_>, _>>()
        .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].index().labels(), &[IndexLabel::Int64(0)]);
        assert_eq!(chunks[1].index().labels(), &[IndexLabel::Int64(0)]);
        assert_eq!(
            chunks[0].column("amount").unwrap().values(),
            &[Scalar::Float64(10.5)]
        );
        assert_eq!(
            chunks[1].column("amount").unwrap().values(),
            &[Scalar::Float64(11.25)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_chunks_with_options_and_index_col_missing_column_errors() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE query_options_missing_index (id INTEGER, amount TEXT);
             INSERT INTO query_options_missing_index (id, amount) VALUES (1, '10.5');",
        )
        .expect("create query_options_missing_index table");

        let err = read_sql_query_chunks_with_options_and_index_col(
            &conn,
            "SELECT id, amount FROM query_options_missing_index",
            &SqlReadOptions {
                params: None,
                parse_dates: None,
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            Some("missing"),
            1,
        )
        .expect_err("missing index_col should error during iterator construction");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("index_col")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_chunks_rejects_zero_chunksize() {
        let conn = make_sql_test_conn();

        let err = read_sql_query_chunks(&conn, "SELECT 1", 0)
            .expect_err("zero query chunksize should be rejected");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("chunksize")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_chunks_with_index_col_promotes_each_chunk_index() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE query_index_chunked (id INTEGER, label TEXT, value INTEGER);
             INSERT INTO query_index_chunked (id, label, value) VALUES
                (1, 'alpha', 10),
                (2, 'beta', 20),
                (3, 'gamma', 30);",
        )
        .expect("create query_index_chunked table");

        let chunks = read_sql_query_chunks_with_index_col(
            &conn,
            "SELECT id, label, value FROM query_index_chunked ORDER BY id",
            Some("label"),
            2,
        )
        .expect("query indexed chunk iterator")
        .collect::<Result<Vec<_>, _>>()
        .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].index().name(), Some("label"));
        assert_eq!(
            chunks[0].index().labels(),
            &[
                IndexLabel::Utf8("alpha".to_owned()),
                IndexLabel::Utf8("beta".to_owned())
            ]
        );
        assert!(chunks[0].column("label").is_none());
        assert_eq!(
            chunks[1].index().labels(),
            &[IndexLabel::Utf8("gamma".to_owned())]
        );
        assert_eq!(
            chunks[1].column("value").unwrap().values(),
            &[Scalar::Int64(30)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_chunks_with_index_col_none_keeps_fresh_chunk_range_indexes() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE query_no_index_chunked (id INTEGER, label TEXT);
             INSERT INTO query_no_index_chunked (id, label) VALUES
                (1, 'alpha'),
                (2, 'beta');",
        )
        .expect("create query_no_index_chunked table");

        let chunks = read_sql_chunks_with_index_col(
            &conn,
            "SELECT id, label FROM query_no_index_chunked ORDER BY id",
            None,
            1,
        )
        .expect("query chunk iterator")
        .collect::<Result<Vec<_>, _>>()
        .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].index().labels(), &[IndexLabel::Int64(0)]);
        assert_eq!(chunks[1].index().labels(), &[IndexLabel::Int64(0)]);
        assert_eq!(chunks[1].column_names(), vec!["id", "label"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_query_chunks_with_index_col_missing_column_errors() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE query_missing_index_chunked (id INTEGER, value INTEGER);
             INSERT INTO query_missing_index_chunked (id, value) VALUES (1, 10);",
        )
        .expect("create query_missing_index_chunked table");

        let err = read_sql_query_chunks_with_index_col(
            &conn,
            "SELECT id, value FROM query_missing_index_chunked",
            Some("missing"),
            1,
        )
        .expect_err("missing index_col should error during iterator construction");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("index_col")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_chunks_batches_rows() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE table_chunked (id INTEGER, name TEXT);
             INSERT INTO table_chunked (id, name) VALUES
                (1, 'alpha'),
                (2, 'beta'),
                (3, 'gamma'),
                (4, 'delta');",
        )
        .expect("create table_chunked table");

        let chunks = read_sql_table_chunks(&conn, "table_chunked", 3)
            .expect("table chunk iterator")
            .collect::<Result<Vec<_>, _>>()
            .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].column_names(), vec!["id", "name"]);
        assert_eq!(
            chunks[0].column("id").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)]
        );
        assert_eq!(
            chunks[1].column("name").unwrap().values(),
            &[Scalar::Utf8("delta".to_owned())]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_chunks_rejects_zero_chunksize() {
        let conn = make_sql_test_conn();
        conn.execute_batch("CREATE TABLE zero_chunked (id INTEGER);")
            .expect("create zero_chunked table");

        let err = read_sql_table_chunks(&conn, "zero_chunked", 0)
            .expect_err("zero table chunksize should be rejected");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("chunksize")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_chunks_rejects_invalid_table_name() {
        let conn = make_sql_test_conn();

        let err = read_sql_table_chunks(&conn, "bad table", 1)
            .expect_err("invalid table name should be rejected");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid table name")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_chunks_with_index_col_promotes_each_chunk_index() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE table_index_chunked (id INTEGER, name TEXT, score INTEGER);
             INSERT INTO table_index_chunked (id, name, score) VALUES
                (10, 'alpha', 100),
                (20, 'beta', 200),
                (30, 'gamma', 300);",
        )
        .expect("create table_index_chunked table");

        let chunks =
            read_sql_table_chunks_with_index_col(&conn, "table_index_chunked", Some("id"), 2)
                .expect("table indexed chunk iterator")
                .collect::<Result<Vec<_>, _>>()
                .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].index().name(), Some("id"));
        assert_eq!(
            chunks[0].index().labels(),
            &[IndexLabel::Int64(10), IndexLabel::Int64(20)]
        );
        assert!(chunks[0].column("id").is_none());
        assert_eq!(
            chunks[1].column("score").unwrap().values(),
            &[Scalar::Int64(300)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_with_options_applies_parse_dates_and_coerce_float() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE table_options (ts TEXT, amount TEXT, label TEXT);
             INSERT INTO table_options (ts, amount, label) VALUES
                ('2024-01-15', '$12.50', 'a'),
                ('2024-02-01 05:06:07', '1,234.50', 'b');",
        )
        .expect("create table_options table");

        let frame = read_sql_table_with_options(
            &conn,
            "table_options",
            &SqlReadOptions {
                params: None,
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect("read table with options");

        assert_eq!(
            frame.column("ts").unwrap().values(),
            &[
                Scalar::Utf8("2024-01-15 00:00:00".to_owned()),
                Scalar::Utf8("2024-02-01 05:06:07".to_owned())
            ]
        );
        assert_eq!(
            frame.column("amount").unwrap().values(),
            &[Scalar::Float64(12.5), Scalar::Float64(1234.5)]
        );
        assert_eq!(
            frame.column("label").unwrap().values(),
            &[Scalar::Utf8("a".to_owned()), Scalar::Utf8("b".to_owned())]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_chunks_with_options_applies_options_before_chunking() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE table_options_chunked (ts TEXT, amount TEXT);
             INSERT INTO table_options_chunked (ts, amount) VALUES
                ('2024-03-01', '$10.00'),
                ('2024-03-02', '$20.50'),
                ('2024-03-03', '-3.25');",
        )
        .expect("create table_options_chunked table");

        let chunks = read_sql_table_chunks_with_options(
            &conn,
            "table_options_chunked",
            &SqlReadOptions {
                params: None,
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            2,
        )
        .expect("table option chunk iterator")
        .collect::<Result<Vec<_>, _>>()
        .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(
            chunks[0].column("ts").unwrap().values(),
            &[
                Scalar::Utf8("2024-03-01 00:00:00".to_owned()),
                Scalar::Utf8("2024-03-02 00:00:00".to_owned())
            ]
        );
        assert_eq!(
            chunks[0].column("amount").unwrap().values(),
            &[Scalar::Float64(10.0), Scalar::Float64(20.5)]
        );
        assert_eq!(
            chunks[1].column("amount").unwrap().values(),
            &[Scalar::Float64(-3.25)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_chunks_with_options_validates_chunksize_and_table_name() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE table_options_errors (ts TEXT);
             INSERT INTO table_options_errors (ts) VALUES ('2024-01-01');",
        )
        .expect("create table_options_errors table");

        let zero = read_sql_table_chunks_with_options(
            &conn,
            "table_options_errors",
            &SqlReadOptions::default(),
            0,
        )
        .expect_err("zero chunksize should be rejected");
        assert!(matches!(zero, IoError::Sql(msg) if msg.contains("chunksize")));

        let invalid = read_sql_table_with_options(
            &conn,
            "bad table",
            &SqlReadOptions {
                parse_dates: Some(vec!["ts".to_owned()]),
                ..SqlReadOptions::default()
            },
        )
        .expect_err("invalid table name should be rejected");
        assert!(matches!(invalid, IoError::Sql(msg) if msg.contains("invalid table name")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_with_options_and_index_col_applies_options_before_indexing() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE table_options_index (ts TEXT, amount TEXT, label TEXT);
             INSERT INTO table_options_index (ts, amount, label) VALUES
                ('2024-04-01', '$10.00', 'a'),
                ('2024-04-02 03:04:05', '20.50', 'b');",
        )
        .expect("create table_options_index table");

        let frame = read_sql_table_with_options_and_index_col(
            &conn,
            "table_options_index",
            &SqlReadOptions {
                params: None,
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            Some("ts"),
        )
        .expect("read table with options and index_col");

        assert_eq!(frame.index().name(), Some("ts"));
        assert_eq!(
            frame.index().labels(),
            &[
                IndexLabel::Utf8("2024-04-01 00:00:00".to_owned()),
                IndexLabel::Utf8("2024-04-02 03:04:05".to_owned())
            ]
        );
        assert!(frame.column("ts").is_none());
        assert_eq!(
            frame.column("amount").unwrap().values(),
            &[Scalar::Float64(10.0), Scalar::Float64(20.5)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_with_options_and_index_col_none_keeps_options_and_range_index() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE table_options_no_index (id INTEGER, amount TEXT);
             INSERT INTO table_options_no_index (id, amount) VALUES
                (1, '$1.25'),
                (2, '$2.50');",
        )
        .expect("create table_options_no_index table");

        let frame = read_sql_table_with_options_and_index_col(
            &conn,
            "table_options_no_index",
            &SqlReadOptions {
                params: None,
                parse_dates: None,
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            None,
        )
        .expect("read table with options and no index_col");

        assert_eq!(
            frame.index().labels(),
            &[IndexLabel::Int64(0), IndexLabel::Int64(1)]
        );
        assert_eq!(frame.column_names(), vec!["id", "amount"]);
        assert_eq!(
            frame.column("amount").unwrap().values(),
            &[Scalar::Float64(1.25), Scalar::Float64(2.5)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_chunks_with_options_and_index_col_promotes_each_chunk_index() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE table_options_index_chunked (ts TEXT, amount TEXT);
             INSERT INTO table_options_index_chunked (ts, amount) VALUES
                ('2024-05-01', '$10.00'),
                ('2024-05-02', '$20.00'),
                ('2024-05-03', '$30.50');",
        )
        .expect("create table_options_index_chunked table");

        let chunks = read_sql_table_chunks_with_options_and_index_col(
            &conn,
            "table_options_index_chunked",
            &SqlReadOptions {
                params: None,
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            Some("ts"),
            2,
        )
        .expect("table indexed option chunk iterator")
        .collect::<Result<Vec<_>, _>>()
        .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].index().name(), Some("ts"));
        assert_eq!(
            chunks[0].index().labels(),
            &[
                IndexLabel::Utf8("2024-05-01 00:00:00".to_owned()),
                IndexLabel::Utf8("2024-05-02 00:00:00".to_owned())
            ]
        );
        assert!(chunks[0].column("ts").is_none());
        assert_eq!(
            chunks[0].column("amount").unwrap().values(),
            &[Scalar::Float64(10.0), Scalar::Float64(20.0)]
        );
        assert_eq!(
            chunks[1].index().labels(),
            &[IndexLabel::Utf8("2024-05-03 00:00:00".to_owned())]
        );
        assert_eq!(
            chunks[1].column("amount").unwrap().values(),
            &[Scalar::Float64(30.5)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_table_chunks_with_options_and_index_col_missing_column_errors() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE table_options_missing_index (id INTEGER, amount TEXT);
             INSERT INTO table_options_missing_index (id, amount) VALUES (1, '$10.00');",
        )
        .expect("create table_options_missing_index table");

        let err = read_sql_table_chunks_with_options_and_index_col(
            &conn,
            "table_options_missing_index",
            &SqlReadOptions {
                params: None,
                parse_dates: None,
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            Some("missing"),
            1,
        )
        .expect_err("missing index_col should error during iterator construction");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("index_col")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_with_parse_dates_coerces_named_columns() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE events (ts TEXT, value INTEGER);
             INSERT INTO events (ts, value) VALUES
                ('2024-01-15', 1),
                ('2024-02-01 05:06:07', 2);",
        )
        .expect("create events table");

        let frame = read_sql_with_options(
            &conn,
            "SELECT ts, value FROM events ORDER BY value",
            &SqlReadOptions {
                params: None,
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: false,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect("read sql with parse_dates");

        assert_eq!(
            frame.column("ts").unwrap().values()[0],
            Scalar::Utf8("2024-01-15 00:00:00".into())
        );
        assert_eq!(
            frame.column("ts").unwrap().values()[1],
            Scalar::Utf8("2024-02-01 05:06:07".into())
        );
        assert_eq!(frame.column("value").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(frame.column("value").unwrap().values()[1], Scalar::Int64(2));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_with_parse_dates_missing_column_errors() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE metrics (value INTEGER);
             INSERT INTO metrics (value) VALUES (1);",
        )
        .expect("create metrics table");

        let err = read_sql_with_options(
            &conn,
            "SELECT value FROM metrics",
            &SqlReadOptions {
                params: None,
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: false,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect_err("missing parse_dates column should error");

        assert!(
            matches!(err, IoError::MissingParseDateColumns(missing) if missing == vec!["ts".to_owned()])
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_with_params_binds_positional_placeholders() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "data", SqlIfExists::Fail).unwrap();

        let filtered = read_sql_with_options(
            &conn,
            "SELECT ints, names FROM data WHERE ints > ? AND names != ? ORDER BY ints",
            &SqlReadOptions {
                params: Some(vec![Scalar::Int64(15), Scalar::Utf8("bob".to_owned())]),
                parse_dates: None,
                coerce_float: false,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect("read sql with params");

        assert_eq!(filtered.index().len(), 1);
        assert_eq!(
            filtered.column("ints").unwrap().values(),
            &[Scalar::Int64(30)]
        );
        assert_eq!(
            filtered.column("names").unwrap().values(),
            &[Scalar::Utf8("carol".into())]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_with_params_wrong_arity_errors() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "data", SqlIfExists::Fail).unwrap();

        let err = read_sql_with_options(
            &conn,
            "SELECT ints FROM data WHERE ints > ? AND names != ?",
            &SqlReadOptions {
                params: Some(vec![Scalar::Int64(15)]),
                parse_dates: None,
                coerce_float: false,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect_err("wrong arity should error");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("parameter")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_coerce_float_promotes_decimal_like_text_columns() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE payments (id INTEGER, amount TEXT, fee TEXT);
             INSERT INTO payments (id, amount, fee) VALUES
                (1, '12.50', '$1,234.50'),
                (2, '-3.25', NULL);",
        )
        .expect("create payments table");

        // Per fd90.41: pandas default for coerce_float is True, and our
        // SqlReadOptions::default() now matches. So the bare read_sql
        // path coerces decimal-like text columns to Float64 by default.
        let default_frame =
            read_sql(&conn, "SELECT amount FROM payments ORDER BY id").expect("default read");
        assert_eq!(default_frame.column("amount").unwrap().dtype(), DType::Float64);
        assert_eq!(
            default_frame.column("amount").unwrap().values(),
            &[Scalar::Float64(12.5), Scalar::Float64(-3.25)],
        );

        // Explicitly opting out of coerce_float keeps the raw Utf8.
        let no_coerce = read_sql_with_options(
            &conn,
            "SELECT amount FROM payments ORDER BY id",
            &SqlReadOptions {
                coerce_float: false,
                ..SqlReadOptions::default()
            },
        )
        .expect("read without coerce_float");
        assert_eq!(
            no_coerce.column("amount").unwrap().values(),
            &[
                Scalar::Utf8("12.50".to_owned()),
                Scalar::Utf8("-3.25".to_owned()),
            ],
        );

        let coerced = read_sql_with_options(
            &conn,
            "SELECT amount, fee FROM payments ORDER BY id",
            &SqlReadOptions {
                coerce_float: true,
                ..SqlReadOptions::default()
            },
        )
        .expect("read with coerce_float");

        let amount = coerced.column("amount").expect("amount");
        assert_eq!(amount.dtype(), DType::Float64);
        assert_eq!(
            amount.values(),
            &[Scalar::Float64(12.5), Scalar::Float64(-3.25)],
        );

        let fee = coerced.column("fee").expect("fee");
        assert_eq!(fee.dtype(), DType::Float64);
        assert_eq!(fee.values()[0], Scalar::Float64(1234.5));
        assert!(matches!(fee.values()[1], Scalar::Null(NullKind::NaN)));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_coerce_float_leaves_non_numeric_text_columns_unchanged() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE mixed (id INTEGER, maybe_amount TEXT, label TEXT);
             INSERT INTO mixed (id, maybe_amount, label) VALUES
                (1, '12.50', 'alpha'),
                (2, 'not numeric', '20.0');",
        )
        .expect("create mixed table");

        let frame = read_sql_with_options(
            &conn,
            "SELECT maybe_amount, label FROM mixed ORDER BY id",
            &SqlReadOptions {
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
                ..SqlReadOptions::default()
            },
        )
        .expect("read with coerce_float");

        assert_eq!(
            frame.column("maybe_amount").unwrap().values(),
            &[
                Scalar::Utf8("12.50".to_owned()),
                Scalar::Utf8("not numeric".to_owned()),
            ],
        );
        assert_eq!(
            frame.column("label").unwrap().values(),
            &[
                Scalar::Utf8("alpha".to_owned()),
                Scalar::Utf8("20.0".to_owned()),
            ],
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_chunks_batches_rows_and_resets_index_per_chunk() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE chunked (id INTEGER, name TEXT);
             INSERT INTO chunked (id, name) VALUES
                (1, 'alpha'),
                (2, 'beta'),
                (3, 'gamma'),
                (4, 'delta'),
                (5, 'epsilon');",
        )
        .expect("create chunked table");

        let chunks = read_sql_chunks(&conn, "SELECT id, name FROM chunked ORDER BY id", 2)
            .expect("chunk iterator")
            .collect::<Result<Vec<_>, _>>()
            .expect("all chunks");

        assert_eq!(chunks.len(), 3);
        assert_eq!(
            chunks[0].index().labels(),
            &[IndexLabel::Int64(0), IndexLabel::Int64(1)]
        );
        assert_eq!(
            chunks[1].index().labels(),
            &[IndexLabel::Int64(0), IndexLabel::Int64(1)]
        );
        assert_eq!(chunks[2].index().labels(), &[IndexLabel::Int64(0)]);
        assert_eq!(
            chunks[0].column("id").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
        assert_eq!(
            chunks[1].column("id").unwrap().values(),
            &[Scalar::Int64(3), Scalar::Int64(4)]
        );
        assert_eq!(
            chunks[2].column("name").unwrap().values(),
            &[Scalar::Utf8("epsilon".to_owned())]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_chunks_with_options_applies_params_parse_dates_and_coerce_float() {
        let conn = make_sql_test_conn();
        conn.execute_batch(
            "CREATE TABLE events (ts TEXT, amount TEXT, keep INTEGER);
             INSERT INTO events (ts, amount, keep) VALUES
                ('2024-01-15', '12.50', 0),
                ('2024-02-01 05:06:07', '$1,234.50', 1),
                ('2024-03-03', '-3.25', 1);",
        )
        .expect("create events table");

        let chunks = read_sql_chunks_with_options(
            &conn,
            "SELECT ts, amount FROM events WHERE keep = ? ORDER BY ts",
            &SqlReadOptions {
                params: Some(vec![Scalar::Int64(1)]),
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: true,
                dtype: None,
                schema: None,
                columns: None,
                index_col: None,
            },
            1,
        )
        .expect("chunk iterator")
        .collect::<Result<Vec<_>, _>>()
        .expect("all chunks");

        assert_eq!(chunks.len(), 2);
        assert_eq!(
            chunks[0].column("ts").unwrap().values(),
            &[Scalar::Utf8("2024-02-01 05:06:07".to_owned())]
        );
        assert_eq!(
            chunks[0].column("amount").unwrap().values(),
            &[Scalar::Float64(1234.5)]
        );
        assert_eq!(
            chunks[1].column("ts").unwrap().values(),
            &[Scalar::Utf8("2024-03-03 00:00:00".to_owned())]
        );
        assert_eq!(
            chunks[1].column("amount").unwrap().values(),
            &[Scalar::Float64(-3.25)]
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_read_chunks_rejects_zero_chunksize() {
        let conn = make_sql_test_conn();

        let err =
            read_sql_chunks(&conn, "SELECT 1", 0).expect_err("zero chunksize should be rejected");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("chunksize")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_duplicate_column_names_error() {
        let conn = make_sql_test_conn();
        let err = read_sql(&conn, "SELECT 1 as dup, 2 as dup");
        assert!(matches!(err, Err(IoError::DuplicateColumnName(name)) if name == "dup"));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_if_exists_fail() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "tbl", SqlIfExists::Fail).unwrap();

        let err = write_sql(&frame, &conn, "tbl", SqlIfExists::Fail);
        assert!(err.is_err());
        assert!(matches!(&err.unwrap_err(), IoError::Sql(msg) if msg.contains("already exists")),);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_if_exists_replace() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "tbl", SqlIfExists::Fail).unwrap();

        // Replace should succeed and overwrite.
        write_sql(&frame, &conn, "tbl", SqlIfExists::Replace).unwrap();
        let frame2 = read_sql_table(&conn, "tbl").unwrap();
        assert_eq!(frame2.index().len(), 3);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_if_exists_append() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "tbl", SqlIfExists::Fail).unwrap();

        // Append should add rows.
        write_sql(&frame, &conn, "tbl", SqlIfExists::Append).unwrap();
        let frame2 = read_sql_table(&conn, "tbl").unwrap();
        assert_eq!(frame2.index().len(), 6); // 3 + 3
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_with_nulls() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "vals".to_string(),
            Column::new(
                DType::Float64,
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame =
            DataFrame::new_with_column_order(Index::new(labels), columns, vec!["vals".to_string()])
                .unwrap();

        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "nulltest", SqlIfExists::Fail).unwrap();
        let frame2 = read_sql_table(&conn, "nulltest").unwrap();

        assert_eq!(
            frame2.column("vals").unwrap().values()[0],
            Scalar::Float64(1.0)
        );
        assert!(frame2.column("vals").unwrap().values()[1].is_missing());
        assert_eq!(
            frame2.column("vals").unwrap().values()[2],
            Scalar::Float64(3.0)
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_bool_roundtrip() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "flags".to_string(),
            Column::new(
                DType::Bool,
                vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
            )
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame = DataFrame::new_with_column_order(
            Index::new(labels),
            columns,
            vec!["flags".to_string()],
        )
        .unwrap();

        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "booltest", SqlIfExists::Fail).unwrap();
        let frame2 = read_sql_table(&conn, "booltest").unwrap();

        // Bools stored as INTEGER(0/1), read back as Int64.
        assert_eq!(
            frame2.column("flags").unwrap().values()[0],
            Scalar::Int64(1)
        );
        assert_eq!(
            frame2.column("flags").unwrap().values()[1],
            Scalar::Int64(0)
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_invalid_table_name_rejected() {
        let conn = make_sql_test_conn();
        let err = read_sql_table(&conn, "Robert'; DROP TABLE students; --");
        assert!(err.is_err());
        assert!(
            matches!(&err.unwrap_err(), IoError::Sql(msg) if msg.contains("invalid table name")),
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_empty_table_name_rejected() {
        let conn = make_sql_test_conn();
        let err = read_sql_table(&conn, "");
        assert!(err.is_err());
        assert!(
            matches!(&err.unwrap_err(), IoError::Sql(msg) if msg.contains("invalid table name")),
        );

        let frame = make_test_dataframe();
        let err = write_sql(&frame, &conn, "", SqlIfExists::Fail);
        assert!(err.is_err());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_empty_result() {
        let conn = make_sql_test_conn();
        conn.execute_batch("CREATE TABLE empty (x INTEGER, y TEXT)")
            .unwrap();
        let frame = read_sql_table(&conn, "empty").unwrap();
        assert_eq!(frame.index().len(), 0);
        assert_eq!(frame.column_names().len(), 2);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_extension_trait() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();

        // Use the extension trait method.
        use super::DataFrameIoExt;
        frame.to_sql(&conn, "ext_test", SqlIfExists::Fail).unwrap();

        let frame2 = read_sql_table(&conn, "ext_test").unwrap();
        assert_eq!(frame2.index().len(), 3);
    }

    // ── Arrow IPC / Feather tests ────────────────────────────────────

    #[test]
    fn feather_bytes_roundtrip() {
        let frame = make_test_dataframe();
        let bytes = super::write_feather_bytes(&frame).expect("write feather");
        assert!(!bytes.is_empty());

        let frame2 = super::read_feather_bytes(&bytes).expect("read feather");
        assert_eq!(frame2.index().len(), 3);

        // Check all column values survive round-trip exactly.
        let ints = frame2.column("ints").unwrap();
        assert_eq!(ints.values()[0], Scalar::Int64(10));
        assert_eq!(ints.values()[1], Scalar::Int64(20));
        assert_eq!(ints.values()[2], Scalar::Int64(30));

        let floats = frame2.column("floats").unwrap();
        assert_eq!(floats.values()[0], Scalar::Float64(1.5));
        assert_eq!(floats.values()[2], Scalar::Float64(3.5));

        let names = frame2.column("names").unwrap();
        assert_eq!(names.values()[0], Scalar::Utf8("alice".into()));
        assert_eq!(names.values()[2], Scalar::Utf8("carol".into()));
    }

    #[test]
    fn feather_row_multiindex_roundtrip_restores_logical_row_axis() {
        let frame = make_row_multiindex_test_dataframe();
        let bytes = super::write_feather_bytes(&frame).expect("write feather");
        let roundtrip = super::read_feather_bytes(&bytes).expect("read feather");

        assert!(roundtrip.equals(&frame));
        assert!(roundtrip.column("__index_level_0__").is_none());
        assert_eq!(
            roundtrip
                .row_multiindex()
                .expect("row multiindex should be restored")
                .get_level_values(1)
                .unwrap()
                .labels(),
            frame
                .row_multiindex()
                .expect("source row multiindex")
                .get_level_values(1)
                .unwrap()
                .labels()
        );
    }

    #[test]
    fn feather_file_roundtrip() {
        let frame = make_test_dataframe();
        let dir = std::env::temp_dir();
        let path = dir.join("fp_io_test_feather_roundtrip.feather");

        super::write_feather(&frame, &path).expect("write feather file");
        let frame2 = super::read_feather(&path).expect("read feather file");
        assert_eq!(frame2.index().len(), 3);
        assert_eq!(
            frame2.column("ints").unwrap().values()[0],
            Scalar::Int64(10)
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ipc_stream_bytes_roundtrip() {
        let frame = make_test_dataframe();
        let bytes = super::write_ipc_stream_bytes(&frame).expect("write ipc stream");
        assert!(!bytes.is_empty());

        let frame2 = super::read_ipc_stream_bytes(&bytes).expect("read ipc stream");
        assert_eq!(frame2.index().len(), 3);
        assert_eq!(
            frame2.column("ints").unwrap().values()[1],
            Scalar::Int64(20)
        );
        assert_eq!(
            frame2.column("names").unwrap().values()[1],
            Scalar::Utf8("bob".into())
        );
    }

    #[test]
    fn ipc_stream_row_multiindex_roundtrip_restores_logical_row_axis() {
        let frame = make_row_multiindex_test_dataframe();
        let bytes = super::write_ipc_stream_bytes(&frame).expect("write ipc stream");
        let roundtrip = super::read_ipc_stream_bytes(&bytes).expect("read ipc stream");

        assert!(roundtrip.equals(&frame));
        assert!(roundtrip.row_multiindex().is_some());
    }

    #[test]
    fn feather_with_nulls() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "vals".to_string(),
            Column::new(
                DType::Float64,
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame =
            DataFrame::new_with_column_order(Index::new(labels), columns, vec!["vals".to_string()])
                .unwrap();

        let bytes = super::write_feather_bytes(&frame).expect("write");
        let frame2 = super::read_feather_bytes(&bytes).expect("read");

        assert_eq!(
            frame2.column("vals").unwrap().values()[0],
            Scalar::Float64(1.0)
        );
        assert!(frame2.column("vals").unwrap().values()[1].is_missing());
        assert_eq!(
            frame2.column("vals").unwrap().values()[2],
            Scalar::Float64(3.0)
        );
    }

    #[test]
    fn feather_nullable_int_roundtrip_preserves_int_dtype() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "vals".to_string(),
            Column::new(
                DType::Int64,
                vec![
                    Scalar::Int64(10),
                    Scalar::Null(NullKind::Null),
                    Scalar::Int64(30),
                ],
            )
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame =
            DataFrame::new_with_column_order(Index::new(labels), columns, vec!["vals".to_string()])
                .unwrap();

        let bytes = super::write_feather_bytes(&frame).expect("write");
        let frame2 = super::read_feather_bytes(&bytes).expect("read");
        let vals = frame2.column("vals").unwrap();

        assert_eq!(vals.dtype(), DType::Int64);
        assert_eq!(vals.values()[0], Scalar::Int64(10));
        assert_eq!(vals.values()[1], Scalar::Null(NullKind::Null));
        assert_eq!(vals.values()[2], Scalar::Int64(30));
    }

    #[test]
    fn series_arrow_array_nullable_int_roundtrip() {
        let series = Series::from_values(
            "vals",
            vec![
                IndexLabel::Utf8("r0".into()),
                IndexLabel::Utf8("r1".into()),
                IndexLabel::Utf8("r2".into()),
            ],
            vec![
                Scalar::Int64(10),
                Scalar::Null(NullKind::Null),
                Scalar::Int64(30),
            ],
        )
        .unwrap();

        let (dt, arr) = super::series_to_arrow_array(&series).expect("arrow encode");
        assert_eq!(dt, ArrowDataType::Int64);

        let typed = arr
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("int64 arrow array");
        assert_eq!(typed.value(0), 10);
        assert!(typed.is_null(1));
        assert_eq!(typed.value(2), 30);

        let roundtrip = super::series_from_arrow_array(
            series.name(),
            series.index().labels().to_vec(),
            arr.as_ref(),
            &dt,
        )
        .expect("arrow decode");

        assert_eq!(roundtrip.name(), "vals");
        assert_eq!(roundtrip.index().labels(), series.index().labels());
        assert_eq!(roundtrip.column().dtype(), DType::Int64);
        assert_eq!(roundtrip.values(), series.values());
    }

    #[test]
    fn feather_bool_column() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "flags".to_string(),
            Column::new(
                DType::Bool,
                vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
            )
            .unwrap(),
        );

        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame = DataFrame::new_with_column_order(
            Index::new(labels),
            columns,
            vec!["flags".to_string()],
        )
        .unwrap();

        let bytes = super::write_feather_bytes(&frame).expect("write");
        let frame2 = super::read_feather_bytes(&bytes).expect("read");

        assert_eq!(
            frame2.column("flags").unwrap().values()[0],
            Scalar::Bool(true)
        );
        assert_eq!(
            frame2.column("flags").unwrap().values()[1],
            Scalar::Bool(false)
        );
    }

    #[test]
    fn feather_preserves_column_order() {
        let frame = make_test_dataframe();
        let bytes = super::write_feather_bytes(&frame).expect("write");
        let frame2 = super::read_feather_bytes(&bytes).expect("read");

        assert_eq!(
            frame2
                .column_names()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            frame
                .column_names()
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn feather_extension_trait() {
        use super::DataFrameIoExt;

        let frame = make_test_dataframe();
        let bytes = frame.to_feather_bytes().unwrap();
        let frame2 = super::read_feather_bytes(&bytes).unwrap();
        assert_eq!(frame2.index().len(), 3);
    }

    // ── Adversarial parser tests (frankenpandas-yby) ─────────────────

    // ── CsvReadOptions extended params tests (frankenpandas-qoz) ────

    #[test]
    fn csv_nrows_limits_rows() {
        let input = "x\n1\n2\n3\n4\n5\n";
        let opts = CsvReadOptions {
            nrows: Some(3),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(frame.index().len(), 3);
        assert_eq!(frame.column("x").unwrap().values()[2], Scalar::Int64(3));
    }

    #[test]
    fn csv_skiprows_skips_data_rows() {
        let input = "x\n1\n2\n3\n4\n5\n";
        let opts = CsvReadOptions {
            skiprows: 2,
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(frame.index().len(), 3); // skipped header + first data row
        assert_eq!(frame.column("2").unwrap().values()[0], Scalar::Int64(3));
    }

    #[test]
    fn csv_skiprows_and_nrows_combined() {
        let input = "x\n1\n2\n3\n4\n5\n";
        let opts = CsvReadOptions {
            skiprows: 1,
            nrows: Some(2),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(frame.index().len(), 2); // skipped header; read 2 data rows
        assert_eq!(frame.column("1").unwrap().values()[0], Scalar::Int64(2));
        assert_eq!(frame.column("1").unwrap().values()[1], Scalar::Int64(3));
    }

    #[test]
    fn csv_usecols_selects_columns() {
        let input = "a,b,c\n1,2,3\n4,5,6\n";
        let opts = CsvReadOptions {
            usecols: Some(vec!["a".into(), "c".into()]),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(frame.column_names().len(), 2);
        assert!(frame.column("a").is_some());
        assert!(frame.column("b").is_none());
        assert!(frame.column("c").is_some());
        assert_eq!(frame.column("a").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(frame.column("c").unwrap().values()[1], Scalar::Int64(6));
    }

    #[test]
    fn csv_usecols_nonexistent_column_errors() {
        let input = "a,b\n1,2\n";
        let opts = CsvReadOptions {
            usecols: Some(vec!["a".into(), "nonexistent".into()]),
            ..Default::default()
        };
        let err = read_csv_with_options(input, &opts).expect_err("missing usecols should error");
        assert!(matches!(err, IoError::MissingUsecols(_)));
    }

    #[test]
    fn csv_dtype_coercion() {
        let input = "id,score\n1,95\n2,87\n";
        let mut dtype_map = std::collections::HashMap::new();
        dtype_map.insert("score".to_owned(), fp_types::DType::Float64);
        let opts = CsvReadOptions {
            dtype: Some(dtype_map),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        // score column should be Float64, not Int64
        assert_eq!(
            frame.column("score").unwrap().values()[0],
            Scalar::Float64(95.0)
        );
        assert_eq!(
            frame.column("score").unwrap().values()[1],
            Scalar::Float64(87.0)
        );
        // id column should remain Int64 (not in dtype map)
        assert_eq!(frame.column("id").unwrap().values()[0], Scalar::Int64(1));
    }

    #[test]
    fn csv_dtype_coercion_invalid_value_errors() {
        let input = "id,score\n1,abc\n";
        let mut dtype_map = std::collections::HashMap::new();
        dtype_map.insert("score".to_owned(), fp_types::DType::Int64);
        let opts = CsvReadOptions {
            dtype: Some(dtype_map),
            ..Default::default()
        };
        let err = read_csv_with_options(input, &opts).expect_err("invalid cast must error");
        assert!(matches!(
            err,
            IoError::Column(fp_columnar::ColumnError::Type(
                fp_types::TypeError::InvalidCast { .. }
            ))
        ));
    }

    #[test]
    fn csv_skiprows_beyond_data_errors() {
        let input = "x\n1\n2\n";
        let opts = CsvReadOptions {
            skiprows: 100,
            ..Default::default()
        };
        let err = read_csv_with_options(input, &opts).expect_err("skiprows removes header");
        assert!(matches!(err, IoError::MissingHeaders));
    }

    #[test]
    fn csv_nrows_zero_returns_empty() {
        let input = "x\n1\n2\n3\n";
        let opts = CsvReadOptions {
            nrows: Some(0),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(frame.index().len(), 0);
    }

    #[test]
    fn csv_decimal_comma_parses_quoted_float_fields() {
        let input = "price\n\"1,50\"\n\"3,75\"\n";
        let opts = CsvReadOptions {
            decimal: b',',
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(
            frame.column("price").unwrap().values(),
            &[Scalar::Float64(1.5), Scalar::Float64(3.75)]
        );
    }

    #[test]
    fn csv_default_decimal_keeps_comma_decimal_strings_as_utf8() {
        let input = "price\n\"1,50\"\n";
        let frame = read_csv_with_options(input, &CsvReadOptions::default()).expect("parse");
        assert_eq!(
            frame.column("price").unwrap().values(),
            &[Scalar::Utf8("1,50".to_owned())]
        );
    }

    #[test]
    fn csv_true_false_values_do_not_override_numeric_inference() {
        let input = "flag\n1\n0\n";
        let opts = CsvReadOptions {
            true_values: vec!["1".to_owned()],
            false_values: vec!["0".to_owned()],
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(
            frame.column("flag").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(0)]
        );
    }

    #[test]
    fn csv_true_false_values_convert_non_numeric_tokens() {
        let input = "flag\nyes\nno\n";
        let opts = CsvReadOptions {
            true_values: vec!["yes".to_owned()],
            false_values: vec!["no".to_owned()],
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(
            frame.column("flag").unwrap().values(),
            &[Scalar::Bool(true), Scalar::Bool(false)]
        );
    }

    #[test]
    fn csv_default_parsing_keeps_numeric_boolean_tokens_as_ints() {
        let input = "flag\n1\n0\n";
        let frame = read_csv_with_options(input, &CsvReadOptions::default()).expect("parse");
        assert_eq!(
            frame.column("flag").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(0)]
        );
    }

    #[test]
    fn csv_missing_numeric_column_promotes_ints_to_float() {
        let input = "a,b,c\n,NA,NaN\n1,,x\n";
        let frame = read_csv_with_options(input, &CsvReadOptions::default()).expect("parse");
        assert_eq!(
            frame.column("a").unwrap().values(),
            &[Scalar::Null(NullKind::NaN), Scalar::Float64(1.0)]
        );
        assert!(frame.column("b").unwrap().values()[0].is_missing());
        assert!(frame.column("b").unwrap().values()[1].is_missing());
        assert_eq!(
            frame.column("c").unwrap().values(),
            &[Scalar::Null(NullKind::Null), Scalar::Utf8("x".to_owned())]
        );
    }

    #[test]
    fn csv_parse_dates_mixed_naive_and_aware_strings_preserves_object_like_values() {
        let input = "ts,value\n2024-01-15 10:30:00,1\n2024-01-15T10:30:00Z,2\n";
        let opts = CsvReadOptions {
            parse_dates: Some(vec!["ts".to_owned()]),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(
            frame.column("ts").unwrap().values(),
            &[
                Scalar::Utf8("2024-01-15 10:30:00".to_owned()),
                Scalar::Utf8("2024-01-15T10:30:00Z".to_owned()),
            ]
        );
        assert_eq!(
            frame.column("value").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
    }

    #[test]
    fn csv_parse_dates_combined_columns_replaces_source_columns() {
        let input = "date,time,value\n2024-01-15,10:30:00,1\n2024-01-16,11:45:30,2\n";
        let opts = CsvReadOptions {
            parse_date_combinations: Some(vec![vec!["date".to_owned(), "time".to_owned()]]),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        assert_eq!(frame.column_names(), vec!["date_time", "value"]);
        assert_eq!(
            frame.column("date_time").unwrap().values(),
            &[
                Scalar::Utf8("2024-01-15 10:30:00".to_owned()),
                Scalar::Utf8("2024-01-16 11:45:30".to_owned()),
            ]
        );
        assert!(frame.column("date").is_none());
        assert!(frame.column("time").is_none());
        assert_eq!(
            frame.column("value").unwrap().values(),
            &[Scalar::Int64(1), Scalar::Int64(2)]
        );
    }

    #[test]
    fn csv_parse_date_combinations_named_uses_caller_supplied_name() {
        let input = "date,time,value\n2024-01-15,10:30:00,1\n2024-01-16,11:45:30,2\n";
        let opts = CsvReadOptions {
            parse_date_combinations_named: Some(vec![(
                "timestamp".to_owned(),
                vec!["date".to_owned(), "time".to_owned()],
            )]),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        // Dict-style rename: combined column named "timestamp" rather than
        // the default underscore-joined "date_time".
        assert_eq!(frame.column_names(), vec!["timestamp", "value"]);
        assert!(frame.column("date").is_none());
        assert!(frame.column("time").is_none());
        assert_eq!(
            frame.column("timestamp").unwrap().values(),
            &[
                Scalar::Utf8("2024-01-15 10:30:00".to_owned()),
                Scalar::Utf8("2024-01-16 11:45:30".to_owned()),
            ]
        );
    }

    #[test]
    fn csv_parse_date_combinations_named_multiple_groups() {
        let input = "d1,t1,d2,t2,value\n2024-01-01,09:00:00,2024-01-01,17:00:00,10\n2024-02-01,09:00:00,2024-02-01,17:00:00,20\n";
        let opts = CsvReadOptions {
            parse_date_combinations_named: Some(vec![
                ("start".to_owned(), vec!["d1".to_owned(), "t1".to_owned()]),
                ("end".to_owned(), vec!["d2".to_owned(), "t2".to_owned()]),
            ]),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        let names = frame.column_names();
        assert!(names.iter().any(|n| n.as_str() == "start"));
        assert!(names.iter().any(|n| n.as_str() == "end"));
        assert!(!names.iter().any(|n| n.as_str() == "d1"));
        assert!(!names.iter().any(|n| n.as_str() == "t2"));
        assert_eq!(
            frame.column("value").unwrap().values(),
            &[Scalar::Int64(10), Scalar::Int64(20)]
        );
        assert_eq!(
            frame.column("start").unwrap().values(),
            &[
                Scalar::Utf8("2024-01-01 09:00:00".to_owned()),
                Scalar::Utf8("2024-02-01 09:00:00".to_owned()),
            ]
        );
    }

    #[test]
    fn csv_parse_date_combinations_named_rejects_duplicate_output_names() {
        let input = "a,b,c,d\n2024,01,2024,02\n";
        let opts = CsvReadOptions {
            parse_date_combinations_named: Some(vec![
                ("ts".to_owned(), vec!["a".to_owned(), "b".to_owned()]),
                ("ts".to_owned(), vec!["c".to_owned(), "d".to_owned()]),
            ]),
            ..Default::default()
        };
        let err = read_csv_with_options(input, &opts).unwrap_err();
        assert!(matches!(err, IoError::DuplicateColumnName(name) if name == "ts"));
    }

    #[test]
    fn csv_parse_date_combinations_named_rejects_missing_source_column() {
        let input = "date,time,value\n2024-01-01,09:00:00,1\n";
        let opts = CsvReadOptions {
            parse_date_combinations_named: Some(vec![(
                "ts".to_owned(),
                vec!["date".to_owned(), "missing".to_owned()],
            )]),
            ..Default::default()
        };
        let err = read_csv_with_options(input, &opts).unwrap_err();
        assert!(matches!(err, IoError::MissingParseDateColumns(_)));
    }

    #[test]
    fn csv_parse_date_combinations_named_empty_sources_skipped() {
        let input = "a,b\n1,2\n";
        let opts = CsvReadOptions {
            parse_date_combinations_named: Some(vec![("unused".to_owned(), Vec::new())]),
            ..Default::default()
        };
        let frame = read_csv_with_options(input, &opts).expect("parse");
        // Empty source list is a no-op; original columns remain.
        assert_eq!(frame.column_names(), vec!["a", "b"]);
    }

    // ── JSONL tests (frankenpandas-sue) ──────────────────────────────

    #[test]
    fn jsonl_write_read_roundtrip() {
        let frame = make_test_dataframe();
        let jsonl = super::write_jsonl_string(&frame).expect("JSONL write failed");

        // Each line should be a valid JSON object.
        let line_count = jsonl.lines().count();
        assert_eq!(line_count, 3, "3 rows = 3 lines");

        let back = super::read_jsonl_str(&jsonl).expect("JSONL read failed");
        assert_eq!(back.index().len(), 3);
        assert_eq!(back.column("ints").unwrap().values()[0], Scalar::Int64(10));
        assert_eq!(
            back.column("names").unwrap().values()[2],
            Scalar::Utf8("carol".into())
        );
    }

    #[test]
    fn jsonl_preserves_column_order() {
        let input = r#"
{"b":1,"a":2}
{"c":3}
"#;
        let frame = super::read_jsonl_str(input).expect("JSONL read failed");
        let order: Vec<&str> = frame
            .column_names()
            .iter()
            .map(|name| name.as_str())
            .collect();
        assert_eq!(order, vec!["b", "a", "c"]);
    }

    #[test]
    fn jsonl_each_line_is_valid_json() {
        let frame = make_test_dataframe();
        let jsonl = super::write_jsonl_string(&frame).unwrap();

        for (i, line) in jsonl.lines().enumerate() {
            let parsed: serde_json::Value =
                serde_json::from_str(line).expect("jsonl line must be valid JSON");
            assert!(parsed.is_object(), "line {i} must be a JSON object");
        }
    }

    #[test]
    fn jsonl_with_nulls() {
        use fp_types::DType;

        let mut columns = BTreeMap::new();
        columns.insert(
            "v".to_string(),
            Column::new(
                DType::Float64,
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(3.0),
                ],
            )
            .unwrap(),
        );
        let labels = vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ];
        let frame =
            DataFrame::new_with_column_order(Index::new(labels), columns, vec!["v".to_string()])
                .unwrap();

        let jsonl = super::write_jsonl_string(&frame).unwrap();
        let back = super::read_jsonl_str(&jsonl).unwrap();
        assert!(back.column("v").unwrap().values()[1].is_missing());
    }

    #[test]
    fn jsonl_empty_input() {
        let back = super::read_jsonl_str("").expect("empty JSONL must parse");
        assert_eq!(back.index().len(), 0);
    }

    #[test]
    fn jsonl_blank_lines_skipped() {
        let input = "{\"a\":1}\n\n{\"a\":2}\n\n";
        let back = super::read_jsonl_str(input).expect("JSONL with blanks must parse");
        assert_eq!(back.index().len(), 2);
    }

    #[test]
    fn jsonl_non_object_line_errors() {
        let input = "{\"a\":1}\n[1,2,3]\n";
        let err = super::read_jsonl_str(input);
        assert!(err.is_err());
    }

    #[test]
    fn jsonl_different_keys_across_rows() {
        // Rows with different keys should produce union of all columns.
        let input = "{\"a\":1,\"b\":2}\n{\"a\":3,\"c\":4}\n";
        let frame = super::read_jsonl_str(input).expect("JSONL with different keys must parse");
        assert_eq!(frame.index().len(), 2);
        // Should have columns a, b, c (union of all keys).
        assert!(frame.column("a").is_some(), "column a must exist");
        assert!(frame.column("b").is_some(), "column b must exist");
        assert!(frame.column("c").is_some(), "column c must exist");
        // Row 0: a=1, b=2, c=null.
        assert_eq!(frame.column("a").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(frame.column("b").unwrap().values()[0], Scalar::Float64(2.0));
        assert!(frame.column("c").unwrap().values()[0].is_missing());
        // Row 1: a=3, b=null, c=4.
        assert_eq!(frame.column("a").unwrap().values()[1], Scalar::Int64(3));
        assert!(frame.column("b").unwrap().values()[1].is_missing());
        assert_eq!(frame.column("c").unwrap().values()[1], Scalar::Float64(4.0));
    }

    #[test]
    fn adversarial_csv_very_long_field() {
        // A single field with >100K characters should parse without panic.
        let long_val = "x".repeat(200_000);
        let input = format!("col\n{long_val}\n");
        let frame = read_csv_str(&input).expect("long field must parse");
        assert_eq!(frame.index().len(), 1);
        match &frame.column("col").unwrap().values()[0] {
            Scalar::Utf8(s) => assert_eq!(s.len(), 200_000),
            other => assert!(
                matches!(other, Scalar::Utf8(_)),
                "expected Utf8 for long field"
            ),
        }
    }

    #[test]
    fn adversarial_csv_many_columns() {
        // CSV with 1000 columns should parse correctly.
        let ncols = 1000;
        let headers: Vec<String> = (0..ncols).map(|i| format!("c{i}")).collect();
        let mut csv = headers.join(",");
        csv.push('\n');
        let vals: Vec<String> = (0..ncols).map(|i| i.to_string()).collect();
        csv.push_str(&vals.join(","));
        csv.push('\n');

        let frame = read_csv_str(&csv).expect("1000-column CSV must parse");
        assert_eq!(frame.columns().len(), ncols);
        assert_eq!(frame.index().len(), 1);
    }

    #[test]
    fn adversarial_csv_empty_rows() {
        // CSV with empty rows between data should handle gracefully.
        // The csv crate skips truly empty records, but rows with the right
        // number of empty fields produce null values.
        let input = "a,b\n1,2\n,\n3,4\n";
        let frame = read_csv_str(input).expect("parse");
        assert_eq!(frame.index().len(), 3);
        // Row 1 (index 1) has empty fields → null
        assert!(frame.column("a").unwrap().values()[1].is_missing());
    }

    #[test]
    fn adversarial_csv_field_with_newlines_in_quotes() {
        // Embedded newlines in quoted fields must not break row boundaries.
        let input = "msg\n\"line1\nline2\nline3\"\n\"single\"\n";
        let frame = read_csv_str(input).expect("quoted newlines must parse");
        assert_eq!(frame.index().len(), 2);
    }

    #[test]
    fn adversarial_csv_header_only_no_data() {
        let input = "x,y,z\n";
        let frame = read_csv_str(input).expect("header-only must parse");
        assert_eq!(frame.index().len(), 0);
        assert_eq!(frame.columns().len(), 3);
    }

    #[test]
    fn adversarial_json_deeply_nested_values() {
        // JSON with nested objects as values should store them as strings.
        let input = r#"[{"a":1,"b":{"nested":"value"}}]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("nested JSON must parse");
        assert_eq!(frame.index().len(), 1);
        // The nested object becomes a Utf8 representation.
        let b_val = &frame.column("b").unwrap().values()[0];
        assert!(matches!(b_val, Scalar::Utf8(_)));
    }

    #[test]
    fn adversarial_json_i64_max_value() {
        // JSON with values at i64 boundary.
        let input = format!(r#"[{{"v":{}}}]"#, i64::MAX);
        let frame = read_json_str(&input, JsonOrient::Records).expect("i64::MAX must parse");
        assert_eq!(
            frame.column("v").unwrap().values()[0],
            Scalar::Int64(i64::MAX)
        );
    }

    #[test]
    fn adversarial_json_i64_min_value() {
        let input = format!(r#"[{{"v":{}}}]"#, i64::MIN);
        let frame = read_json_str(&input, JsonOrient::Records).expect("i64::MIN must parse");
        assert_eq!(
            frame.column("v").unwrap().values()[0],
            Scalar::Int64(i64::MIN)
        );
    }

    #[test]
    fn adversarial_json_float_special_values() {
        // JSON doesn't natively support Infinity/NaN, but we should handle
        // null gracefully.
        let input = r#"[{"v":null},{"v":1.7976931348623157e+308}]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("special floats must parse");
        assert!(frame.column("v").unwrap().values()[0].is_missing());
        // f64::MAX is approximately 1.7976931348623157e+308
        if let Scalar::Float64(v) = frame.column("v").unwrap().values()[1] {
            assert!(v.is_finite());
        }
    }

    #[test]
    fn adversarial_json_empty_records_array() {
        let input = r#"[]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("empty array must parse");
        assert_eq!(frame.index().len(), 0);
    }

    #[test]
    fn adversarial_json_empty_columns_object() {
        let input = r#"{}"#;
        let frame = read_json_str(input, JsonOrient::Columns).expect("empty object must parse");
        assert_eq!(frame.index().len(), 0);
        assert_eq!(frame.columns().len(), 0);
    }

    #[test]
    fn adversarial_csv_unicode_values() {
        // CSV with multi-byte UTF-8 characters in values.
        let input = "name,emoji\n日本語,🎉\nрусский,🚀\n";
        let frame = read_csv_str(input).expect("unicode CSV must parse");
        assert_eq!(frame.index().len(), 2);
        assert_eq!(
            frame.column("name").unwrap().values()[0],
            Scalar::Utf8("日本語".into())
        );
        assert_eq!(
            frame.column("emoji").unwrap().values()[1],
            Scalar::Utf8("🚀".into())
        );
    }

    #[test]
    fn adversarial_csv_single_column_no_trailing_newline() {
        let input = "val\n42";
        let frame = read_csv_str(input).expect("no trailing newline must parse");
        assert_eq!(frame.index().len(), 1);
        assert_eq!(frame.column("val").unwrap().values()[0], Scalar::Int64(42));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn adversarial_sql_large_batch_insert() {
        // Insert 10K rows in a single write_sql call.
        let n = 10_000;
        let vals: Vec<Scalar> = (0..n).map(|i| Scalar::Int64(i as i64)).collect();
        let df = fp_frame::DataFrame::from_dict(&["x"], vec![("x", vals)]).unwrap();

        let conn = make_sql_test_conn();
        write_sql(&df, &conn, "big_table", SqlIfExists::Fail).unwrap();
        let back = read_sql_table(&conn, "big_table").unwrap();
        assert_eq!(back.index().len(), n);
        assert_eq!(
            back.column("x").unwrap().values()[n - 1],
            Scalar::Int64((n - 1) as i64)
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn adversarial_sql_column_name_with_spaces_accepted() {
        // Column names with spaces are valid in SQL (quoted identifiers).
        // Table names are restricted, but column names go through quoting.
        let df = fp_frame::DataFrame::from_dict(
            &["has space"],
            vec![("has space", vec![Scalar::Int64(1)])],
        )
        .unwrap();

        let conn = make_sql_test_conn();
        // This should work since column names are quoted.
        let result = write_sql(&df, &conn, "test_spaces", SqlIfExists::Fail);
        assert!(
            result.is_ok(),
            "columns with spaces should work: {:?}",
            result.err()
        );

        let back = read_sql_table(&conn, "test_spaces").unwrap();
        assert!(back.column("has space").is_some());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn adversarial_sql_column_name_with_quotes_accepted() {
        let col_name = "has\"quote";
        let df =
            fp_frame::DataFrame::from_dict(&[col_name], vec![(col_name, vec![Scalar::Int64(7)])])
                .unwrap();

        let conn = make_sql_test_conn();
        let result = write_sql(&df, &conn, "test_quotes", SqlIfExists::Fail);
        assert!(
            result.is_ok(),
            "columns with quotes should work: {:?}",
            result.err()
        );

        let back = read_sql_table(&conn, "test_quotes").unwrap();
        assert_eq!(back.column(col_name).unwrap().values()[0], Scalar::Int64(7));
    }

    // ── SqlConnection capability + dialect probes (br-frankenpandas-6dtf) ────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_dialect_name_is_sqlite() {
        let conn = make_sql_test_conn();
        assert_eq!(super::SqlConnection::dialect_name(&conn), "sqlite");
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_supports_returning_is_true() {
        // Bundled SQLite is 3.35+, so RETURNING is supported.
        let conn = make_sql_test_conn();
        assert!(super::SqlConnection::supports_returning(&conn));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_max_param_count_is_32766() {
        let conn = make_sql_test_conn();
        assert_eq!(super::SqlConnection::max_param_count(&conn), Some(32766));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_with_transaction_commits_on_ok() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE txn_test (x INTEGER)").unwrap();
        let result: Result<i64, IoError> = super::SqlConnection::with_transaction(&conn, |c| {
            super::SqlConnection::execute_batch(c, "INSERT INTO txn_test VALUES (42)")?;
            Ok(42)
        });
        assert_eq!(result.unwrap(), 42);
        // Verify the row committed.
        let row_count =
            super::SqlConnection::query(&conn, "SELECT COUNT(*) FROM txn_test", &[]).unwrap();
        assert_eq!(row_count.rows.len(), 1);
        assert_eq!(row_count.rows[0][0], Scalar::Int64(1));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_with_transaction_rolls_back_on_err() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE txn_rollback (x INTEGER)")
            .unwrap();
        let result: Result<(), IoError> = super::SqlConnection::with_transaction(&conn, |c| {
            super::SqlConnection::execute_batch(c, "INSERT INTO txn_rollback VALUES (99)")?;
            Err(IoError::Sql("simulated failure".to_string()))
        });
        assert!(result.is_err());
        // Row should NOT have committed.
        let row_count =
            super::SqlConnection::query(&conn, "SELECT COUNT(*) FROM txn_rollback", &[]).unwrap();
        assert_eq!(row_count.rows[0][0], Scalar::Int64(0));
    }

    #[test]
    fn default_capability_probes_are_conservative() {
        // A test-double SqlConnection that doesn't override defaults
        // should report the conservative-default values from the trait.
        struct StubSql;
        impl super::SqlConnection for StubSql {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }

        let stub = StubSql;
        assert_eq!(super::SqlConnection::dialect_name(&stub), "unknown");
        assert!(!super::SqlConnection::supports_returning(&stub));
        assert_eq!(super::SqlConnection::max_param_count(&stub), None);
        // Default with_transaction passes through (no BEGIN/COMMIT).
        let result: Result<i64, IoError> = super::SqlConnection::with_transaction(&stub, |_| Ok(7));
        assert_eq!(result.unwrap(), 7);
        // Default quote_identifier produces ANSI double-quotes.
        assert_eq!(
            super::SqlConnection::quote_identifier(&stub, "col").unwrap(),
            r#""col""#
        );
    }

    // ── quote_identifier tests (br-frankenpandas-2y7w / fd90.10) ────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_quote_identifier_uses_ansi_double_quotes() {
        let conn = make_sql_test_conn();
        assert_eq!(
            super::SqlConnection::quote_identifier(&conn, "users").unwrap(),
            r#""users""#
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_quote_identifier_doubles_embedded_quotes() {
        let conn = make_sql_test_conn();
        // Identifier containing a `"` must be escaped by doubling the quote.
        assert_eq!(
            super::SqlConnection::quote_identifier(&conn, r#"value"raw"#).unwrap(),
            r#""value""raw""#
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_quote_identifier_rejects_null_bytes() {
        let conn = make_sql_test_conn();
        let err = super::SqlConnection::quote_identifier(&conn, "evil\0name").expect_err("nul");
        assert!(matches!(err, IoError::Sql(_)));
    }

    #[test]
    fn default_quote_identifier_doubles_embedded_quotes() {
        // Verify the default impl on a non-overriding stub matches the
        // SQLite behavior (ANSI is the shared default for SQLite + Postgres).
        struct StubSql;
        impl super::SqlConnection for StubSql {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let stub = StubSql;
        assert_eq!(
            super::SqlConnection::quote_identifier(&stub, r#"value"raw"#).unwrap(),
            r#""value""raw""#
        );
        assert!(super::SqlConnection::quote_identifier(&stub, "evil\0").is_err());
    }

    // ── SqlReadOptions::dtype tests (br-frankenpandas-l9pt / fd90.11) ───

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_dtype_override_int_to_float() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE amounts (amount INTEGER); INSERT INTO amounts VALUES (1), (2), (3);",
        )
        .unwrap();
        let mut dtype_map = BTreeMap::new();
        dtype_map.insert("amount".to_owned(), DType::Float64);
        let frame = read_sql_with_options(
            &conn,
            "SELECT amount FROM amounts ORDER BY amount",
            &SqlReadOptions {
                params: None,
                parse_dates: None,
                coerce_float: false,
                dtype: Some(dtype_map),
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect("read with dtype");
        let col = frame.column("amount").expect("amount");
        assert_eq!(col.dtype(), DType::Float64);
        assert_eq!(col.values()[0], Scalar::Float64(1.0));
        assert_eq!(col.values()[2], Scalar::Float64(3.0));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_dtype_override_unsupported_cast_returns_typed_error() {
        // fp_types::cast_scalar_owned doesn't currently support Int64→Utf8
        // (that gap is tracked separately). This test asserts that when the
        // cast is unsupported, dtype override surfaces a typed IoError::Sql
        // with diagnostic context, NOT a panic and not a silent skip.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE labels (id INTEGER); INSERT INTO labels VALUES (10), (20);",
        )
        .unwrap();
        let mut dtype_map = BTreeMap::new();
        dtype_map.insert("id".to_owned(), DType::Utf8);
        let err = read_sql_with_options(
            &conn,
            "SELECT id FROM labels ORDER BY id",
            &SqlReadOptions {
                params: None,
                parse_dates: None,
                coerce_float: false,
                dtype: Some(dtype_map),
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect_err("expected dtype override error");
        match err {
            IoError::Sql(message) => {
                assert!(
                    message.contains("dtype override on column 'id'"),
                    "unexpected error message: {message}"
                );
                assert!(
                    message.contains("Utf8"),
                    "unexpected error message: {message}"
                );
            }
            other => unreachable!("expected IoError::Sql, got {other:?}"),
        }
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_dtype_override_missing_column_is_ignored() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE t (x INTEGER); INSERT INTO t VALUES (1);",
        )
        .unwrap();
        let mut dtype_map = BTreeMap::new();
        dtype_map.insert("nonexistent".to_owned(), DType::Float64);
        let frame = read_sql_with_options(
            &conn,
            "SELECT x FROM t",
            &SqlReadOptions {
                params: None,
                parse_dates: None,
                coerce_float: false,
                dtype: Some(dtype_map),
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect("read with dtype-on-missing-col");
        let col = frame.column("x").expect("x");
        assert_eq!(col.dtype(), DType::Int64);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_dtype_override_preserves_nulls() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE nulls_tbl (v INTEGER); INSERT INTO nulls_tbl VALUES (1), (NULL), (3);",
        )
        .unwrap();
        let mut dtype_map = BTreeMap::new();
        dtype_map.insert("v".to_owned(), DType::Float64);
        let frame = read_sql_with_options(
            &conn,
            "SELECT v FROM nulls_tbl ORDER BY rowid",
            &SqlReadOptions {
                params: None,
                parse_dates: None,
                coerce_float: false,
                dtype: Some(dtype_map),
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect("read with dtype + nulls");
        let col = frame.column("v").expect("v");
        assert_eq!(col.dtype(), DType::Float64);
        assert!(col.values()[1].is_missing());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_dtype_skipped_when_column_in_parse_dates() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE evt (ts TEXT); INSERT INTO evt VALUES ('2024-01-01 00:00:00');",
        )
        .unwrap();
        let mut dtype_map = BTreeMap::new();
        dtype_map.insert("ts".to_owned(), DType::Float64);
        let frame = read_sql_with_options(
            &conn,
            "SELECT ts FROM evt",
            &SqlReadOptions {
                params: None,
                parse_dates: Some(vec!["ts".to_owned()]),
                coerce_float: false,
                dtype: Some(dtype_map),
                schema: None,
                columns: None,
                index_col: None,
            },
        )
        .expect("read with parse_dates priority");
        let col = frame.column("ts").expect("ts");
        assert_eq!(col.dtype(), DType::Utf8);
    }

    // ── Schema probes (br-frankenpandas-6dk9 / fd90.13) ─────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn rusqlite_does_not_support_schemas_by_default() {
        let conn = make_sql_test_conn();
        assert!(!super::SqlConnection::supports_schemas(&conn));
        assert_eq!(super::SqlConnection::default_schema(&conn), None);
    }

    #[test]
    fn default_schema_probes_are_conservative() {
        // A test-double with no schema overrides reports the conservative
        // single-namespace defaults (matches SQLite + most embedded
        // backends). Production multi-schema backends (PG, MySQL) override.
        struct StubSql;
        impl super::SqlConnection for StubSql {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let stub = StubSql;
        assert!(!super::SqlConnection::supports_schemas(&stub));
        assert_eq!(super::SqlConnection::default_schema(&stub), None);
    }

    #[test]
    fn schema_probe_overrides_take_effect() {
        // A multi-schema-style test backend (e.g. simulating PostgreSQL)
        // overrides supports_schemas + default_schema. The overrides MUST
        // win over the trait defaults.
        struct PgLikeSqlConn;
        impl super::SqlConnection for PgLikeSqlConn {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn default_schema(&self) -> Option<String> {
                Some("public".to_owned())
            }
        }
        let conn = PgLikeSqlConn;
        assert!(super::SqlConnection::supports_schemas(&conn));
        assert_eq!(
            super::SqlConnection::default_schema(&conn).as_deref(),
            Some("public")
        );
    }

    // ── SqlReadOptions::schema tests (br-frankenpandas-u6zn / fd90.14) ──

    #[test]
    fn sql_select_all_query_no_schema_uses_bare_table() {
        struct StubSql;
        impl super::SqlConnection for StubSql {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = StubSql;
        let q1 = super::sql_select_all_query_in_schema(&conn, "users", None).expect("q1");
        assert_eq!(q1, "SELECT * FROM \"users\"");
        let q2 =
            super::sql_select_all_query_in_schema(&conn, "users", Some("ignored")).expect("q2");
        assert_eq!(q2, "SELECT * FROM \"users\"");
    }

    #[test]
    fn sql_select_all_query_with_schema_qualifies_on_multi_schema_backend() {
        struct PgLikeSchemaSql;
        impl super::SqlConnection for PgLikeSchemaSql {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
        }
        let conn = PgLikeSchemaSql;
        let q = super::sql_select_all_query_in_schema(&conn, "users", Some("analytics"))
            .expect("q");
        assert_eq!(q, "SELECT * FROM \"analytics\".\"users\"");
        let bare = super::sql_select_all_query_in_schema(&conn, "users", None).expect("bare");
        assert_eq!(bare, "SELECT * FROM \"users\"");
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_table_with_options_schema_silently_ignored_on_sqlite() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE bare_tbl (x INTEGER); INSERT INTO bare_tbl VALUES (1), (2);",
        )
        .unwrap();
        let frame = read_sql_table_with_options(
            &conn,
            "bare_tbl",
            &SqlReadOptions {
                params: None,
                parse_dates: None,
                coerce_float: false,
                dtype: None,
                schema: Some("ignored_on_sqlite".to_owned()),
                columns: None,
                index_col: None,
            },
        )
        .expect("read with schema=Some on SQLite");
        let col = frame.column("x").expect("x");
        assert_eq!(col.values()[0], Scalar::Int64(1));
        assert_eq!(col.values()[1], Scalar::Int64(2));
    }

    #[test]
    fn sql_select_all_query_in_schema_validates_schema_name() {
        struct PgLikeValidate;
        impl super::SqlConnection for PgLikeValidate {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
        }
        let conn = PgLikeValidate;
        let err = super::sql_select_all_query_in_schema(&conn, "users", Some("evil; DROP"))
            .expect_err("malformed schema must reject");
        // Per fd90.56: error message now correctly identifies the
        // bad identifier as a schema, not a table.
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid schema name")));
    }

    // ── SqlWriteOptions::schema tests (br-frankenpandas-udn6 / fd90.15) ─

    #[test]
    fn sql_create_table_query_in_schema_qualifies_on_multi_schema_backend() {
        struct PgLikeWrite;
        impl super::SqlConnection for PgLikeWrite {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
        }
        let conn = PgLikeWrite;
        let cols = vec!["id INTEGER".to_owned(), "name TEXT".to_owned()];
        let q = super::sql_create_table_query_in_schema(
            &conn,
            "users",
            Some("analytics"),
            &cols,
        )
        .expect("create");
        assert_eq!(
            q,
            "CREATE TABLE IF NOT EXISTS \"analytics\".\"users\" (id INTEGER, name TEXT)"
        );
        let bare = super::sql_create_table_query_in_schema(&conn, "users", None, &cols)
            .expect("bare");
        assert_eq!(
            bare,
            "CREATE TABLE IF NOT EXISTS \"users\" (id INTEGER, name TEXT)"
        );
    }

    #[test]
    fn sql_insert_rows_query_in_schema_qualifies_on_multi_schema_backend() {
        struct PgLikeInsert;
        impl super::SqlConnection for PgLikeInsert {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
        }
        let conn = PgLikeInsert;
        let cols = vec!["id".to_owned(), "name".to_owned()];
        let q = super::sql_insert_rows_query_in_schema(&conn, "users", Some("analytics"), &cols)
            .expect("insert");
        assert_eq!(
            q,
            "INSERT INTO \"analytics\".\"users\" (\"id\", \"name\") VALUES (?, ?)"
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_with_options_schema_silently_ignored_on_sqlite() {
        let conn = make_sql_test_conn();
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        // SQLite reports supports_schemas=false; passing schema=Some(s) must
        // not break the write — the bare table reference is used.
        write_sql_with_options(
            &frame,
            &conn,
            "bare_write_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: Some("ignored_on_sqlite".to_owned()),
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect("write with schema=Some on SQLite");
        let back = read_sql_table(&conn, "bare_write_tbl").expect("read");
        let col = back.column("x").expect("x");
        assert_eq!(col.values()[0], Scalar::Int64(1));
        assert_eq!(col.values()[1], Scalar::Int64(2));
    }

    #[test]
    fn sql_create_table_query_in_schema_validates_schema_name() {
        struct PgLikeValidate;
        impl super::SqlConnection for PgLikeValidate {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
        }
        let conn = PgLikeValidate;
        let cols = vec!["x INTEGER".to_owned()];
        let err = super::sql_create_table_query_in_schema(
            &conn,
            "users",
            Some("evil; DROP"),
            &cols,
        )
        .expect_err("malformed schema must reject");
        // Per fd90.56: schema validation error now says invalid
        // schema name (not invalid table name).
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid schema name")));
    }

    // ── DROP TABLE schema-qualification (br-frankenpandas-hxob / fd90.16) ─

    #[test]
    fn sql_drop_table_query_bare_on_non_multi_schema() {
        struct StubSql;
        impl super::SqlConnection for StubSql {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = StubSql;
        let q = super::sql_drop_table_query_in_schema(&conn, "users", None).expect("drop none");
        assert_eq!(q, "DROP TABLE IF EXISTS \"users\"");
        // schema=Some on non-multi-schema is silently ignored.
        let q2 =
            super::sql_drop_table_query_in_schema(&conn, "users", Some("ignored")).expect("drop");
        assert_eq!(q2, "DROP TABLE IF EXISTS \"users\"");
    }

    #[test]
    fn sql_drop_table_query_qualifies_on_multi_schema_backend() {
        struct PgLikeDrop;
        impl super::SqlConnection for PgLikeDrop {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
        }
        let conn = PgLikeDrop;
        let q = super::sql_drop_table_query_in_schema(&conn, "users", Some("analytics"))
            .expect("drop qualified");
        assert_eq!(q, "DROP TABLE IF EXISTS \"analytics\".\"users\"");
        let bare =
            super::sql_drop_table_query_in_schema(&conn, "users", None).expect("drop bare");
        assert_eq!(bare, "DROP TABLE IF EXISTS \"users\"");
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_replace_with_schema_silently_ignored_on_sqlite() {
        // Replace path drops + recreates. SQLite reports supports_schemas
        // == false; the schema is silently ignored on the DROP and on the
        // CREATE/INSERT — the round trip lands data in the bare table.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE replace_tbl (x INTEGER); INSERT INTO replace_tbl VALUES (99);",
        )
        .unwrap();
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1), Scalar::Int64(2)])],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "replace_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Replace,
                index: false,
                index_label: None,
                schema: Some("ignored_on_sqlite".to_owned()),
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect("replace + schema=Some on SQLite");
        let back = read_sql_table(&conn, "replace_tbl").expect("read");
        let col = back.column("x").expect("x");
        // Pre-existing 99 was dropped; new rows present.
        assert_eq!(col.values().len(), 2);
        assert_eq!(col.values()[0], Scalar::Int64(1));
        assert_eq!(col.values()[1], Scalar::Int64(2));
    }

    // ── table_exists_in_schema (br-frankenpandas-70d1 / fd90.17) ────────

    #[test]
    fn default_table_exists_in_schema_delegates_to_table_exists() {
        // A stub that returns table_exists=true for "users" must report the
        // same value via the schema-aware default impl regardless of schema.
        struct StubExistsTrue;
        impl super::SqlConnection for StubExistsTrue {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, name: &str) -> Result<bool, IoError> {
                Ok(name == "users")
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = StubExistsTrue;
        // Schema is ignored by the default impl.
        assert!(super::SqlConnection::table_exists_in_schema(&conn, "users", None).unwrap());
        assert!(
            super::SqlConnection::table_exists_in_schema(&conn, "users", Some("ignored"))
                .unwrap()
        );
        assert!(!super::SqlConnection::table_exists_in_schema(&conn, "missing", None).unwrap());
    }

    #[test]
    fn multi_schema_override_scopes_table_exists() {
        // PgLikeSchemaCheck overrides table_exists_in_schema to scope by
        // schema: only ('analytics', 'users') exists.
        struct PgLikeSchemaCheck;
        impl super::SqlConnection for PgLikeSchemaCheck {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<super::SqlQueryResult, IoError> {
                Ok(super::SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                // Bare table_exists isn't queried by the override path.
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn table_exists_in_schema(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<bool, IoError> {
                Ok(table == "users" && schema == Some("analytics"))
            }
        }
        let conn = PgLikeSchemaCheck;
        assert!(
            super::SqlConnection::table_exists_in_schema(&conn, "users", Some("analytics"))
                .unwrap()
        );
        // Different schema → false.
        assert!(
            !super::SqlConnection::table_exists_in_schema(&conn, "users", Some("audit"))
                .unwrap()
        );
        // No schema → false (override scopes by Some).
        assert!(
            !super::SqlConnection::table_exists_in_schema(&conn, "users", None).unwrap()
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_fail_with_schema_some_still_rejects_existing_on_sqlite() {
        // SQLite ignores schema everywhere; the Fail branch still reports
        // 'table already exists' when the bare table is present.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE preexists_tbl (x INTEGER);",
        )
        .unwrap();
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1)])],
        )
        .unwrap();
        let err = write_sql_with_options(
            &frame,
            &conn,
            "preexists_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: Some("ignored_on_sqlite".to_owned()),
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect_err("Fail branch must still reject pre-existing");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("already exists")));
    }

    // ── SqlWriteOptions::dtype overrides (br-frankenpandas-ev2s / fd90.18) ─

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_dtype_override_emits_custom_sql_type() {
        // SQLite is permissive on declared types — the column-type string
        // ends up in sqlite_master.sql verbatim, which we can grep to verify
        // the override took effect during CREATE TABLE.
        let conn = make_sql_test_conn();
        let frame = fp_frame::DataFrame::from_dict(
            &["amount"],
            vec![("amount", vec![Scalar::Int64(100), Scalar::Int64(250)])],
        )
        .unwrap();
        let mut overrides = BTreeMap::new();
        overrides.insert("amount".to_owned(), "NUMERIC(10,2)".to_owned());
        write_sql_with_options(
            &frame,
            &conn,
            "money_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: Some(overrides),
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect("write with dtype override");
        let sm = super::SqlConnection::query(
            &conn,
            "SELECT sql FROM sqlite_master WHERE name = 'money_tbl'",
            &[],
        )
        .unwrap();
        let create_sql = match &sm.rows[0][0] {
            Scalar::Utf8(s) => s.clone(),
            other => unreachable!("unexpected sqlite_master payload: {other:?}"),
        };
        assert!(
            create_sql.contains("NUMERIC(10,2)"),
            "expected override to land in CREATE TABLE; got: {create_sql}"
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_dtype_override_multiple_columns() {
        let conn = make_sql_test_conn();
        let frame = fp_frame::DataFrame::from_dict(
            &["a", "b"],
            vec![
                ("a", vec![Scalar::Int64(1)]),
                ("b", vec![Scalar::Float64(1.5)]),
            ],
        )
        .unwrap();
        let mut overrides = BTreeMap::new();
        overrides.insert("a".to_owned(), "BIGINT".to_owned());
        overrides.insert("b".to_owned(), "DECIMAL(8,4)".to_owned());
        write_sql_with_options(
            &frame,
            &conn,
            "multi_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: Some(overrides),
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect("write with multi-column overrides");
        let sm = super::SqlConnection::query(
            &conn,
            "SELECT sql FROM sqlite_master WHERE name = 'multi_tbl'",
            &[],
        )
        .unwrap();
        let create_sql = match &sm.rows[0][0] {
            Scalar::Utf8(s) => s.clone(),
            other => unreachable!("unexpected sqlite_master payload: {other:?}"),
        };
        assert!(create_sql.contains("BIGINT"));
        assert!(create_sql.contains("DECIMAL(8,4)"));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_dtype_override_for_missing_column_silently_ignored() {
        let conn = make_sql_test_conn();
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1)])],
        )
        .unwrap();
        let mut overrides = BTreeMap::new();
        overrides.insert("nonexistent".to_owned(), "BIGINT".to_owned());
        // No error — pandas silently ignores dtype entries for columns not in the frame.
        write_sql_with_options(
            &frame,
            &conn,
            "missing_col_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: Some(overrides),
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect("write with override on missing col");
        // The actual 'x' column kept its inferred type.
        let sm = super::SqlConnection::query(
            &conn,
            "SELECT sql FROM sqlite_master WHERE name = 'missing_col_tbl'",
            &[],
        )
        .unwrap();
        let create_sql = match &sm.rows[0][0] {
            Scalar::Utf8(s) => s.clone(),
            other => unreachable!("unexpected sqlite_master payload: {other:?}"),
        };
        assert!(create_sql.contains("INTEGER"));
        assert!(!create_sql.contains("BIGINT"));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_dtype_none_falls_back_to_inferred_type() {
        let conn = make_sql_test_conn();
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1)])],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "no_override_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect("write without override");
        let sm = super::SqlConnection::query(
            &conn,
            "SELECT sql FROM sqlite_master WHERE name = 'no_override_tbl'",
            &[],
        )
        .unwrap();
        let create_sql = match &sm.rows[0][0] {
            Scalar::Utf8(s) => s.clone(),
            other => unreachable!("unexpected sqlite_master payload: {other:?}"),
        };
        // INTEGER is conn.dtype_sql(DType::Int64) for rusqlite.
        assert!(create_sql.contains("INTEGER"));
    }

    // ── SqlInsertMethod::Multi (br-frankenpandas-i0ml / fd90.19) ─────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_multi_round_trip_matches_single() {
        // Same frame written via Single vs Multi must produce identical
        // SELECT * results — the only observable difference should be
        // the wire-format efficiency, never the stored values.
        let frame = fp_frame::DataFrame::from_dict(
            &["id", "name", "amount"],
            vec![
                (
                    "id",
                    vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
                ),
                (
                    "name",
                    vec![
                        Scalar::Utf8("alice".into()),
                        Scalar::Utf8("bob".into()),
                        Scalar::Utf8("carol".into()),
                    ],
                ),
                (
                    "amount",
                    vec![
                        Scalar::Float64(1.5),
                        Scalar::Float64(2.5),
                        Scalar::Float64(3.5),
                    ],
                ),
            ],
        )
        .unwrap();

        let conn_single = make_sql_test_conn();
        write_sql_with_options(
            &frame,
            &conn_single,
            "single_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .unwrap();
        let single = read_sql(&conn_single, "SELECT * FROM single_tbl ORDER BY id").unwrap();

        let conn_multi = make_sql_test_conn();
        write_sql_with_options(
            &frame,
            &conn_multi,
            "multi_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Multi,
                chunksize: None,
            },
        )
        .unwrap();
        let multi = read_sql(&conn_multi, "SELECT * FROM multi_tbl ORDER BY id").unwrap();

        assert_eq!(single.column_names(), multi.column_names());
        for name in single.column_names() {
            let s = single.column(name).unwrap().values().to_vec();
            let m = multi.column(name).unwrap().values().to_vec();
            assert_eq!(s, m, "column {name} diverged between Single and Multi");
        }
    }

    #[test]
    fn sql_multi_row_insert_query_emits_correct_placeholder_count() {
        // 3 rows × 2 cols = 6 placeholders, ordinals 1..=6.
        struct PgLikeStub;
        impl super::SqlConnection for PgLikeStub {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn parameter_marker(&self, ordinal: usize) -> String {
                format!("${ordinal}")
            }
        }
        let conn = PgLikeStub;
        let cols = vec!["a".to_owned(), "b".to_owned()];
        let sql =
            super::sql_multi_row_insert_query_in_schema(&conn, "tbl", None, &cols, 3).unwrap();
        // Expect: INSERT INTO "tbl" ("a", "b") VALUES ($1, $2), ($3, $4), ($5, $6)
        assert!(sql.contains("VALUES ($1, $2), ($3, $4), ($5, $6)"), "got: {sql}");
    }

    #[test]
    fn sql_multi_row_insert_query_rejects_zero_rows() {
        struct StubConn;
        impl super::SqlConnection for StubConn {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = StubConn;
        let cols = vec!["a".to_owned()];
        let err = super::sql_multi_row_insert_query_in_schema(&conn, "tbl", None, &cols, 0)
            .expect_err("zero rows must be rejected");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("at least one row")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_multi_chunks_at_max_param_boundary() {
        // Verify the chunking logic dispatches multiple INSERT statements
        // when num_rows * num_cols exceeds max_param_count. We override
        // max_param_count on a recording stub to force a tiny budget.
        use std::cell::RefCell;
        struct ChunkRecorder {
            statements: RefCell<Vec<String>>,
            row_counts: RefCell<Vec<usize>>,
        }
        impl super::SqlConnection for ChunkRecorder {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, sql: &str, rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                self.statements.borrow_mut().push(sql.to_owned());
                self.row_counts
                    .borrow_mut()
                    .push(rows.first().map_or(0, std::vec::Vec::len));
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn max_param_count(&self) -> Option<usize> {
                // 4 params total, ncols=2 → 2 rows per chunk.
                Some(4)
            }
        }
        let conn = ChunkRecorder {
            statements: RefCell::new(vec![]),
            row_counts: RefCell::new(vec![]),
        };
        let frame = fp_frame::DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(4),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(20),
                        Scalar::Int64(30),
                        Scalar::Int64(40),
                        Scalar::Int64(50),
                    ],
                ),
            ],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "chunked",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Multi,
                chunksize: None,
            },
        )
        .unwrap();
        // 5 rows / 2 per chunk = 3 chunks (2, 2, 1).
        let stmts = conn.statements.borrow();
        let counts = conn.row_counts.borrow();
        assert_eq!(stmts.len(), 3, "expected 3 chunked INSERTs");
        // Flat row payloads: 2 rows * 2 cols = 4 scalars, 4, then 1 row * 2 = 2.
        assert_eq!(counts.as_slice(), &[4, 4, 2]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_multi_no_max_param_sends_single_statement() {
        // When the backend reports max_param_count() == None, the whole
        // frame should ship in a single multi-row INSERT.
        use std::cell::RefCell;
        struct UnboundedStub {
            statements: RefCell<Vec<String>>,
        }
        impl super::SqlConnection for UnboundedStub {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                self.statements.borrow_mut().push(sql.to_owned());
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn max_param_count(&self) -> Option<usize> {
                None
            }
        }
        let conn = UnboundedStub {
            statements: RefCell::new(vec![]),
        };
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![(
                "x",
                vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
            )],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "uncapped",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Multi,
                chunksize: None,
            },
        )
        .unwrap();
        let stmts = conn.statements.borrow();
        assert_eq!(stmts.len(), 1, "expected exactly one multi-row INSERT");
        // 3 tuples → 2 commas separating tuples in VALUES (...), (...), (...).
        let stmt = &stmts[0];
        assert_eq!(
            stmt.matches("(?)").count(),
            3,
            "expected 3 row tuples in: {stmt}"
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_multi_preserves_nulls() {
        // NaT/Null values must round-trip through the multi-row path.
        let frame = fp_frame::DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Null(NullKind::Null),
                        Scalar::Int64(3),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Utf8("x".into()),
                        Scalar::Utf8("y".into()),
                        Scalar::Null(NullKind::Null),
                    ],
                ),
            ],
        )
        .unwrap();
        let conn = make_sql_test_conn();
        write_sql_with_options(
            &frame,
            &conn,
            "nulls_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Multi,
                chunksize: None,
            },
        )
        .unwrap();
        let back = read_sql(&conn, "SELECT a, b FROM nulls_tbl ORDER BY rowid").unwrap();
        let a = back.column("a").unwrap().values();
        let b = back.column("b").unwrap().values();
        assert_eq!(a[0], Scalar::Int64(1));
        assert!(matches!(a[1], Scalar::Null(_)));
        assert_eq!(a[2], Scalar::Int64(3));
        assert_eq!(b[0], Scalar::Utf8("x".into()));
        assert_eq!(b[1], Scalar::Utf8("y".into()));
        assert!(matches!(b[2], Scalar::Null(_)));
    }

    #[test]
    fn sql_insert_method_default_is_single() {
        assert_eq!(SqlInsertMethod::default(), SqlInsertMethod::Single);
    }

    // ── list_sql_tables / SqlConnection::list_tables (br-vhq2 / fd90.20) ─

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_tables_empty_db_returns_empty_vec() {
        let conn = make_sql_test_conn();
        let tables = list_sql_tables(&conn, None).unwrap();
        assert!(tables.is_empty(), "expected no tables; got {tables:?}");
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_tables_returns_user_tables_sorted() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE zebra (x INTEGER);").unwrap();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE alpha (y TEXT);").unwrap();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE mango (z REAL);").unwrap();
        let tables = list_sql_tables(&conn, None).unwrap();
        assert_eq!(tables, vec!["alpha", "mango", "zebra"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_tables_excludes_sqlite_internal_tables() {
        let conn = make_sql_test_conn();
        // Forcing creation of an internal sqlite_sequence table by using
        // AUTOINCREMENT — that table must NOT appear in the result.
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE seq_demo (id INTEGER PRIMARY KEY AUTOINCREMENT, v TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "INSERT INTO seq_demo (v) VALUES ('one');",
        )
        .unwrap();
        let tables = list_sql_tables(&conn, None).unwrap();
        assert_eq!(tables, vec!["seq_demo"]);
        assert!(!tables.iter().any(|name| name.starts_with("sqlite_")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_tables_keeps_user_tables_with_sqlite_prefix_no_underscore() {
        // Per fd90.50: a user table named like 'sqliteX' (no underscore
        // between 'sqlite' and the rest) was being incorrectly excluded
        // by the buggy `NOT LIKE 'sqlite_%'` filter (where `_` is a
        // single-char wildcard). After the ESCAPE '\' fix the
        // underscore is treated literally, so only names starting with
        // the literal substring 'sqlite_' are excluded.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE sqliteX (x INTEGER);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE sqliteY (y TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE sqlite1234 (z REAL);",
        )
        .unwrap();
        let tables = list_sql_tables(&conn, None).unwrap();
        assert_eq!(tables, vec!["sqlite1234", "sqliteX", "sqliteY"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_views_keeps_user_views_with_sqlite_prefix_no_underscore() {
        // Companion to the list_tables case (fd90.50): same escape fix
        // applies to list_views.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE base (x INTEGER);").unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE VIEW sqliteX_view AS SELECT x FROM base;",
        )
        .unwrap();
        let views = list_sql_views(&conn, None).unwrap();
        // sqliteX_view: 'sqliteX' (no underscore after 'sqlite') so the
        // literal-underscore filter accepts it.
        assert_eq!(views, vec!["sqliteX_view"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_tables_schema_silently_ignored_on_sqlite() {
        // SQLite reports supports_schemas() == false. Passing a schema
        // arg must NOT error; it is silently dropped and all tables are
        // returned (matches the documented option-struct ignore policy).
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE only_one (x INTEGER);")
            .unwrap();
        let with_schema =
            list_sql_tables(&conn, Some("ignored_on_sqlite")).expect("schema arg must not error");
        let without_schema = list_sql_tables(&conn, None).unwrap();
        assert_eq!(with_schema, without_schema);
    }

    #[test]
    fn list_sql_tables_default_impl_returns_empty() {
        // A backend that doesn't override list_tables falls through to
        // the trait default — returns empty rather than erroring.
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = NoIntrospection;
        let tables = list_sql_tables(&conn, None).unwrap();
        assert!(tables.is_empty());
        let with_schema = list_sql_tables(&conn, Some("any")).unwrap();
        assert!(with_schema.is_empty());
    }

    #[test]
    fn list_sql_tables_routes_schema_to_backend_override() {
        // Multi-schema backend stub: returns different tables per schema.
        struct MultiSchema;
        impl super::SqlConnection for MultiSchema {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn list_tables(&self, schema: Option<&str>) -> Result<Vec<String>, IoError> {
                Ok(match schema {
                    Some("analytics") => {
                        vec!["users".to_owned(), "events".to_owned()]
                    }
                    Some("audit") => vec!["logs".to_owned()],
                    Some(_) => vec![],
                    None => vec!["public_table".to_owned()],
                })
            }
        }
        let conn = MultiSchema;
        assert_eq!(
            list_sql_tables(&conn, Some("analytics")).unwrap(),
            vec!["users", "events"]
        );
        assert_eq!(list_sql_tables(&conn, Some("audit")).unwrap(), vec!["logs"]);
        assert_eq!(
            list_sql_tables(&conn, Some("missing")).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            list_sql_tables(&conn, None).unwrap(),
            vec!["public_table"]
        );
    }

    // ── sql_table_schema / SqlConnection::table_schema (br-w43q / fd90.21) ─

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_unknown_table_returns_none() {
        let conn = make_sql_test_conn();
        let result = sql_table_schema(&conn, "no_such_table", None).unwrap();
        assert!(result.is_none());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_simple_table() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE simple (id INTEGER, name TEXT);",
        )
        .unwrap();
        let schema = sql_table_schema(&conn, "simple", None).unwrap().unwrap();
        assert_eq!(schema.table_name, "simple");
        assert_eq!(schema.columns.len(), 2);
        assert_eq!(schema.columns[0].name, "id");
        assert_eq!(schema.columns[0].declared_type.as_deref(), Some("INTEGER"));
        assert!(schema.columns[0].nullable);
        assert!(schema.columns[0].primary_key_ordinal.is_none());
        assert_eq!(schema.columns[1].name, "name");
        assert_eq!(schema.columns[1].declared_type.as_deref(), Some("TEXT"));
        assert!(schema.columns[1].nullable);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_pk_notnull_default() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE meta ( \
                id INTEGER PRIMARY KEY, \
                name TEXT NOT NULL, \
                status TEXT DEFAULT 'active' \
             );",
        )
        .unwrap();
        let schema = sql_table_schema(&conn, "meta", None).unwrap().unwrap();
        assert_eq!(schema.columns.len(), 3);

        let id = schema.column("id").expect("id col");
        assert_eq!(id.primary_key_ordinal, Some(0));
        // PRIMARY KEY columns in SQLite (without explicit NOT NULL on
        // INTEGER PRIMARY KEY) are reported as nullable=true by
        // table_info — we surface that as-is rather than fabricating.
        // The point is just that primary_key_ordinal is populated.

        let name = schema.column("name").expect("name col");
        assert!(!name.nullable);
        assert!(name.default_value.is_none());
        assert!(name.primary_key_ordinal.is_none());

        let status = schema.column("status").expect("status col");
        assert!(status.nullable);
        assert_eq!(
            status.default_value.as_deref(),
            Some("'active'"),
            "expected SQL literal default text"
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_schema_silently_ignored_on_sqlite() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE only_one (x INTEGER);")
            .unwrap();
        let with_schema = sql_table_schema(&conn, "only_one", Some("ignored_on_sqlite"))
            .expect("schema arg must not error")
            .expect("table exists");
        let without_schema = sql_table_schema(&conn, "only_one", None).unwrap().unwrap();
        assert_eq!(with_schema, without_schema);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_rejects_invalid_table_name() {
        // The PRAGMA path can't bind parameters, so we validate the
        // identifier first. Reject anything with non-alphanumeric chars.
        let conn = make_sql_test_conn();
        let err =
            sql_table_schema(&conn, "x; DROP TABLE users", None).expect_err("must reject");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid")));
    }

    #[test]
    fn sql_table_schema_default_impl_returns_none() {
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = NoIntrospection;
        assert!(sql_table_schema(&conn, "anything", None).unwrap().is_none());
    }

    #[test]
    fn sql_table_schema_routes_schema_to_backend_override() {
        struct MultiSchema;
        impl super::SqlConnection for MultiSchema {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn table_schema(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                if table == "users" && schema == Some("analytics") {
                    Ok(Some(SqlTableSchema {
                        table_name: "users".to_owned(),
                        columns: vec![SqlColumnSchema {
                            name: "id".to_owned(),
                            declared_type: Some("BIGINT".to_owned()),
                            nullable: false,
                            default_value: None,
                            primary_key_ordinal: Some(0),
                            comment: None,
                            autoincrement: false,
                        }],
                    }))
                } else {
                    Ok(None)
                }
            }
        }
        let conn = MultiSchema;
        let analytics_users = sql_table_schema(&conn, "users", Some("analytics"))
            .unwrap()
            .expect("found");
        assert_eq!(analytics_users.columns[0].declared_type.as_deref(), Some("BIGINT"));
        assert!(sql_table_schema(&conn, "users", Some("audit")).unwrap().is_none());
        assert!(sql_table_schema(&conn, "users", None).unwrap().is_none());
    }

    // ── list_sql_schemas / SqlConnection::list_schemas (br-lxhi / fd90.22) ─

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_schemas_returns_empty_on_sqlite() {
        // SQLite has no meaningful schema concept; the trait default
        // (empty Vec) is the right answer.
        let conn = make_sql_test_conn();
        let schemas = list_sql_schemas(&conn).unwrap();
        assert!(schemas.is_empty(), "expected no schemas; got {schemas:?}");
    }

    #[test]
    fn list_sql_schemas_default_impl_returns_empty() {
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = NoIntrospection;
        assert!(list_sql_schemas(&conn).unwrap().is_empty());
    }

    #[test]
    fn list_sql_schemas_routes_to_backend_override() {
        // Multi-schema backend stub: returns the schemas the connection's
        // role can see, with internal schemas filtered out.
        struct MultiSchemaServer;
        impl super::SqlConnection for MultiSchemaServer {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn list_schemas(&self) -> Result<Vec<String>, IoError> {
                // Filter out information_schema + pg_catalog by default.
                Ok(vec![
                    "public".to_owned(),
                    "analytics".to_owned(),
                    "audit".to_owned(),
                ])
            }
        }
        let conn = MultiSchemaServer;
        let schemas = list_sql_schemas(&conn).unwrap();
        assert_eq!(schemas, vec!["public", "analytics", "audit"]);
        // Verify the override actually filters internal schemas
        // (test contract: stub never returns 'pg_catalog' or
        // 'information_schema').
        assert!(!schemas.iter().any(|s| s.starts_with("pg_")));
        assert!(!schemas.iter().any(|s| s == "information_schema"));
    }

    #[test]
    fn list_sql_schemas_propagates_backend_error() {
        // If the backend errors during introspection, the error should
        // surface through the wrapper unchanged.
        struct BrokenIntrospection;
        impl super::SqlConnection for BrokenIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn list_schemas(&self) -> Result<Vec<String>, IoError> {
                Err(IoError::Sql("permission denied for catalog".to_owned()))
            }
        }
        let conn = BrokenIntrospection;
        let err = list_sql_schemas(&conn).expect_err("should surface backend error");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("permission denied")));
    }

    // ── truncate_sql_table / SqlConnection::truncate_table (br-phum / fd90.23) ─

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn truncate_sql_table_clears_rows_but_preserves_schema() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE rolling (id INTEGER, val TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "INSERT INTO rolling VALUES (1, 'a'), (2, 'b'), (3, 'c');",
        )
        .unwrap();
        // Sanity: rows present.
        let before = super::SqlConnection::query(&conn, "SELECT COUNT(*) FROM rolling", &[])
            .unwrap();
        assert_eq!(before.rows[0][0], Scalar::Int64(3));

        truncate_sql_table(&conn, "rolling", None).unwrap();

        // Rows gone, table still there.
        let after = super::SqlConnection::query(&conn, "SELECT COUNT(*) FROM rolling", &[])
            .unwrap();
        assert_eq!(after.rows[0][0], Scalar::Int64(0));
        assert!(super::SqlConnection::table_exists(&conn, "rolling").unwrap());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn truncate_sql_table_schema_silently_ignored_on_sqlite() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t (x INTEGER);").unwrap();
        super::SqlConnection::execute_batch(&conn, "INSERT INTO t VALUES (1);").unwrap();
        truncate_sql_table(&conn, "t", Some("ignored_on_sqlite"))
            .expect("schema arg must not error on SQLite");
        let count = super::SqlConnection::query(&conn, "SELECT COUNT(*) FROM t", &[]).unwrap();
        assert_eq!(count.rows[0][0], Scalar::Int64(0));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn truncate_sql_table_rejects_invalid_table_name() {
        let conn = make_sql_test_conn();
        let err = truncate_sql_table(&conn, "x; DROP TABLE users", None)
            .expect_err("must reject invalid identifier");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid")));
    }

    #[test]
    fn truncate_sql_table_routes_schema_to_quote_identifier() {
        // Multi-schema backend stub records the SQL it receives.
        use std::cell::RefCell;
        struct PgLikeRecorder {
            statements: RefCell<Vec<String>>,
        }
        impl super::SqlConnection for PgLikeRecorder {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, sql: &str) -> Result<(), IoError> {
                self.statements.borrow_mut().push(sql.to_owned());
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
        }
        let conn = PgLikeRecorder {
            statements: RefCell::new(vec![]),
        };
        truncate_sql_table(&conn, "events", Some("analytics")).unwrap();
        let stmts = conn.statements.borrow();
        assert_eq!(stmts.len(), 1);
        // Default impl uses DELETE FROM ... and quote_identifier on
        // both schema + table parts.
        assert!(
            stmts[0].contains("DELETE FROM \"analytics\".\"events\""),
            "expected schema-qualified DELETE; got: {}",
            stmts[0]
        );
    }

    #[test]
    fn truncate_sql_table_backend_override_uses_truncate_keyword() {
        // PG/MySQL impls would override with TRUNCATE TABLE for speed.
        // Verify the trait dispatch picks up the override.
        use std::cell::RefCell;
        struct FastTruncate {
            statements: RefCell<Vec<String>>,
        }
        impl super::SqlConnection for FastTruncate {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, sql: &str) -> Result<(), IoError> {
                self.statements.borrow_mut().push(sql.to_owned());
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn truncate_table(
                &self,
                table_name: &str,
                _schema: Option<&str>,
            ) -> Result<(), IoError> {
                self.execute_batch(&format!("TRUNCATE TABLE \"{table_name}\""))
            }
        }
        let conn = FastTruncate {
            statements: RefCell::new(vec![]),
        };
        truncate_sql_table(&conn, "events", None).unwrap();
        let stmts = conn.statements.borrow();
        assert_eq!(stmts.len(), 1);
        assert!(stmts[0].starts_with("TRUNCATE TABLE"), "got: {}", stmts[0]);
    }

    // ── sql_server_version / SqlConnection::server_version (br-e23k / fd90.24) ─

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_server_version_returns_sqlite_version_string() {
        let conn = make_sql_test_conn();
        let version = sql_server_version(&conn).unwrap().expect("SQLite reports version");
        // Expect dotted-version format like "3.45.1". The parts must be
        // non-empty digits — exact value depends on the bundled SQLite.
        let parts: Vec<&str> = version.split('.').collect();
        assert!(
            parts.len() >= 2,
            "expected dotted version; got: {version}"
        );
        for part in &parts {
            assert!(
                !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()),
                "expected numeric version parts; got {version}"
            );
        }
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_server_version_starts_with_three_for_sqlite_3_x() {
        // SQLite has been at major version 3 since 2004; bundled
        // rusqlite is current (3.40+), so the major must be "3".
        let conn = make_sql_test_conn();
        let version = sql_server_version(&conn).unwrap().unwrap();
        assert!(version.starts_with("3."), "expected SQLite 3.x; got {version}");
    }

    #[test]
    fn sql_server_version_default_impl_returns_none() {
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = NoIntrospection;
        assert!(sql_server_version(&conn).unwrap().is_none());
    }

    #[test]
    fn sql_server_version_routes_to_backend_override() {
        struct PgLikeStub;
        impl super::SqlConnection for PgLikeStub {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn server_version(&self) -> Result<Option<String>, IoError> {
                // Mimic `SHOW server_version` payload.
                Ok(Some("16.2".to_owned()))
            }
        }
        let conn = PgLikeStub;
        assert_eq!(sql_server_version(&conn).unwrap().as_deref(), Some("16.2"));
    }

    #[test]
    fn sql_server_version_propagates_backend_error() {
        struct BrokenIntrospection;
        impl super::SqlConnection for BrokenIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn server_version(&self) -> Result<Option<String>, IoError> {
                Err(IoError::Sql("connection lost".to_owned()))
            }
        }
        let conn = BrokenIntrospection;
        let err = sql_server_version(&conn).expect_err("should surface backend error");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("connection lost")));
    }

    // ── sql_primary_key_columns / SqlConnection::primary_key_columns
    //    (br-uw3y / fd90.25) ────────────────────────────────────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_primary_key_columns_unknown_table_returns_empty() {
        let conn = make_sql_test_conn();
        let pk = sql_primary_key_columns(&conn, "no_such_table", None).unwrap();
        assert!(pk.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_primary_key_columns_table_without_pk_returns_empty() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE no_pk (a INTEGER, b TEXT);",
        )
        .unwrap();
        let pk = sql_primary_key_columns(&conn, "no_pk", None).unwrap();
        assert!(pk.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_primary_key_columns_single_pk() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE single_pk (id INTEGER PRIMARY KEY, name TEXT);",
        )
        .unwrap();
        let pk = sql_primary_key_columns(&conn, "single_pk", None).unwrap();
        assert_eq!(pk, vec!["id"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_primary_key_columns_composite_pk_ordered_by_ordinal() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE composite ( \
                year INTEGER NOT NULL, \
                month INTEGER NOT NULL, \
                code TEXT NOT NULL, \
                value REAL, \
                PRIMARY KEY (year, month, code) \
             );",
        )
        .unwrap();
        let pk = sql_primary_key_columns(&conn, "composite", None).unwrap();
        // PK declaration order: year, month, code.
        assert_eq!(pk, vec!["year", "month", "code"]);
    }

    #[test]
    fn sql_primary_key_columns_default_impl_returns_empty_when_no_introspection() {
        // Backend with no table_schema override returns Ok(None) →
        // primary_key_columns falls through to empty Vec.
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = NoIntrospection;
        assert!(sql_primary_key_columns(&conn, "anything", None).unwrap().is_empty());
    }

    #[test]
    fn sql_primary_key_columns_routes_schema_to_table_schema_override() {
        // Backend that returns ordinal-sorted PK columns from a
        // multi-schema introspection.
        struct MultiSchemaPk;
        impl super::SqlConnection for MultiSchemaPk {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn table_schema(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                if table == "events" && schema == Some("analytics") {
                    Ok(Some(SqlTableSchema {
                        table_name: "events".to_owned(),
                        columns: vec![
                            // Intentionally out-of-declaration-order to
                            // verify the helper sorts by ordinal.
                            SqlColumnSchema {
                                name: "code".to_owned(),
                                declared_type: Some("TEXT".to_owned()),
                                nullable: false,
                                default_value: None,
                                primary_key_ordinal: Some(2),
                                comment: None,
                                autoincrement: false,
                            },
                            SqlColumnSchema {
                                name: "year".to_owned(),
                                declared_type: Some("INTEGER".to_owned()),
                                nullable: false,
                                default_value: None,
                                primary_key_ordinal: Some(0),
                                comment: None,
                                autoincrement: false,
                            },
                            SqlColumnSchema {
                                name: "value".to_owned(),
                                declared_type: Some("REAL".to_owned()),
                                nullable: true,
                                default_value: None,
                                primary_key_ordinal: None,
                                comment: None,
                                autoincrement: false,
                            },
                            SqlColumnSchema {
                                name: "month".to_owned(),
                                declared_type: Some("INTEGER".to_owned()),
                                nullable: false,
                                default_value: None,
                                primary_key_ordinal: Some(1),
                                comment: None,
                                autoincrement: false,
                            },
                        ],
                    }))
                } else {
                    Ok(None)
                }
            }
        }
        let conn = MultiSchemaPk;
        let pk = sql_primary_key_columns(&conn, "events", Some("analytics")).unwrap();
        // Sorted by primary_key_ordinal: 0=year, 1=month, 2=code.
        assert_eq!(pk, vec!["year", "month", "code"]);
        // Wrong schema → empty (table_schema returns None).
        assert!(sql_primary_key_columns(&conn, "events", Some("audit")).unwrap().is_empty());
    }

    // ── sql_max_identifier_length / SqlConnection::max_identifier_length
    //    (br-cs81 / fd90.26) ────────────────────────────────────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_max_identifier_length_returns_none_on_sqlite() {
        // SQLite has no documented identifier-length limit; the trait
        // default (None) is the right answer.
        let conn = make_sql_test_conn();
        assert_eq!(sql_max_identifier_length(&conn), None);
    }

    #[test]
    fn sql_max_identifier_length_default_impl_returns_none() {
        struct Generic;
        impl super::SqlConnection for Generic {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        assert_eq!(sql_max_identifier_length(&Generic), None);
    }

    #[test]
    fn sql_max_identifier_length_pg_override_reports_63() {
        struct PgLikeStub;
        impl super::SqlConnection for PgLikeStub {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn max_identifier_length(&self) -> Option<usize> {
                Some(63)
            }
        }
        assert_eq!(sql_max_identifier_length(&PgLikeStub), Some(63));
    }

    #[test]
    fn sql_max_identifier_length_mysql_override_reports_64() {
        struct MySqlLikeStub;
        impl super::SqlConnection for MySqlLikeStub {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn max_identifier_length(&self) -> Option<usize> {
                Some(64)
            }
        }
        assert_eq!(sql_max_identifier_length(&MySqlLikeStub), Some(64));
    }

    #[test]
    fn sql_max_identifier_length_mssql_override_reports_128() {
        struct MsSqlLikeStub;
        impl super::SqlConnection for MsSqlLikeStub {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn max_identifier_length(&self) -> Option<usize> {
                Some(128)
            }
        }
        assert_eq!(sql_max_identifier_length(&MsSqlLikeStub), Some(128));
    }

    // ── write_sql identifier-length validation (br-9ynk / fd90.27) ────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_long_column_name_succeeds_on_sqlite() {
        // SQLite reports max_identifier_length() == None → no validation.
        let conn = make_sql_test_conn();
        // 80 chars > PG/MySQL caps but fine on SQLite.
        let long_col: String = std::iter::repeat_n('a', 80).collect();
        let frame = fp_frame::DataFrame::from_dict(
            &[long_col.as_str()],
            vec![(long_col.as_str(), vec![Scalar::Int64(1)])],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "long_col_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect("SQLite has no identifier limit");
    }

    fn make_pg_like_recorder()
    -> impl super::SqlConnection + 'static {
        // Stub PG-like backend: enforces 63-char limit, accepts all
        // execute_batch / insert_rows so write_sql can reach the
        // identifier-length check before failing on emit.
        struct PgLikeLimit;
        impl super::SqlConnection for PgLikeLimit {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn max_identifier_length(&self) -> Option<usize> {
                Some(63)
            }
            fn supports_schemas(&self) -> bool {
                true
            }
        }
        PgLikeLimit
    }

    #[test]
    fn write_sql_rejects_long_column_name_on_pg_like_backend() {
        let conn = make_pg_like_recorder();
        let long_col: String = std::iter::repeat_n('c', 64).collect();
        let frame = fp_frame::DataFrame::from_dict(
            &[long_col.as_str()],
            vec![(long_col.as_str(), vec![Scalar::Int64(1)])],
        )
        .unwrap();
        let err = write_sql_with_options(
            &frame,
            &conn,
            "ok_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect_err("64-char column must exceed PG limit");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("column") && msg.contains("63")));
    }

    #[test]
    fn write_sql_rejects_long_table_name_on_pg_like_backend() {
        let conn = make_pg_like_recorder();
        // 64-char identifier (table names also subject to the PG cap).
        // Use only alphanumeric so validate_sql_table_name passes first.
        let long_tbl: String = std::iter::repeat_n('t', 64).collect();
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1)])],
        )
        .unwrap();
        let err = write_sql_with_options(
            &frame,
            &conn,
            &long_tbl,
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect_err("64-char table must exceed PG limit");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("table") && msg.contains("63")));
    }

    #[test]
    fn write_sql_rejects_long_index_label_on_pg_like_backend() {
        let conn = make_pg_like_recorder();
        let long_label: String = std::iter::repeat_n('i', 64).collect();
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1)])],
        )
        .unwrap();
        let err = write_sql_with_options(
            &frame,
            &conn,
            "ok_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: true,
                index_label: Some(long_label),
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect_err("64-char index label must exceed PG limit");
        assert!(
            matches!(err, IoError::Sql(msg) if msg.contains("index label") && msg.contains("63"))
        );
    }

    #[test]
    fn write_sql_rejects_long_schema_name_on_pg_like_backend() {
        let conn = make_pg_like_recorder();
        let long_schema: String = std::iter::repeat_n('s', 64).collect();
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1)])],
        )
        .unwrap();
        let err = write_sql_with_options(
            &frame,
            &conn,
            "ok_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: Some(long_schema),
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect_err("64-char schema must exceed PG limit");
        assert!(
            matches!(err, IoError::Sql(msg) if msg.contains("schema") && msg.contains("63"))
        );
    }

    #[test]
    fn write_sql_just_at_the_boundary_is_accepted() {
        let conn = make_pg_like_recorder();
        // Exactly 63 chars: at the PG limit, must be accepted.
        let just_fits: String = std::iter::repeat_n('a', 63).collect();
        let frame = fp_frame::DataFrame::from_dict(
            &[just_fits.as_str()],
            vec![(just_fits.as_str(), vec![Scalar::Int64(1)])],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "ok_tbl",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .expect("63-char column at boundary should be accepted");
    }

    // ── list_sql_indexes / SqlConnection::list_indexes (br-bgv9 / fd90.28) ─

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_indexes_unknown_table_returns_empty() {
        let conn = make_sql_test_conn();
        let indexes = list_sql_indexes(&conn, "no_such_tbl", None).unwrap();
        assert!(indexes.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_indexes_table_without_indexes() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE plain (a INTEGER, b TEXT);",
        )
        .unwrap();
        let indexes = list_sql_indexes(&conn, "plain", None).unwrap();
        assert!(indexes.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_indexes_single_column() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE events (id INTEGER, ts TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE INDEX idx_events_ts ON events (ts);",
        )
        .unwrap();
        let indexes = list_sql_indexes(&conn, "events", None).unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].name, "idx_events_ts");
        assert_eq!(indexes[0].columns, vec!["ts"]);
        assert!(!indexes[0].unique);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_indexes_unique_index() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE users (id INTEGER, email TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE UNIQUE INDEX idx_users_email ON users (email);",
        )
        .unwrap();
        let indexes = list_sql_indexes(&conn, "users", None).unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].name, "idx_users_email");
        assert_eq!(indexes[0].columns, vec!["email"]);
        assert!(indexes[0].unique);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_indexes_composite_columns_in_definition_order() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE rolling (year INT, month INT, code TEXT, val REAL);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE INDEX idx_rolling_y_m_c ON rolling (year, month, code);",
        )
        .unwrap();
        let indexes = list_sql_indexes(&conn, "rolling", None).unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].columns, vec!["year", "month", "code"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_indexes_filters_pk_auto_index() {
        // INTEGER PRIMARY KEY in SQLite creates an automatic index that
        // SQLAlchemy.Inspector hides. We must hide it too — only the
        // explicit CREATE INDEX should surface.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE pk_only (id INTEGER PRIMARY KEY, name TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE INDEX idx_pk_only_name ON pk_only (name);",
        )
        .unwrap();
        let indexes = list_sql_indexes(&conn, "pk_only", None).unwrap();
        // Only the explicit user index should appear.
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].name, "idx_pk_only_name");
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_indexes_rejects_invalid_table_name() {
        let conn = make_sql_test_conn();
        let err = list_sql_indexes(&conn, "x; DROP TABLE users", None)
            .expect_err("must reject invalid identifier");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid")));
    }

    #[test]
    fn list_sql_indexes_default_impl_returns_empty() {
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        assert!(list_sql_indexes(&NoIntrospection, "anything", None).unwrap().is_empty());
    }

    #[test]
    fn list_sql_indexes_routes_to_backend_override() {
        struct MultiSchemaIdx;
        impl super::SqlConnection for MultiSchemaIdx {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn list_indexes(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Vec<SqlIndexSchema>, IoError> {
                if table == "events" && schema == Some("analytics") {
                    Ok(vec![
                        SqlIndexSchema {
                            name: "idx_events_ts".to_owned(),
                            columns: vec!["ts".to_owned()],
                            unique: false,
                        },
                        SqlIndexSchema {
                            name: "uq_events_uid".to_owned(),
                            columns: vec!["user_id".to_owned()],
                            unique: true,
                        },
                    ])
                } else {
                    Ok(vec![])
                }
            }
        }
        let conn = MultiSchemaIdx;
        let indexes = list_sql_indexes(&conn, "events", Some("analytics")).unwrap();
        assert_eq!(indexes.len(), 2);
        assert!(indexes.iter().any(|i| i.unique && i.name == "uq_events_uid"));
        // Wrong schema → empty (override scopes by Some).
        assert!(list_sql_indexes(&conn, "events", Some("audit")).unwrap().is_empty());
    }

    // ── list_sql_foreign_keys / SqlConnection::list_foreign_keys
    //    (br-uht8 / fd90.29) ────────────────────────────────────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_foreign_keys_unknown_table_returns_empty() {
        let conn = make_sql_test_conn();
        let fks = list_sql_foreign_keys(&conn, "no_such_tbl", None).unwrap();
        assert!(fks.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_foreign_keys_table_without_fk() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE plain (a INTEGER, b TEXT);",
        )
        .unwrap();
        let fks = list_sql_foreign_keys(&conn, "plain", None).unwrap();
        assert!(fks.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_foreign_keys_single_column_fk() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE parent (id INTEGER PRIMARY KEY, label TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE child (cid INTEGER, parent_id INTEGER, \
             FOREIGN KEY (parent_id) REFERENCES parent(id));",
        )
        .unwrap();
        let fks = list_sql_foreign_keys(&conn, "child", None).unwrap();
        assert_eq!(fks.len(), 1);
        assert_eq!(fks[0].columns, vec!["parent_id"]);
        assert_eq!(fks[0].referenced_table, "parent");
        assert_eq!(fks[0].referenced_columns, vec!["id"]);
        // SQLite PRAGMA does not surface constraint names.
        assert!(fks[0].constraint_name.is_none());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_foreign_keys_composite_fk_ordered_by_seq() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE rolling ( \
                year INTEGER NOT NULL, \
                month INTEGER NOT NULL, \
                code TEXT NOT NULL, \
                PRIMARY KEY (year, month, code) \
             );",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE rolling_fact ( \
                fact_id INTEGER, year INTEGER, month INTEGER, code TEXT, \
                FOREIGN KEY (year, month, code) \
                  REFERENCES rolling(year, month, code) \
             );",
        )
        .unwrap();
        let fks = list_sql_foreign_keys(&conn, "rolling_fact", None).unwrap();
        assert_eq!(fks.len(), 1);
        // Pairs preserved in declaration order (seq=0,1,2).
        assert_eq!(fks[0].columns, vec!["year", "month", "code"]);
        assert_eq!(fks[0].referenced_columns, vec!["year", "month", "code"]);
        assert_eq!(fks[0].referenced_table, "rolling");
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_foreign_keys_multiple_fks_on_one_table() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE users (id INTEGER PRIMARY KEY);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE products (sku TEXT PRIMARY KEY);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE orders ( \
                oid INTEGER, \
                user_id INTEGER, \
                product_sku TEXT, \
                FOREIGN KEY (user_id) REFERENCES users(id), \
                FOREIGN KEY (product_sku) REFERENCES products(sku) \
             );",
        )
        .unwrap();
        let fks = list_sql_foreign_keys(&conn, "orders", None).unwrap();
        assert_eq!(fks.len(), 2);
        let user_fk = fks.iter().find(|f| f.referenced_table == "users").unwrap();
        assert_eq!(user_fk.columns, vec!["user_id"]);
        assert_eq!(user_fk.referenced_columns, vec!["id"]);
        let prod_fk = fks.iter().find(|f| f.referenced_table == "products").unwrap();
        assert_eq!(prod_fk.columns, vec!["product_sku"]);
        assert_eq!(prod_fk.referenced_columns, vec!["sku"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_foreign_keys_rejects_invalid_table_name() {
        let conn = make_sql_test_conn();
        let err = list_sql_foreign_keys(&conn, "x; DROP TABLE users", None)
            .expect_err("must reject invalid identifier");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_foreign_keys_resolves_implicit_pk_single_column() {
        // Per fd90.44: FOREIGN KEY (parent_id) REFERENCES parent
        // (no column list) is an implicit reference to parent's PK.
        // SQLite returns NULL for the 'to' column; we now resolve via
        // the parent's primary_key_columns.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE imp_parent (pid INTEGER PRIMARY KEY, label TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE imp_child ( \
                cid INTEGER, \
                parent_id INTEGER, \
                FOREIGN KEY (parent_id) REFERENCES imp_parent \
             );",
        )
        .unwrap();
        let fks = list_sql_foreign_keys(&conn, "imp_child", None).unwrap();
        assert_eq!(fks.len(), 1, "implicit-PK FK must surface (was being silently dropped)");
        assert_eq!(fks[0].columns, vec!["parent_id"]);
        assert_eq!(fks[0].referenced_table, "imp_parent");
        // resolved from parent's PK.
        assert_eq!(fks[0].referenced_columns, vec!["pid"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_foreign_keys_resolves_implicit_pk_composite() {
        // Composite FK with implicit reference to composite PK.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE imp_parent_comp ( \
                year INTEGER NOT NULL, \
                month INTEGER NOT NULL, \
                PRIMARY KEY (year, month) \
             );",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE imp_child_comp ( \
                cid INTEGER, \
                fyear INTEGER NOT NULL, \
                fmonth INTEGER NOT NULL, \
                FOREIGN KEY (fyear, fmonth) REFERENCES imp_parent_comp \
             );",
        )
        .unwrap();
        let fks = list_sql_foreign_keys(&conn, "imp_child_comp", None).unwrap();
        assert_eq!(fks.len(), 1);
        assert_eq!(fks[0].columns, vec!["fyear", "fmonth"]);
        assert_eq!(fks[0].referenced_table, "imp_parent_comp");
        // Resolved from composite PK in declaration order.
        assert_eq!(fks[0].referenced_columns, vec!["year", "month"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_foreign_keys_explicit_columns_unchanged() {
        // Existing behavior preserved: explicit columns still
        // round-trip exactly as before fd90.44.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE exp_parent (pid INTEGER PRIMARY KEY);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE exp_child ( \
                cid INTEGER, \
                parent_id INTEGER, \
                FOREIGN KEY (parent_id) REFERENCES exp_parent(pid) \
             );",
        )
        .unwrap();
        let fks = list_sql_foreign_keys(&conn, "exp_child", None).unwrap();
        assert_eq!(fks.len(), 1);
        assert_eq!(fks[0].columns, vec!["parent_id"]);
        assert_eq!(fks[0].referenced_columns, vec!["pid"]);
    }

    #[test]
    fn list_sql_foreign_keys_default_impl_returns_empty() {
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        assert!(
            list_sql_foreign_keys(&NoIntrospection, "anything", None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn list_sql_foreign_keys_routes_to_backend_override() {
        struct MultiSchemaFk;
        impl super::SqlConnection for MultiSchemaFk {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn list_foreign_keys(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Vec<SqlForeignKeySchema>, IoError> {
                if table == "orders" && schema == Some("sales") {
                    Ok(vec![SqlForeignKeySchema {
                        constraint_name: Some("orders_user_fk".to_owned()),
                        columns: vec!["user_id".to_owned()],
                        referenced_table: "users".to_owned(),
                        referenced_columns: vec!["id".to_owned()],
                    }])
                } else {
                    Ok(vec![])
                }
            }
        }
        let conn = MultiSchemaFk;
        let fks = list_sql_foreign_keys(&conn, "orders", Some("sales")).unwrap();
        assert_eq!(fks.len(), 1);
        assert_eq!(fks[0].constraint_name.as_deref(), Some("orders_user_fk"));
        assert_eq!(fks[0].referenced_table, "users");
        // Wrong schema → empty (override scopes by Some).
        assert!(list_sql_foreign_keys(&conn, "orders", Some("audit")).unwrap().is_empty());
    }

    // ── list_sql_views / SqlConnection::list_views (br-gm3r / fd90.30) ────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_views_empty_db_returns_empty() {
        let conn = make_sql_test_conn();
        let views = list_sql_views(&conn, None).unwrap();
        assert!(views.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_views_returns_user_views_sorted() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE base (id INTEGER, val TEXT);")
            .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE VIEW zebra_view AS SELECT id FROM base;",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE VIEW alpha_view AS SELECT val FROM base;",
        )
        .unwrap();
        let views = list_sql_views(&conn, None).unwrap();
        assert_eq!(views, vec!["alpha_view", "zebra_view"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_views_separated_from_list_tables() {
        // list_views must NOT surface tables; list_tables must NOT surface
        // views. The two buckets are disjoint per SQLAlchemy.Inspector.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE just_tbl (x INTEGER);").unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE VIEW just_view AS SELECT x FROM just_tbl;",
        )
        .unwrap();

        let tables = list_sql_tables(&conn, None).unwrap();
        let views = list_sql_views(&conn, None).unwrap();
        assert_eq!(tables, vec!["just_tbl"]);
        assert_eq!(views, vec!["just_view"]);
        assert!(!tables.contains(&"just_view".to_owned()));
        assert!(!views.contains(&"just_tbl".to_owned()));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_views_schema_silently_ignored_on_sqlite() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t (x INTEGER);").unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE VIEW v AS SELECT x FROM t;",
        )
        .unwrap();
        let with_schema =
            list_sql_views(&conn, Some("ignored_on_sqlite")).expect("schema arg must not error");
        let without_schema = list_sql_views(&conn, None).unwrap();
        assert_eq!(with_schema, without_schema);
    }

    #[test]
    fn list_sql_views_default_impl_returns_empty() {
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = NoIntrospection;
        assert!(list_sql_views(&conn, None).unwrap().is_empty());
        assert!(list_sql_views(&conn, Some("any")).unwrap().is_empty());
    }

    #[test]
    fn list_sql_views_routes_schema_to_backend_override() {
        struct MultiSchemaViews;
        impl super::SqlConnection for MultiSchemaViews {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn list_views(&self, schema: Option<&str>) -> Result<Vec<String>, IoError> {
                Ok(match schema {
                    Some("reporting") => vec!["daily".to_owned(), "weekly".to_owned()],
                    Some("audit") => vec!["log_view".to_owned()],
                    _ => vec![],
                })
            }
        }
        let conn = MultiSchemaViews;
        assert_eq!(
            list_sql_views(&conn, Some("reporting")).unwrap(),
            vec!["daily", "weekly"]
        );
        assert_eq!(
            list_sql_views(&conn, Some("audit")).unwrap(),
            vec!["log_view"]
        );
        assert!(list_sql_views(&conn, None).unwrap().is_empty());
    }

    // ── list_sql_unique_constraints / SqlConnection::list_unique_constraints
    //    (br-sh4v / fd90.31) ────────────────────────────────────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_unique_constraints_unknown_table_returns_empty() {
        let conn = make_sql_test_conn();
        let uqs = list_sql_unique_constraints(&conn, "no_such", None).unwrap();
        assert!(uqs.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_unique_constraints_table_without_uq() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE plain (a INTEGER, b TEXT);")
            .unwrap();
        let uqs = list_sql_unique_constraints(&conn, "plain", None).unwrap();
        assert!(uqs.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_unique_constraints_inline_unique() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT UNIQUE);",
        )
        .unwrap();
        let uqs = list_sql_unique_constraints(&conn, "users", None).unwrap();
        assert_eq!(uqs.len(), 1);
        assert_eq!(uqs[0].columns, vec!["email"]);
        // SQLite gives backend-generated names like sqlite_autoindex_users_1.
        assert!(
            uqs[0].name.starts_with("sqlite_autoindex_users_"),
            "expected sqlite_autoindex_ name; got {}",
            uqs[0].name
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_unique_constraints_composite_table_constraint() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE rolling ( \
                year INTEGER, month INTEGER, code TEXT, val REAL, \
                UNIQUE (year, month, code) \
             );",
        )
        .unwrap();
        let uqs = list_sql_unique_constraints(&conn, "rolling", None).unwrap();
        assert_eq!(uqs.len(), 1);
        assert_eq!(uqs[0].columns, vec!["year", "month", "code"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_unique_constraints_disjoint_from_create_unique_index() {
        // Per SQLAlchemy: get_unique_constraints surfaces declared UNIQUE
        // constraints (origin='u'); get_indexes surfaces user-created
        // CREATE UNIQUE INDEX (origin='c'). The two must be disjoint.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE mixed ( \
                a INTEGER, \
                b TEXT, \
                c TEXT, \
                UNIQUE (a) \
             );",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE UNIQUE INDEX idx_mixed_b ON mixed (b);",
        )
        .unwrap();

        let uqs = list_sql_unique_constraints(&conn, "mixed", None).unwrap();
        let idxs = list_sql_indexes(&conn, "mixed", None).unwrap();

        // The UNIQUE constraint is in uqs only.
        assert_eq!(uqs.len(), 1);
        assert_eq!(uqs[0].columns, vec!["a"]);
        // The CREATE UNIQUE INDEX is in idxs only.
        assert_eq!(idxs.len(), 1);
        assert_eq!(idxs[0].name, "idx_mixed_b");
        assert!(idxs[0].unique);
        assert_eq!(idxs[0].columns, vec!["b"]);

        // No overlap by name (uqs uses sqlite_autoindex_, idxs uses idx_).
        assert!(!uqs.iter().any(|u| u.name == "idx_mixed_b"));
        assert!(!idxs.iter().any(|i| i.name.starts_with("sqlite_autoindex_")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn list_sql_unique_constraints_rejects_invalid_table_name() {
        let conn = make_sql_test_conn();
        let err = list_sql_unique_constraints(&conn, "x; DROP TABLE users", None)
            .expect_err("must reject invalid identifier");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid")));
    }

    #[test]
    fn list_sql_unique_constraints_default_impl_returns_empty() {
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        assert!(
            list_sql_unique_constraints(&NoIntrospection, "anything", None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn list_sql_unique_constraints_routes_to_backend_override() {
        struct MultiSchemaUq;
        impl super::SqlConnection for MultiSchemaUq {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn list_unique_constraints(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Vec<SqlUniqueConstraintSchema>, IoError> {
                if table == "users" && schema == Some("public") {
                    Ok(vec![SqlUniqueConstraintSchema {
                        name: "users_email_key".to_owned(),
                        columns: vec!["email".to_owned()],
                    }])
                } else {
                    Ok(vec![])
                }
            }
        }
        let conn = MultiSchemaUq;
        let uqs = list_sql_unique_constraints(&conn, "users", Some("public")).unwrap();
        assert_eq!(uqs.len(), 1);
        assert_eq!(uqs[0].name, "users_email_key");
        assert!(
            list_sql_unique_constraints(&conn, "users", Some("audit"))
                .unwrap()
                .is_empty()
        );
    }

    // ── sql_table_comment / SqlConnection::table_comment (br-yu3w / fd90.32) ─

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_comment_returns_none_on_sqlite() {
        // SQLite has no native table-comment storage; the trait default
        // returns None even for a real table.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t (x INTEGER);").unwrap();
        let comment = sql_table_comment(&conn, "t", None).unwrap();
        assert!(comment.is_none());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_comment_returns_none_on_sqlite_for_unknown_table() {
        let conn = make_sql_test_conn();
        let comment = sql_table_comment(&conn, "no_such", None).unwrap();
        assert!(comment.is_none());
    }

    #[test]
    fn sql_table_comment_default_impl_returns_none() {
        struct NoIntrospection;
        impl super::SqlConnection for NoIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = NoIntrospection;
        assert!(sql_table_comment(&conn, "anything", None).unwrap().is_none());
    }

    #[test]
    fn sql_table_comment_routes_to_backend_override() {
        struct PgLikeStub;
        impl super::SqlConnection for PgLikeStub {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn table_comment(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Option<String>, IoError> {
                if table == "users" && schema == Some("public") {
                    Ok(Some("Customer accounts table".to_owned()))
                } else {
                    Ok(None)
                }
            }
        }
        let conn = PgLikeStub;
        assert_eq!(
            sql_table_comment(&conn, "users", Some("public")).unwrap().as_deref(),
            Some("Customer accounts table")
        );
        assert!(sql_table_comment(&conn, "users", Some("audit")).unwrap().is_none());
        assert!(sql_table_comment(&conn, "missing", Some("public")).unwrap().is_none());
    }

    #[test]
    fn sql_table_comment_propagates_backend_error() {
        struct BrokenIntrospection;
        impl super::SqlConnection for BrokenIntrospection {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn table_comment(
                &self,
                _table: &str,
                _schema: Option<&str>,
            ) -> Result<Option<String>, IoError> {
                Err(IoError::Sql("permission denied for pg_description".to_owned()))
            }
        }
        let conn = BrokenIntrospection;
        let err = sql_table_comment(&conn, "anything", None)
            .expect_err("backend error must surface");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("permission denied")));
    }

    // ── SqlWriteOptions::chunksize (br-ls9z / fd90.33) ────────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_chunksize_zero_rejected() {
        let conn = make_sql_test_conn();
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![("x", vec![Scalar::Int64(1)])],
        )
        .unwrap();
        let err = write_sql_with_options(
            &frame,
            &conn,
            "t",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: Some(0),
            },
        )
        .expect_err("chunksize=0 must be rejected");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("chunksize")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_chunksize_none_preserves_single_transaction_semantics() {
        // 5 rows, chunksize=None — should round-trip cleanly into one
        // transaction (same as before fd90.33 landed).
        let conn = make_sql_test_conn();
        let frame = fp_frame::DataFrame::from_dict(
            &["id"],
            vec![(
                "id",
                vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(3),
                    Scalar::Int64(4),
                    Scalar::Int64(5),
                ],
            )],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "no_chunk",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: None,
            },
        )
        .unwrap();
        let count =
            super::SqlConnection::query(&conn, "SELECT COUNT(*) FROM no_chunk", &[]).unwrap();
        assert_eq!(count.rows[0][0], Scalar::Int64(5));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn write_sql_single_chunksize_round_trips_all_rows() {
        // 5 rows with chunksize=2: chunks of (2, 2, 1). All rows must
        // round-trip and the table must contain every row regardless
        // of how the chunks committed.
        let conn = make_sql_test_conn();
        let frame = fp_frame::DataFrame::from_dict(
            &["id"],
            vec![(
                "id",
                vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(3),
                    Scalar::Int64(4),
                    Scalar::Int64(5),
                ],
            )],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "chunked",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: Some(2),
            },
        )
        .unwrap();
        let result =
            super::SqlConnection::query(&conn, "SELECT id FROM chunked ORDER BY id", &[]).unwrap();
        let ids: Vec<i64> = result
            .rows
            .iter()
            .map(|r| match &r[0] {
                Scalar::Int64(v) => *v,
                other => unreachable!("unexpected scalar: {other:?}"),
            })
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn write_sql_single_chunksize_dispatches_correct_chunk_counts() {
        // Recording stub verifies the chunk boundaries.
        use std::cell::RefCell;
        struct Recorder {
            row_counts: RefCell<Vec<usize>>,
        }
        impl super::SqlConnection for Recorder {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                self.row_counts.borrow_mut().push(rows.len());
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
        }
        let conn = Recorder {
            row_counts: RefCell::new(vec![]),
        };
        let frame = fp_frame::DataFrame::from_dict(
            &["x"],
            vec![(
                "x",
                vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(3),
                    Scalar::Int64(4),
                    Scalar::Int64(5),
                ],
            )],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "chunked",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Single,
                chunksize: Some(2),
            },
        )
        .unwrap();
        // Single mode: each chunk submits a slice of rows to insert_rows.
        // chunks of size 2, 2, 1 → 3 calls with row counts [2, 2, 1].
        assert_eq!(*conn.row_counts.borrow(), vec![2usize, 2, 1]);
    }

    #[test]
    fn write_sql_multi_chunksize_takes_min_with_param_cap() {
        // Multi mode with max_param_count=10, ncols=2 → param chunk = 5
        // rows. chunksize=3 should win (min(3, 5) = 3).
        // Multi mode flattens each chunk to a single insert_rows call
        // where rows[0].len() = chunk_size * ncols.
        use std::cell::RefCell;
        struct ParamCapRecorder {
            row_counts: RefCell<Vec<usize>>,
        }
        impl super::SqlConnection for ParamCapRecorder {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                // Multi mode passes a single flattened "row" per chunk.
                self.row_counts
                    .borrow_mut()
                    .push(rows.first().map_or(0, std::vec::Vec::len));
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn max_param_count(&self) -> Option<usize> {
                Some(10)
            }
        }
        let conn = ParamCapRecorder {
            row_counts: RefCell::new(vec![]),
        };
        let frame = fp_frame::DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(4),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(20),
                        Scalar::Int64(30),
                        Scalar::Int64(40),
                        Scalar::Int64(50),
                    ],
                ),
            ],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "chunked",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Multi,
                chunksize: Some(3),
            },
        )
        .unwrap();
        // chunksize=3 wins over param cap (5). 5 rows / 3 per chunk = 2
        // chunks (3, 2). Flat scalars per chunk: 3*2=6 then 2*2=4.
        assert_eq!(*conn.row_counts.borrow(), vec![6usize, 4]);
    }

    #[test]
    fn write_sql_multi_chunksize_param_cap_wins_when_smaller() {
        // Param cap = 4 (ncols=2 → 2 rows/chunk). chunksize=10 (loose).
        // Effective chunk = min(10, 2) = 2.
        use std::cell::RefCell;
        struct TightCap {
            row_counts: RefCell<Vec<usize>>,
        }
        impl super::SqlConnection for TightCap {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                self.row_counts
                    .borrow_mut()
                    .push(rows.first().map_or(0, std::vec::Vec::len));
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn max_param_count(&self) -> Option<usize> {
                Some(4)
            }
        }
        let conn = TightCap {
            row_counts: RefCell::new(vec![]),
        };
        let frame = fp_frame::DataFrame::from_dict(
            &["a", "b"],
            vec![
                (
                    "a",
                    vec![
                        Scalar::Int64(1),
                        Scalar::Int64(2),
                        Scalar::Int64(3),
                        Scalar::Int64(4),
                        Scalar::Int64(5),
                    ],
                ),
                (
                    "b",
                    vec![
                        Scalar::Int64(10),
                        Scalar::Int64(20),
                        Scalar::Int64(30),
                        Scalar::Int64(40),
                        Scalar::Int64(50),
                    ],
                ),
            ],
        )
        .unwrap();
        write_sql_with_options(
            &frame,
            &conn,
            "chunked",
            &SqlWriteOptions {
                if_exists: SqlIfExists::Fail,
                index: false,
                index_label: None,
                schema: None,
                dtype: None,
                method: SqlInsertMethod::Multi,
                chunksize: Some(10),
            },
        )
        .unwrap();
        // 5 rows / 2 per chunk = 3 chunks (2, 2, 1). Flat scalars: 4, 4, 2.
        assert_eq!(*conn.row_counts.borrow(), vec![4usize, 4, 2]);
    }

    // ── SqlReadOptions::columns (br-d3e9 / fd90.34) ──────────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_table_with_options_columns_none_selects_all() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE projection_default (a INTEGER, b TEXT, c REAL);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "INSERT INTO projection_default VALUES (1, 'x', 1.5);",
        )
        .unwrap();
        let frame = read_sql_table_with_options(
            &conn,
            "projection_default",
            &SqlReadOptions {
                columns: None,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(frame.column_names(), vec!["a", "b", "c"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_table_with_options_columns_projects_subset() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE projection (id INTEGER, name TEXT, ts TEXT, value REAL);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "INSERT INTO projection VALUES (1, 'a', '2024-01-01', 1.5), \
                                            (2, 'b', '2024-01-02', 2.5);",
        )
        .unwrap();
        let frame = read_sql_table_with_options(
            &conn,
            "projection",
            &SqlReadOptions {
                columns: Some(vec!["id".to_owned(), "name".to_owned()]),
                ..Default::default()
            },
        )
        .unwrap();
        // Only id + name, in that order.
        assert_eq!(frame.column_names(), vec!["id", "name"]);
        assert_eq!(frame.column("id").unwrap().values()[0], Scalar::Int64(1));
        assert_eq!(
            frame.column("name").unwrap().values()[0],
            Scalar::Utf8("a".into())
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_table_with_options_columns_preserves_specified_order() {
        // pandas: pd.read_sql_table(t, con, columns=['c', 'a']) →
        // returns ['c', 'a'] in that exact order, NOT alphabetical.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE ordered_proj (a INT, b INT, c INT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "INSERT INTO ordered_proj VALUES (1, 2, 3);",
        )
        .unwrap();
        let frame = read_sql_table_with_options(
            &conn,
            "ordered_proj",
            &SqlReadOptions {
                columns: Some(vec!["c".to_owned(), "a".to_owned()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(frame.column_names(), vec!["c", "a"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_table_with_options_columns_empty_vec_rejected() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t (x INTEGER);").unwrap();
        let err = read_sql_table_with_options(
            &conn,
            "t",
            &SqlReadOptions {
                columns: Some(vec![]),
                ..Default::default()
            },
        )
        .expect_err("empty columns must be rejected");
        assert!(
            matches!(err, IoError::Sql(msg) if msg.contains("columns must be non-empty"))
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_table_with_options_columns_invalid_name_rejected() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t (x INTEGER);").unwrap();
        let err = read_sql_table_with_options(
            &conn,
            "t",
            &SqlReadOptions {
                columns: Some(vec!["x; DROP TABLE t".to_owned()]),
                ..Default::default()
            },
        )
        .expect_err("invalid column name must be rejected");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("invalid")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_table_with_options_columns_combines_with_parse_dates() {
        // columns + parse_dates: project a subset, then date-coerce.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE events (id INT, ts TEXT, note TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "INSERT INTO events VALUES (1, '2024-01-15', 'launched');",
        )
        .unwrap();
        let frame = read_sql_table_with_options(
            &conn,
            "events",
            &SqlReadOptions {
                columns: Some(vec!["id".to_owned(), "ts".to_owned()]),
                index_col: None,
                parse_dates: Some(vec!["ts".to_owned()]),
                ..Default::default()
            },
        )
        .unwrap();
        // Only id + ts surfaced; ts was reformatted by parse_dates
        // (the project-then-coerce path emits the canonical
        // 'YYYY-MM-DD HH:MM:SS' shape via Scalar::Utf8).
        assert_eq!(frame.column_names(), vec!["id", "ts"]);
        assert_eq!(
            frame.column("ts").unwrap().values()[0],
            Scalar::Utf8("2024-01-15 00:00:00".to_owned())
        );
    }

    // ── SqlColumnSchema::comment (br-cfld / fd90.35) ─────────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_comment_is_none_on_sqlite() {
        // SQLite has no column-comment storage; the rusqlite override
        // must always emit comment=None even when the table is real.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE labeled (id INTEGER, name TEXT);",
        )
        .unwrap();
        let schema = sql_table_schema(&conn, "labeled", None).unwrap().unwrap();
        for col in &schema.columns {
            assert!(
                col.comment.is_none(),
                "SQLite should report no column comment; got {:?} on {}",
                col.comment,
                col.name
            );
        }
    }

    #[test]
    fn sql_table_schema_comment_routes_to_backend_override() {
        // PG-like backend stub returns explicit comment text per column.
        struct PgLikeWithComments;
        impl super::SqlConnection for PgLikeWithComments {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn table_schema(
                &self,
                table: &str,
                _schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                if table == "users" {
                    Ok(Some(SqlTableSchema {
                        table_name: "users".to_owned(),
                        columns: vec![
                            SqlColumnSchema {
                                name: "id".to_owned(),
                                declared_type: Some("BIGINT".to_owned()),
                                nullable: false,
                                default_value: None,
                                primary_key_ordinal: Some(0),
                                comment: Some("Surrogate primary key".to_owned()),
                                autoincrement: false,
                            },
                            SqlColumnSchema {
                                name: "email".to_owned(),
                                declared_type: Some("TEXT".to_owned()),
                                nullable: false,
                                default_value: None,
                                primary_key_ordinal: None,
                                comment: Some("Login identifier".to_owned()),
                                autoincrement: false,
                            },
                            SqlColumnSchema {
                                name: "name".to_owned(),
                                declared_type: Some("TEXT".to_owned()),
                                nullable: true,
                                default_value: None,
                                primary_key_ordinal: None,
                                comment: None,
                                autoincrement: false,
                            },
                        ],
                    }))
                } else {
                    Ok(None)
                }
            }
        }
        let conn = PgLikeWithComments;
        let schema = sql_table_schema(&conn, "users", None).unwrap().unwrap();
        let id = schema.column("id").unwrap();
        assert_eq!(id.comment.as_deref(), Some("Surrogate primary key"));
        let email = schema.column("email").unwrap();
        assert_eq!(email.comment.as_deref(), Some("Login identifier"));
        // Mixed: some columns may have no comment even on PG.
        let name = schema.column("name").unwrap();
        assert!(name.comment.is_none());
    }

    // ── SqlReadOptions::index_col (br-c1h9 / fd90.36) ─────────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_with_options_index_col_none_keeps_range_index() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE keyed (id INTEGER, val INTEGER);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(&conn, "INSERT INTO keyed VALUES (1, 10), (2, 20);")
            .unwrap();
        let frame = read_sql_with_options(
            &conn,
            "SELECT id, val FROM keyed ORDER BY id",
            &SqlReadOptions {
                index_col: None,
                ..Default::default()
            },
        )
        .unwrap();
        // Default RangeIndex: labels 0, 1.
        assert_eq!(frame.index().len(), 2);
        assert_eq!(frame.column_names(), vec!["id", "val"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_with_options_index_col_promotes_named_column() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE keyed (id INTEGER, val INTEGER);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(&conn, "INSERT INTO keyed VALUES (10, 1), (20, 2);")
            .unwrap();
        let frame = read_sql_with_options(
            &conn,
            "SELECT id, val FROM keyed ORDER BY id",
            &SqlReadOptions {
                index_col: Some("id".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
        // 'id' removed from columns, used as index labels.
        assert_eq!(frame.column_names(), vec!["val"]);
        assert_eq!(frame.index().len(), 2);
        // Index labels should be the id values (10, 20).
        let labels: Vec<i64> = frame
            .index()
            .labels()
            .iter()
            .filter_map(|l| match l {
                IndexLabel::Int64(v) => Some(*v),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec![10, 20]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_with_options_index_col_missing_column_errors() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t (a INTEGER);").unwrap();
        super::SqlConnection::execute_batch(&conn, "INSERT INTO t VALUES (1);").unwrap();
        let err = read_sql_with_options(
            &conn,
            "SELECT a FROM t",
            &SqlReadOptions {
                index_col: Some("nonexistent".to_owned()),
                ..Default::default()
            },
        )
        .expect_err("missing index_col must error");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("not present")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_with_options_index_col_empty_string_rejected() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t (a INTEGER);").unwrap();
        let err = read_sql_with_options(
            &conn,
            "SELECT a FROM t",
            &SqlReadOptions {
                index_col: Some(String::new()),
                ..Default::default()
            },
        )
        .expect_err("empty index_col must be rejected");
        assert!(matches!(err, IoError::Sql(msg) if msg.contains("empty string")));
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_table_with_options_index_col_combines_with_columns_projection() {
        // columns + index_col: project ['id', 'val'], promote 'id' to
        // index, leaving only 'val' as a data column.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE wide (id INTEGER, val INTEGER, ts TEXT, note TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "INSERT INTO wide VALUES (5, 100, 't1', 'first');",
        )
        .unwrap();
        let frame = read_sql_table_with_options(
            &conn,
            "wide",
            &SqlReadOptions {
                columns: Some(vec!["id".to_owned(), "val".to_owned()]),
                index_col: Some("id".to_owned()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(frame.column_names(), vec!["val"]);
        let labels: Vec<i64> = frame
            .index()
            .labels()
            .iter()
            .filter_map(|l| match l {
                IndexLabel::Int64(v) => Some(*v),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec![5]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn read_sql_table_with_options_and_index_col_explicit_arg_wins_over_options() {
        // Both options.index_col=Some('a') and explicit index_col=Some('b').
        // The explicit arg must win — options.index_col is silently
        // overridden to avoid double-promotion.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE both (a INTEGER, b INTEGER, c TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "INSERT INTO both VALUES (1, 100, 'x'), (2, 200, 'y');",
        )
        .unwrap();
        let frame = read_sql_table_with_options_and_index_col(
            &conn,
            "both",
            &SqlReadOptions {
                index_col: Some("a".to_owned()),
                ..Default::default()
            },
            Some("b"),
        )
        .unwrap();
        // 'b' is promoted to index, 'a' and 'c' remain as columns.
        assert_eq!(frame.column_names(), vec!["a", "c"]);
        let labels: Vec<i64> = frame
            .index()
            .labels()
            .iter()
            .filter_map(|l| match l {
                IndexLabel::Int64(v) => Some(*v),
                _ => None,
            })
            .collect();
        assert_eq!(labels, vec![100, 200]);
    }

    // ── SqlColumnSchema::autoincrement (br-bkl2 / fd90.37) ────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_autoincrement_detected_on_integer_primary_key() {
        // SQLite rowid-alias rule: INTEGER PRIMARY KEY is an
        // auto-incrementing rowid alias.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE auto_a (id INTEGER PRIMARY KEY, name TEXT);",
        )
        .unwrap();
        let schema = sql_table_schema(&conn, "auto_a", None).unwrap().unwrap();
        let id = schema.column("id").unwrap();
        assert!(
            id.autoincrement,
            "INTEGER PRIMARY KEY must be autoincrement; got {id:?}"
        );
        let name = schema.column("name").unwrap();
        assert!(!name.autoincrement, "non-PK column must not be autoincrement");
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_autoincrement_detected_with_explicit_keyword() {
        // The explicit AUTOINCREMENT keyword affects rowid reuse, not
        // the autoincrement property pandas surfaces.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE auto_b (id INTEGER PRIMARY KEY AUTOINCREMENT, val TEXT);",
        )
        .unwrap();
        let schema = sql_table_schema(&conn, "auto_b", None).unwrap().unwrap();
        let id = schema.column("id").unwrap();
        assert!(id.autoincrement);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_autoincrement_not_set_for_text_primary_key() {
        // TEXT PRIMARY KEY is NOT a rowid alias; not auto-incrementing.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE text_pk (code TEXT PRIMARY KEY, name TEXT);",
        )
        .unwrap();
        let schema = sql_table_schema(&conn, "text_pk", None).unwrap().unwrap();
        let code = schema.column("code").unwrap();
        assert!(
            !code.autoincrement,
            "TEXT PRIMARY KEY is not autoincrement; got {code:?}"
        );
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_autoincrement_not_set_for_non_pk_integer() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE plain_int (val INTEGER, name TEXT);",
        )
        .unwrap();
        let schema = sql_table_schema(&conn, "plain_int", None).unwrap().unwrap();
        let val = schema.column("val").unwrap();
        assert!(!val.autoincrement, "non-PK INTEGER is not autoincrement");
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_autoincrement_not_set_for_composite_pk_integer() {
        // Per fd90.42: SQLite's rowid-alias rule requires the column
        // to be the SOLE primary key. The tightened heuristic counts
        // PK columns first; composite PKs (multiple pk>0 rows) never
        // qualify even when the first column is INTEGER.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE composite_pk ( \
                year INTEGER NOT NULL, \
                month INTEGER NOT NULL, \
                code TEXT NOT NULL, \
                PRIMARY KEY (year, month, code) \
             );",
        )
        .unwrap();
        let schema = sql_table_schema(&conn, "composite_pk", None).unwrap().unwrap();
        let year = schema.column("year").unwrap();
        let month = schema.column("month").unwrap();
        let code = schema.column("code").unwrap();
        // Each part of the composite PK keeps its declaration-order
        // ordinal but NONE of them is autoincrement.
        assert_eq!(year.primary_key_ordinal, Some(0));
        assert_eq!(month.primary_key_ordinal, Some(1));
        assert_eq!(code.primary_key_ordinal, Some(2));
        assert!(!year.autoincrement, "composite PK first col must not be autoincrement");
        assert!(!month.autoincrement);
        assert!(!code.autoincrement);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_table_schema_autoincrement_two_pass_count_distinguishes_single_vs_composite() {
        // Confirm the fix path: single-column INTEGER PRIMARY KEY -> true,
        // composite INTEGER+INTEGER PRIMARY KEY -> false on both.
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE single_int_pk (id INTEGER PRIMARY KEY, label TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE composite_int_pk ( \
                a INTEGER NOT NULL, \
                b INTEGER NOT NULL, \
                PRIMARY KEY (a, b) \
             );",
        )
        .unwrap();

        let single = sql_table_schema(&conn, "single_int_pk", None).unwrap().unwrap();
        assert!(single.column("id").unwrap().autoincrement);

        let composite = sql_table_schema(&conn, "composite_int_pk", None).unwrap().unwrap();
        // Both columns have INTEGER type and pk>0 but neither qualifies.
        assert!(!composite.column("a").unwrap().autoincrement);
        assert!(!composite.column("b").unwrap().autoincrement);
    }

    #[test]
    fn sql_table_schema_autoincrement_routes_to_backend_override() {
        // PG-like backend stub returns explicit autoincrement true for
        // a SERIAL/IDENTITY column.
        struct PgLikeAutoinc;
        impl super::SqlConnection for PgLikeAutoinc {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn table_schema(
                &self,
                table: &str,
                _schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                if table == "users" {
                    Ok(Some(SqlTableSchema {
                        table_name: "users".to_owned(),
                        columns: vec![
                            SqlColumnSchema {
                                name: "id".to_owned(),
                                declared_type: Some("BIGSERIAL".to_owned()),
                                nullable: false,
                                default_value: None,
                                primary_key_ordinal: Some(0),
                                comment: None,
                                autoincrement: true,
                            },
                            SqlColumnSchema {
                                name: "email".to_owned(),
                                declared_type: Some("TEXT".to_owned()),
                                nullable: false,
                                default_value: None,
                                primary_key_ordinal: None,
                                comment: None,
                                autoincrement: false,
                            },
                        ],
                    }))
                } else {
                    Ok(None)
                }
            }
        }
        let conn = PgLikeAutoinc;
        let schema = sql_table_schema(&conn, "users", None).unwrap().unwrap();
        assert!(schema.column("id").unwrap().autoincrement);
        assert!(!schema.column("email").unwrap().autoincrement);
    }

    // ── SqlInspector facade (br-szs9 / fd90.38) ──────────────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_tables_views_schemas() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t1 (x INTEGER);").unwrap();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t2 (y TEXT);").unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE VIEW v1 AS SELECT x FROM t1;",
        )
        .unwrap();

        let inspector = SqlInspector::new(&conn);
        assert_eq!(inspector.tables(None).unwrap(), vec!["t1", "t2"]);
        assert_eq!(inspector.views(None).unwrap(), vec!["v1"]);
        // SQLite has no meaningful schemas → empty vec.
        assert!(inspector.schemas().unwrap().is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_columns_pk_indexes_fks() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE parent (pid INTEGER PRIMARY KEY);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE child ( \
                cid INTEGER PRIMARY KEY, \
                parent_id INTEGER, \
                tag TEXT, \
                FOREIGN KEY (parent_id) REFERENCES parent(pid) \
             );",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE INDEX idx_child_tag ON child(tag);",
        )
        .unwrap();

        let inspector = SqlInspector::new(&conn);

        // columns: 3 columns on 'child', cid + parent_id + tag.
        let schema = inspector.columns("child", None).unwrap().unwrap();
        let names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["cid", "parent_id", "tag"]);

        // primary_key_columns: cid is the sole PK.
        let pk = inspector.primary_key_columns("child", None).unwrap();
        assert_eq!(pk, vec!["cid"]);

        // indexes: only the explicit user index (PK auto-index filtered).
        let indexes = inspector.indexes("child", None).unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].name, "idx_child_tag");

        // foreign_keys: child references parent.
        let fks = inspector.foreign_keys("child", None).unwrap();
        assert_eq!(fks.len(), 1);
        assert_eq!(fks[0].columns, vec!["parent_id"]);
        assert_eq!(fks[0].referenced_table, "parent");
        assert_eq!(fks[0].referenced_columns, vec!["pid"]);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_unique_constraints_and_table_exists() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT UNIQUE);",
        )
        .unwrap();
        let inspector = SqlInspector::new(&conn);
        let uqs = inspector.unique_constraints("users", None).unwrap();
        assert_eq!(uqs.len(), 1);
        assert_eq!(uqs[0].columns, vec!["email"]);
        assert!(inspector.table_exists("users", None).unwrap());
        assert!(!inspector.table_exists("not_there", None).unwrap());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_server_version_and_dialect() {
        let conn = make_sql_test_conn();
        let inspector = SqlInspector::new(&conn);
        let version = inspector.server_version().unwrap().unwrap();
        assert!(version.starts_with("3."));
        assert_eq!(inspector.dialect_name(), "sqlite");
        // SQLite has no documented identifier-length cap.
        assert_eq!(inspector.max_identifier_length(), None);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_table_comment_returns_none_on_sqlite() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE t (x INTEGER);").unwrap();
        let inspector = SqlInspector::new(&conn);
        assert!(inspector.table_comment("t", None).unwrap().is_none());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_via_inspect_helper() {
        // The free-fn `inspect(&conn)` is the one-shot construction helper.
        // Per fd90.46: import lives inside the test so --no-default-features
        // builds don't see it as unused.
        use super::inspect;
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE one (x INTEGER);").unwrap();
        let inspector = inspect(&conn);
        assert_eq!(inspector.tables(None).unwrap(), vec!["one"]);
    }

    #[test]
    fn sql_inspector_routes_schema_arg_to_backend() {
        // Multi-schema backend: verifies SqlInspector forwards the schema
        // arg to every method that accepts one.
        struct PgLikeStub;
        impl super::SqlConnection for PgLikeStub {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn list_tables(&self, schema: Option<&str>) -> Result<Vec<String>, IoError> {
                Ok(match schema {
                    Some("analytics") => vec!["events".to_owned()],
                    _ => vec![],
                })
            }
            fn list_views(&self, schema: Option<&str>) -> Result<Vec<String>, IoError> {
                Ok(match schema {
                    Some("analytics") => vec!["daily".to_owned()],
                    _ => vec![],
                })
            }
            fn list_schemas(&self) -> Result<Vec<String>, IoError> {
                Ok(vec!["public".to_owned(), "analytics".to_owned()])
            }
            fn dialect_name(&self) -> &'static str {
                "postgresql"
            }
            fn max_identifier_length(&self) -> Option<usize> {
                Some(63)
            }
        }
        let conn = PgLikeStub;
        let inspector = SqlInspector::new(&conn);

        assert_eq!(
            inspector.tables(Some("analytics")).unwrap(),
            vec!["events"]
        );
        assert_eq!(inspector.views(Some("analytics")).unwrap(), vec!["daily"]);
        assert!(inspector.tables(Some("audit")).unwrap().is_empty());
        assert_eq!(
            inspector.schemas().unwrap(),
            vec!["public", "analytics"]
        );
        assert_eq!(inspector.dialect_name(), "postgresql");
        assert_eq!(inspector.max_identifier_length(), Some(63));
    }

    // ── SqlInspector::has_column / column (br-ppry / fd90.39) ─────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_has_column_returns_true_for_present_column() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE has_col_tbl (id INTEGER, name TEXT);",
        )
        .unwrap();
        let inspector = SqlInspector::new(&conn);
        assert!(inspector.has_column("has_col_tbl", "id", None).unwrap());
        assert!(inspector.has_column("has_col_tbl", "name", None).unwrap());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_has_column_returns_false_for_missing_column() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE only_id (id INTEGER);",
        )
        .unwrap();
        let inspector = SqlInspector::new(&conn);
        // Table exists but no such column.
        assert!(!inspector.has_column("only_id", "name", None).unwrap());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_has_column_returns_false_for_missing_table() {
        let conn = make_sql_test_conn();
        let inspector = SqlInspector::new(&conn);
        // Table doesn't exist → has_column propagates Ok(false), not error.
        assert!(!inspector.has_column("no_such_tbl", "any_col", None).unwrap());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_column_returns_full_metadata_for_present_column() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE detailed (id INTEGER PRIMARY KEY, status TEXT DEFAULT 'active');",
        )
        .unwrap();
        let inspector = SqlInspector::new(&conn);
        let id = inspector.column("detailed", "id", None).unwrap().unwrap();
        assert_eq!(id.name, "id");
        assert_eq!(id.declared_type.as_deref(), Some("INTEGER"));
        assert_eq!(id.primary_key_ordinal, Some(0));
        // INTEGER PRIMARY KEY → SQLite autoincrement (rowid alias).
        assert!(id.autoincrement);

        let status = inspector.column("detailed", "status", None).unwrap().unwrap();
        assert_eq!(status.declared_type.as_deref(), Some("TEXT"));
        assert!(status.nullable);
        assert_eq!(status.default_value.as_deref(), Some("'active'"));
        assert!(!status.autoincrement);
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_column_returns_none_for_missing_column_or_table() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(&conn, "CREATE TABLE only_x (x INTEGER);")
            .unwrap();
        let inspector = SqlInspector::new(&conn);
        // Existing table, missing column → None.
        assert!(inspector.column("only_x", "missing", None).unwrap().is_none());
        // Missing table → None.
        assert!(inspector.column("no_such", "any", None).unwrap().is_none());
    }

    #[test]
    fn sql_inspector_has_column_routes_schema_arg_to_backend() {
        struct PgLikeColumns;
        impl super::SqlConnection for PgLikeColumns {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn table_schema(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                if table == "users" && schema == Some("public") {
                    Ok(Some(SqlTableSchema {
                        table_name: "users".to_owned(),
                        columns: vec![SqlColumnSchema {
                            name: "id".to_owned(),
                            declared_type: Some("BIGINT".to_owned()),
                            nullable: false,
                            default_value: None,
                            primary_key_ordinal: Some(0),
                            comment: None,
                            autoincrement: true,
                        }],
                    }))
                } else {
                    Ok(None)
                }
            }
        }
        let conn = PgLikeColumns;
        let inspector = SqlInspector::new(&conn);
        assert!(inspector.has_column("users", "id", Some("public")).unwrap());
        assert!(!inspector.has_column("users", "id", Some("audit")).unwrap());
        assert!(!inspector.has_column("users", "missing", Some("public")).unwrap());

        let id_col = inspector
            .column("users", "id", Some("public"))
            .unwrap()
            .unwrap();
        assert_eq!(id_col.declared_type.as_deref(), Some("BIGINT"));
        assert!(id_col.autoincrement);
        assert!(inspector.column("users", "id", Some("audit")).unwrap().is_none());
    }

    // ── SqlInspector::reflect_table (br-76mw / fd90.40) ──────────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_reflect_table_unknown_returns_none() {
        let conn = make_sql_test_conn();
        let inspector = SqlInspector::new(&conn);
        let result = inspector.reflect_table("no_such", None).unwrap();
        assert!(result.is_none());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_reflect_table_bundles_all_metadata() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE parent (pid INTEGER PRIMARY KEY, code TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE bundled ( \
                id INTEGER PRIMARY KEY, \
                parent_id INTEGER, \
                slug TEXT, \
                email TEXT UNIQUE, \
                FOREIGN KEY (parent_id) REFERENCES parent(pid) \
             );",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE INDEX idx_bundled_slug ON bundled(slug);",
        )
        .unwrap();

        let inspector = SqlInspector::new(&conn);
        let bundle = inspector
            .reflect_table("bundled", None)
            .unwrap()
            .expect("table exists");

        assert_eq!(bundle.table_name, "bundled");

        // Columns: id, parent_id, slug, email.
        let names: Vec<&str> = bundle.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["id", "parent_id", "slug", "email"]);
        // INTEGER PRIMARY KEY id is autoincrement.
        let id_col = bundle
            .columns
            .iter()
            .find(|c| c.name == "id")
            .expect("id col");
        assert!(id_col.autoincrement);

        // Primary key.
        assert_eq!(bundle.primary_key_columns, vec!["id"]);

        // Indexes (only the user CREATE INDEX; the UNIQUE constraint
        // index goes via unique_constraints, the PK auto-index is
        // hidden).
        assert_eq!(bundle.indexes.len(), 1);
        assert_eq!(bundle.indexes[0].name, "idx_bundled_slug");

        // Unique constraints (the inline UNIQUE on email).
        assert_eq!(bundle.unique_constraints.len(), 1);
        assert_eq!(bundle.unique_constraints[0].columns, vec!["email"]);

        // Foreign keys (parent_id -> parent.pid).
        assert_eq!(bundle.foreign_keys.len(), 1);
        assert_eq!(bundle.foreign_keys[0].columns, vec!["parent_id"]);
        assert_eq!(bundle.foreign_keys[0].referenced_table, "parent");

        // SQLite has no native column/table comment; comment is None.
        assert!(bundle.comment.is_none());
    }

    #[test]
    fn sql_inspector_reflect_table_routes_to_backend_override() {
        // Multi-schema PG-like backend that returns explicit comment +
        // populated metadata. Verifies all five sub-calls flow through
        // and end up in the bundled struct.
        struct PgLikeBundle;
        impl super::SqlConnection for PgLikeBundle {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn table_schema(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                if table == "users" && schema == Some("public") {
                    Ok(Some(SqlTableSchema {
                        table_name: "users".to_owned(),
                        columns: vec![SqlColumnSchema {
                            name: "id".to_owned(),
                            declared_type: Some("BIGINT".to_owned()),
                            nullable: false,
                            default_value: None,
                            primary_key_ordinal: Some(0),
                            comment: None,
                            autoincrement: true,
                        }],
                    }))
                } else {
                    Ok(None)
                }
            }
            fn primary_key_columns(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Vec<String>, IoError> {
                if table == "users" && schema == Some("public") {
                    Ok(vec!["id".to_owned()])
                } else {
                    Ok(vec![])
                }
            }
            fn list_indexes(
                &self,
                _table: &str,
                _schema: Option<&str>,
            ) -> Result<Vec<SqlIndexSchema>, IoError> {
                Ok(vec![SqlIndexSchema {
                    name: "users_status_idx".to_owned(),
                    columns: vec!["status".to_owned()],
                    unique: false,
                }])
            }
            fn list_foreign_keys(
                &self,
                _table: &str,
                _schema: Option<&str>,
            ) -> Result<Vec<SqlForeignKeySchema>, IoError> {
                Ok(vec![])
            }
            fn list_unique_constraints(
                &self,
                _table: &str,
                _schema: Option<&str>,
            ) -> Result<Vec<SqlUniqueConstraintSchema>, IoError> {
                Ok(vec![SqlUniqueConstraintSchema {
                    name: "users_email_key".to_owned(),
                    columns: vec!["email".to_owned()],
                }])
            }
            fn table_comment(
                &self,
                _table: &str,
                _schema: Option<&str>,
            ) -> Result<Option<String>, IoError> {
                Ok(Some("Customer accounts".to_owned()))
            }
        }
        let conn = PgLikeBundle;
        let inspector = SqlInspector::new(&conn);
        let bundle = inspector
            .reflect_table("users", Some("public"))
            .unwrap()
            .expect("present");
        assert_eq!(bundle.table_name, "users");
        assert_eq!(bundle.columns.len(), 1);
        assert_eq!(bundle.primary_key_columns, vec!["id"]);
        assert_eq!(bundle.indexes.len(), 1);
        assert_eq!(bundle.indexes[0].name, "users_status_idx");
        assert_eq!(bundle.unique_constraints.len(), 1);
        assert_eq!(bundle.foreign_keys.len(), 0);
        assert_eq!(bundle.comment.as_deref(), Some("Customer accounts"));

        // Wrong schema -> None (table_schema returns None).
        assert!(
            inspector
                .reflect_table("users", Some("audit"))
                .unwrap()
                .is_none()
        );
    }

    // Use SqlReflectedTable in a smoke test so the struct's named
    // fields are exercised at the use-site too.
    #[test]
    fn sql_reflected_table_bundle_smoke_test() {
        let bundle = SqlReflectedTable {
            table_name: "t".to_owned(),
            columns: vec![],
            primary_key_columns: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            unique_constraints: vec![],
            comment: None,
        };
        assert_eq!(bundle.table_name, "t");
        assert!(bundle.columns.is_empty());
    }

    // ── SqlReflectedTable accessor methods (br-63ac / fd90.51) ────────────

    #[test]
    fn sql_reflected_table_accessors_find_named_entries() {
        let bundle = SqlReflectedTable {
            table_name: "orders".to_owned(),
            columns: vec![
                SqlColumnSchema {
                    name: "id".to_owned(),
                    declared_type: Some("INTEGER".to_owned()),
                    nullable: false,
                    default_value: None,
                    primary_key_ordinal: Some(0),
                    comment: None,
                    autoincrement: true,
                },
                SqlColumnSchema {
                    name: "user_id".to_owned(),
                    declared_type: Some("INTEGER".to_owned()),
                    nullable: false,
                    default_value: None,
                    primary_key_ordinal: None,
                    comment: None,
                    autoincrement: false,
                },
            ],
            primary_key_columns: vec!["id".to_owned()],
            indexes: vec![SqlIndexSchema {
                name: "idx_orders_user".to_owned(),
                columns: vec!["user_id".to_owned()],
                unique: false,
            }],
            foreign_keys: vec![SqlForeignKeySchema {
                constraint_name: None,
                columns: vec!["user_id".to_owned()],
                referenced_table: "users".to_owned(),
                referenced_columns: vec!["id".to_owned()],
            }],
            unique_constraints: vec![SqlUniqueConstraintSchema {
                name: "uq_orders_id".to_owned(),
                columns: vec!["id".to_owned()],
            }],
            comment: Some("Customer orders".to_owned()),
        };

        // column(name): present + missing.
        let id = bundle.column("id").expect("id column");
        assert_eq!(id.declared_type.as_deref(), Some("INTEGER"));
        assert!(id.autoincrement);
        assert!(bundle.column("missing").is_none());

        // index(name): present + missing.
        let idx = bundle.index("idx_orders_user").expect("idx");
        assert_eq!(idx.columns, vec!["user_id"]);
        assert!(bundle.index("idx_does_not_exist").is_none());

        // unique_constraint(name).
        let uq = bundle.unique_constraint("uq_orders_id").expect("uq");
        assert_eq!(uq.columns, vec!["id"]);
        assert!(bundle.unique_constraint("uq_missing").is_none());

        // foreign_keys_for_column(col): matches the FK touching user_id.
        let fks = bundle.foreign_keys_for_column("user_id");
        assert_eq!(fks.len(), 1);
        assert_eq!(fks[0].referenced_table, "users");
        // Column not part of any FK -> empty.
        assert!(bundle.foreign_keys_for_column("id").is_empty());
        assert!(bundle.foreign_keys_for_column("nonexistent").is_empty());
    }

    #[test]
    fn sql_reflected_table_foreign_keys_for_column_handles_composite_fks() {
        let bundle = SqlReflectedTable {
            table_name: "rolling_fact".to_owned(),
            columns: vec![],
            primary_key_columns: vec![],
            indexes: vec![],
            foreign_keys: vec![SqlForeignKeySchema {
                constraint_name: None,
                columns: vec!["fyear".to_owned(), "fmonth".to_owned()],
                referenced_table: "rolling".to_owned(),
                referenced_columns: vec!["year".to_owned(), "month".to_owned()],
            }],
            unique_constraints: vec![],
            comment: None,
        };
        // Composite FK touches both fyear and fmonth — both should
        // surface the same FK.
        assert_eq!(bundle.foreign_keys_for_column("fyear").len(), 1);
        assert_eq!(bundle.foreign_keys_for_column("fmonth").len(), 1);
        assert!(bundle.foreign_keys_for_column("year").is_empty()); // referenced col, not from col
    }

    #[test]
    fn sql_reflected_table_foreign_keys_for_column_returns_multiple_when_relevant() {
        // Rare but valid: one column participates in two FKs (e.g.
        // same column referenced by separate FKs to two parents).
        let bundle = SqlReflectedTable {
            table_name: "audit".to_owned(),
            columns: vec![],
            primary_key_columns: vec![],
            indexes: vec![],
            foreign_keys: vec![
                SqlForeignKeySchema {
                    constraint_name: Some("fk_audit_a".to_owned()),
                    columns: vec!["entity_id".to_owned()],
                    referenced_table: "users".to_owned(),
                    referenced_columns: vec!["id".to_owned()],
                },
                SqlForeignKeySchema {
                    constraint_name: Some("fk_audit_b".to_owned()),
                    columns: vec!["entity_id".to_owned()],
                    referenced_table: "products".to_owned(),
                    referenced_columns: vec!["id".to_owned()],
                },
            ],
            unique_constraints: vec![],
            comment: None,
        };
        let fks = bundle.foreign_keys_for_column("entity_id");
        assert_eq!(fks.len(), 2);
        // Order preserved.
        assert_eq!(fks[0].constraint_name.as_deref(), Some("fk_audit_a"));
        assert_eq!(fks[1].constraint_name.as_deref(), Some("fk_audit_b"));
    }

    // ── indexes_for_column / unique_constraints_for_column (br-37uy / fd90.52) ─

    #[test]
    fn sql_reflected_table_indexes_for_column_matches_any_position() {
        let bundle = SqlReflectedTable {
            table_name: "rolling".to_owned(),
            columns: vec![],
            primary_key_columns: vec![],
            indexes: vec![
                SqlIndexSchema {
                    name: "idx_rolling_year".to_owned(),
                    columns: vec!["year".to_owned()],
                    unique: false,
                },
                SqlIndexSchema {
                    name: "idx_rolling_y_m_c".to_owned(),
                    columns: vec![
                        "year".to_owned(),
                        "month".to_owned(),
                        "code".to_owned(),
                    ],
                    unique: false,
                },
            ],
            foreign_keys: vec![],
            unique_constraints: vec![],
            comment: None,
        };

        // 'year' appears in both indexes (first in idx_year, first in
        // composite). Returns both.
        let year_idxs = bundle.indexes_for_column("year");
        assert_eq!(year_idxs.len(), 2);

        // 'month' only appears in the composite index, in middle position.
        let month_idxs = bundle.indexes_for_column("month");
        assert_eq!(month_idxs.len(), 1);
        assert_eq!(month_idxs[0].name, "idx_rolling_y_m_c");

        // 'code' appears only in the composite, last position.
        let code_idxs = bundle.indexes_for_column("code");
        assert_eq!(code_idxs.len(), 1);

        // Column not in any index.
        assert!(bundle.indexes_for_column("nonexistent").is_empty());
    }

    #[test]
    fn sql_reflected_table_unique_constraints_for_column_matches_any_position() {
        let bundle = SqlReflectedTable {
            table_name: "events".to_owned(),
            columns: vec![],
            primary_key_columns: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            unique_constraints: vec![
                SqlUniqueConstraintSchema {
                    name: "uq_events_email".to_owned(),
                    columns: vec!["email".to_owned()],
                },
                SqlUniqueConstraintSchema {
                    name: "uq_events_user_event_ts".to_owned(),
                    columns: vec![
                        "user_id".to_owned(),
                        "event_id".to_owned(),
                        "ts".to_owned(),
                    ],
                },
            ],
            comment: None,
        };

        let email_uqs = bundle.unique_constraints_for_column("email");
        assert_eq!(email_uqs.len(), 1);
        assert_eq!(email_uqs[0].name, "uq_events_email");

        // 'event_id' middle position in composite.
        let event_uqs = bundle.unique_constraints_for_column("event_id");
        assert_eq!(event_uqs.len(), 1);
        assert_eq!(event_uqs[0].columns, vec!["user_id", "event_id", "ts"]);

        // 'ts' last position in composite.
        let ts_uqs = bundle.unique_constraints_for_column("ts");
        assert_eq!(ts_uqs.len(), 1);

        assert!(bundle.unique_constraints_for_column("nonexistent").is_empty());
    }

    #[test]
    fn sql_reflected_table_for_column_accessors_return_multiple() {
        // A column can appear in multiple indexes / unique constraints.
        let bundle = SqlReflectedTable {
            table_name: "wide".to_owned(),
            columns: vec![],
            primary_key_columns: vec![],
            indexes: vec![
                SqlIndexSchema {
                    name: "idx_a".to_owned(),
                    columns: vec!["x".to_owned()],
                    unique: false,
                },
                SqlIndexSchema {
                    name: "idx_b".to_owned(),
                    columns: vec!["x".to_owned(), "y".to_owned()],
                    unique: true,
                },
            ],
            foreign_keys: vec![],
            unique_constraints: vec![
                SqlUniqueConstraintSchema {
                    name: "uq_a".to_owned(),
                    columns: vec!["x".to_owned()],
                },
                SqlUniqueConstraintSchema {
                    name: "uq_b".to_owned(),
                    columns: vec!["x".to_owned(), "z".to_owned()],
                },
            ],
            comment: None,
        };

        let idx_for_x = bundle.indexes_for_column("x");
        assert_eq!(idx_for_x.len(), 2);
        assert_eq!(idx_for_x[0].name, "idx_a");
        assert_eq!(idx_for_x[1].name, "idx_b");

        let uq_for_x = bundle.unique_constraints_for_column("x");
        assert_eq!(uq_for_x.len(), 2);
        assert_eq!(uq_for_x[0].name, "uq_a");
        assert_eq!(uq_for_x[1].name, "uq_b");
    }

    // ── SqlInspector::reflect_all_tables (br-jmmo / fd90.53) ─────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_reflect_all_tables_empty_db() {
        let conn = make_sql_test_conn();
        let inspector = SqlInspector::new(&conn);
        let bundles = inspector.reflect_all_tables(None).unwrap();
        assert!(bundles.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_reflect_all_tables_returns_one_bundle_per_table() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE alpha (id INTEGER PRIMARY KEY, name TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE beta (uid INTEGER, label TEXT);",
        )
        .unwrap();
        let inspector = SqlInspector::new(&conn);
        let bundles = inspector.reflect_all_tables(None).unwrap();
        assert_eq!(bundles.len(), 2);
        // Ordered alphabetically by list_tables.
        assert_eq!(bundles[0].table_name, "alpha");
        assert_eq!(bundles[1].table_name, "beta");
        // Each bundle has full metadata.
        assert_eq!(
            bundles[0].columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["id", "name"]
        );
        assert_eq!(bundles[0].primary_key_columns, vec!["id"]);
        assert_eq!(bundles[1].columns.len(), 2);
        assert!(bundles[1].primary_key_columns.is_empty());
    }

    #[test]
    fn sql_inspector_reflect_all_tables_skips_disappearing_tables() {
        // Race-condition stub: list_tables returns ["a", "b"] but
        // table_schema returns None for "b" (simulating a concurrent
        // DROP between list and reflect). reflect_all_tables must
        // skip "b" without erroring.
        struct DisappearingTable;
        impl super::SqlConnection for DisappearingTable {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn list_tables(&self, _schema: Option<&str>) -> Result<Vec<String>, IoError> {
                Ok(vec!["a".to_owned(), "b".to_owned()])
            }
            fn table_schema(
                &self,
                table: &str,
                _schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                if table == "a" {
                    Ok(Some(SqlTableSchema {
                        table_name: "a".to_owned(),
                        columns: vec![SqlColumnSchema {
                            name: "x".to_owned(),
                            declared_type: Some("INTEGER".to_owned()),
                            nullable: true,
                            default_value: None,
                            primary_key_ordinal: None,
                            comment: None,
                            autoincrement: false,
                        }],
                    }))
                } else {
                    // b "disappeared" between list and reflect.
                    Ok(None)
                }
            }
        }
        let conn = DisappearingTable;
        let inspector = SqlInspector::new(&conn);
        let bundles = inspector.reflect_all_tables(None).unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].table_name, "a");
    }

    #[test]
    fn sql_inspector_reflect_all_tables_routes_schema_arg() {
        // Multi-schema stub: list_tables returns different sets per
        // schema; reflect_all_tables must propagate the schema arg.
        struct MultiSchemaReflect;
        impl super::SqlConnection for MultiSchemaReflect {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn list_tables(&self, schema: Option<&str>) -> Result<Vec<String>, IoError> {
                Ok(match schema {
                    Some("analytics") => vec!["events".to_owned()],
                    _ => vec![],
                })
            }
            fn table_schema(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                if table == "events" && schema == Some("analytics") {
                    Ok(Some(SqlTableSchema {
                        table_name: "events".to_owned(),
                        columns: vec![SqlColumnSchema {
                            name: "ts".to_owned(),
                            declared_type: Some("TIMESTAMPTZ".to_owned()),
                            nullable: false,
                            default_value: None,
                            primary_key_ordinal: None,
                            comment: None,
                            autoincrement: false,
                        }],
                    }))
                } else {
                    Ok(None)
                }
            }
        }
        let conn = MultiSchemaReflect;
        let inspector = SqlInspector::new(&conn);
        let bundles = inspector.reflect_all_tables(Some("analytics")).unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].table_name, "events");
        assert_eq!(bundles[0].columns[0].declared_type.as_deref(), Some("TIMESTAMPTZ"));
        // Wrong schema -> empty.
        assert!(inspector.reflect_all_tables(Some("audit")).unwrap().is_empty());
    }

    // ── SqlInspector::reflect_all_views (br-zuqt / fd90.54) ──────────────

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_reflect_all_views_empty_db() {
        let conn = make_sql_test_conn();
        let inspector = SqlInspector::new(&conn);
        let bundles = inspector.reflect_all_views(None).unwrap();
        assert!(bundles.is_empty());
    }

    #[cfg(feature = "sql-sqlite")]
    #[test]
    fn sql_inspector_reflect_all_views_returns_one_bundle_per_view() {
        let conn = make_sql_test_conn();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE TABLE base (id INTEGER, label TEXT);",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE VIEW alpha_view AS SELECT id FROM base;",
        )
        .unwrap();
        super::SqlConnection::execute_batch(
            &conn,
            "CREATE VIEW zebra_view AS SELECT label FROM base;",
        )
        .unwrap();
        let inspector = SqlInspector::new(&conn);
        let bundles = inspector.reflect_all_views(None).unwrap();
        // Tables ARE NOT included — only views.
        assert_eq!(bundles.len(), 2);
        assert_eq!(bundles[0].table_name, "alpha_view");
        assert_eq!(bundles[1].table_name, "zebra_view");
        // Each view's columns are surfaced via PRAGMA table_info.
        assert_eq!(
            bundles[0].columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["id"]
        );
        assert_eq!(
            bundles[1].columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            vec!["label"]
        );
        // Views don't carry constraints — PK/FK/UC/index lists are empty.
        for bundle in &bundles {
            assert!(bundle.primary_key_columns.is_empty());
            assert!(bundle.indexes.is_empty());
            assert!(bundle.foreign_keys.is_empty());
            assert!(bundle.unique_constraints.is_empty());
        }
    }

    #[test]
    fn sql_inspector_reflect_all_views_routes_schema_arg() {
        // Multi-schema stub: list_views returns different sets per schema.
        struct MultiSchemaViewReflect;
        impl super::SqlConnection for MultiSchemaViewReflect {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn supports_schemas(&self) -> bool {
                true
            }
            fn list_views(&self, schema: Option<&str>) -> Result<Vec<String>, IoError> {
                Ok(match schema {
                    Some("reporting") => vec!["weekly_summary".to_owned()],
                    _ => vec![],
                })
            }
            fn table_schema(
                &self,
                table: &str,
                schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                if table == "weekly_summary" && schema == Some("reporting") {
                    Ok(Some(SqlTableSchema {
                        table_name: "weekly_summary".to_owned(),
                        columns: vec![SqlColumnSchema {
                            name: "week".to_owned(),
                            declared_type: Some("DATE".to_owned()),
                            nullable: true,
                            default_value: None,
                            primary_key_ordinal: None,
                            comment: None,
                            autoincrement: false,
                        }],
                    }))
                } else {
                    Ok(None)
                }
            }
        }
        let conn = MultiSchemaViewReflect;
        let inspector = SqlInspector::new(&conn);
        let bundles = inspector.reflect_all_views(Some("reporting")).unwrap();
        assert_eq!(bundles.len(), 1);
        assert_eq!(bundles[0].table_name, "weekly_summary");
        assert_eq!(bundles[0].columns[0].declared_type.as_deref(), Some("DATE"));
        // Wrong schema -> empty.
        assert!(inspector.reflect_all_views(Some("audit")).unwrap().is_empty());
    }

    #[test]
    fn sql_inspector_reflect_table_calls_table_schema_only_once() {
        // Per fd90.43: reflect_table must derive primary_key_columns
        // from the fetched SqlTableSchema rather than dispatching
        // primary_key_columns() (which itself calls table_schema). A
        // recording stub counts table_schema invocations and asserts
        // exactly one round-trip.
        use std::cell::Cell;
        struct CountingTableSchema {
            table_schema_calls: Cell<usize>,
        }
        impl super::SqlConnection for CountingTableSchema {
            fn query(&self, _q: &str, _p: &[Scalar]) -> Result<SqlQueryResult, IoError> {
                Ok(SqlQueryResult {
                    columns: vec![],
                    rows: vec![],
                })
            }
            fn execute_batch(&self, _sql: &str) -> Result<(), IoError> {
                Ok(())
            }
            fn table_exists(&self, _name: &str) -> Result<bool, IoError> {
                Ok(false)
            }
            fn insert_rows(&self, _sql: &str, _rows: &[Vec<Scalar>]) -> Result<(), IoError> {
                Ok(())
            }
            fn dtype_sql(&self, _dtype: DType) -> &'static str {
                "TEXT"
            }
            fn index_dtype_sql(&self, _index: &Index) -> &'static str {
                "TEXT"
            }
            fn table_schema(
                &self,
                _table: &str,
                _schema: Option<&str>,
            ) -> Result<Option<SqlTableSchema>, IoError> {
                self.table_schema_calls
                    .set(self.table_schema_calls.get() + 1);
                Ok(Some(SqlTableSchema {
                    table_name: "x".to_owned(),
                    columns: vec![SqlColumnSchema {
                        name: "id".to_owned(),
                        declared_type: Some("BIGINT".to_owned()),
                        nullable: false,
                        default_value: None,
                        primary_key_ordinal: Some(0),
                        comment: None,
                        autoincrement: true,
                    }],
                }))
            }
        }
        let conn = CountingTableSchema {
            table_schema_calls: Cell::new(0),
        };
        let inspector = SqlInspector::new(&conn);
        let bundle = inspector.reflect_table("x", None).unwrap().unwrap();
        // Exactly one table_schema fetch — primary_key_columns derived
        // from the fetched meta, NOT a second round-trip.
        assert_eq!(conn.table_schema_calls.get(), 1);
        assert_eq!(bundle.primary_key_columns, vec!["id"]);
    }

    // ── SqlReadOptions default coerce_float (br-o0x6 / fd90.41) ──────────

    #[test]
    fn sql_read_options_default_coerce_float_matches_pandas() {
        // Pandas defaults coerce_float=True for read_sql / read_sql_query
        // / read_sql_table. We must match — any bare ::default() call
        // should opt INTO coerce_float, not opt out.
        let opts = SqlReadOptions::default();
        assert!(opts.coerce_float, "default coerce_float must be true (pandas parity)");
        // Sanity: other defaults are the natural empty / None values.
        assert!(opts.params.is_none());
        assert!(opts.parse_dates.is_none());
        assert!(opts.dtype.is_none());
        assert!(opts.schema.is_none());
        assert!(opts.columns.is_none());
        assert!(opts.index_col.is_none());
    }
}
