#![forbid(unsafe_code)]

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
use fp_types::{DType, NullKind, Scalar, Timedelta};
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
/// See: https://pandas.pydata.org/docs/reference/api/pandas.read_csv.html
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
        let parsed: serde_json::Value = serde_json::from_str(trimmed)?;
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
        out_columns.insert(name.clone(), Column::from_values(values)?);
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
        let col = Column::from_values(values)?;
        columns.insert(name.clone(), col);
        col_order.push(name);
    }

    let labels: Vec<IndexLabel> = (0..n_rows).map(|i| IndexLabel::Int64(i as i64)).collect();
    let index = Index::new(labels);

    let frame = DataFrame::new_with_column_order(index, columns, col_order)?;
    promote_synthetic_row_multiindex_if_present(&frame)
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

/// Options for reading SQL query results into a DataFrame.
#[derive(Debug, Clone, Default)]
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
}

/// Backend-agnostic in-memory representation of a SQL query result.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Scalar>>,
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

fn escape_sql_ident(name: &str) -> Result<String, IoError> {
    if name.contains('\0') {
        return Err(IoError::Sql("invalid SQL identifier: NUL byte".to_owned()));
    }
    Ok(name.replace('"', "\"\""))
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
    // Validate table name to prevent SQL injection (only allow alphanumeric + underscore, non-empty).
    if table_name.is_empty() || !table_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(IoError::Sql(format!(
            "invalid table name: '{table_name}' (must be non-empty, only alphanumeric and underscore allowed)"
        )));
    }
    let escaped_table = escape_sql_ident(table_name)?;
    read_sql(conn, &format!("SELECT * FROM \"{escaped_table}\""))
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
    if table_name.is_empty() || !table_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(IoError::Sql(format!(
            "invalid table name: '{table_name}' (must be non-empty, only alphanumeric and underscore allowed)"
        )));
    }
    if columns.is_empty() {
        return Err(IoError::Sql(
            "read_sql_table_columns: columns must be non-empty".to_owned(),
        ));
    }
    for name in columns {
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err(IoError::Sql(format!(
                "invalid column name: '{name}' (must be non-empty, only alphanumeric and underscore allowed)"
            )));
        }
    }

    let escaped_table = escape_sql_ident(table_name)?;
    let projection: Vec<String> = columns
        .iter()
        .map(|c| escape_sql_ident(c).map(|q| format!("\"{q}\"")))
        .collect::<Result<_, _>>()?;
    let query = format!(
        "SELECT {} FROM \"{}\"",
        projection.join(", "),
        escaped_table
    );
    read_sql(conn, &query)
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
    let escaped_table = escape_sql_ident(table_name)?;
    let mut sql_col_names =
        Vec::with_capacity(col_names.len() + usize::from(index_label.is_some()));
    if let Some(ref label) = index_label {
        sql_col_names.push(label.clone());
    }
    sql_col_names.extend(col_names.iter().cloned());
    let escaped_cols: Vec<String> = sql_col_names
        .iter()
        .map(|name| escape_sql_ident(name))
        .collect::<Result<Vec<_>, _>>()?;

    // Handle if_exists policy.
    match options.if_exists {
        SqlIfExists::Fail => {
            let exists = conn.table_exists(table_name)?;
            if exists {
                return Err(IoError::Sql(format!("table '{table_name}' already exists")));
            }
        }
        SqlIfExists::Replace => {
            conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{escaped_table}\""))?;
        }
        SqlIfExists::Append => {
            // Table may or may not exist; CREATE TABLE IF NOT EXISTS handles both.
        }
    }

    // Build CREATE TABLE statement.
    let mut col_defs = Vec::with_capacity(sql_col_names.len());
    if let Some(ref label) = index_label {
        col_defs.push(format!(
            "\"{}\" {}",
            escape_sql_ident(label)?,
            conn.index_dtype_sql(frame.index())
        ));
    }
    col_defs.extend(
        col_names
            .iter()
            .map(|name| {
                let escaped = escape_sql_ident(name)?;
                let dt = frame.column(name).map_or(DType::Utf8, |c| c.dtype());
                Ok(format!("\"{}\" {}", escaped, conn.dtype_sql(dt)))
            })
            .collect::<Result<Vec<_>, IoError>>()?,
    );

    let create_sql = format!(
        "CREATE TABLE IF NOT EXISTS \"{}\" ({})",
        escaped_table,
        col_defs.join(", ")
    );
    conn.execute_batch(&create_sql)?;

    let placeholders: Vec<String> = (1..=sql_col_names.len())
        .map(|ordinal| conn.parameter_marker(ordinal))
        .collect();
    let insert_sql = format!(
        "INSERT INTO \"{}\" ({}) VALUES ({})",
        escaped_table,
        escaped_cols
            .iter()
            .map(|n| format!("\"{}\"", n))
            .collect::<Vec<_>>()
            .join(", "),
        placeholders.join(", ")
    );

    let nrows = frame.index().len();
    let mut rows = Vec::with_capacity(nrows);
    for row_idx in 0..nrows {
        let mut row = Vec::with_capacity(sql_col_names.len());
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

    conn.insert_rows(&insert_sql, &rows)?;

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

    use super::{
        SqlIfExists, SqlReadOptions, SqlWriteOptions, read_sql, read_sql_table,
        read_sql_table_columns, read_sql_table_with_index_col, read_sql_with_index_col,
        read_sql_with_options, write_sql, write_sql_with_options,
    };

    fn make_sql_test_conn() -> rusqlite::Connection {
        rusqlite::Connection::open_in_memory().expect("in-memory sqlite")
    }

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

    #[test]
    fn sql_read_with_index_col_unknown_column_errors() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "missing_tbl", SqlIfExists::Fail).expect("write");
        let err = read_sql_table_with_index_col(&conn, "missing_tbl", Some("nope")).unwrap_err();
        assert!(matches!(err, crate::IoError::Sql(_)));
    }

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

    #[test]
    fn sql_read_table_columns_rejects_empty_columns() {
        let conn = make_sql_test_conn();
        let err = read_sql_table_columns(&conn, "any_tbl", &[]).unwrap_err();
        assert!(matches!(err, crate::IoError::Sql(_)));
    }

    #[test]
    fn sql_read_table_columns_rejects_invalid_column_name() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "valid_tbl", SqlIfExists::Fail).expect("write");
        let err = read_sql_table_columns(&conn, "valid_tbl", &["ints; DROP TABLE valid_tbl"])
            .unwrap_err();
        assert!(matches!(err, crate::IoError::Sql(_)));
    }

    #[test]
    fn sql_read_table_columns_rejects_invalid_table_name() {
        let conn = make_sql_test_conn();
        let err = read_sql_table_columns(&conn, "bad table", &["ints"]).unwrap_err();
        assert!(matches!(err, crate::IoError::Sql(_)));
    }

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
            },
        )
        .expect("write with unnamed index");

        let raw = read_sql_table(&conn, "default_index_tbl").expect("read raw table");
        assert!(raw.column("index").is_some());
        assert_eq!(raw.column("index").unwrap().values()[0], Scalar::Int64(0));
        assert_eq!(raw.column("index").unwrap().values()[2], Scalar::Int64(2));
    }

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
            },
        )
        .expect_err("missing parse_dates column should error");

        assert!(
            matches!(err, IoError::MissingParseDateColumns(missing) if missing == vec!["ts".to_owned()])
        );
    }

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
            },
        )
        .expect_err("wrong arity should error");

        assert!(matches!(err, IoError::Sql(msg) if msg.contains("parameter")));
    }

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

        let default_frame =
            read_sql(&conn, "SELECT amount FROM payments ORDER BY id").expect("default read");
        assert_eq!(
            default_frame.column("amount").unwrap().values(),
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

    #[test]
    fn sql_duplicate_column_names_error() {
        let conn = make_sql_test_conn();
        let err = read_sql(&conn, "SELECT 1 as dup, 2 as dup");
        assert!(matches!(err, Err(IoError::DuplicateColumnName(name)) if name == "dup"));
    }

    #[test]
    fn sql_if_exists_fail() {
        let frame = make_test_dataframe();
        let conn = make_sql_test_conn();
        write_sql(&frame, &conn, "tbl", SqlIfExists::Fail).unwrap();

        let err = write_sql(&frame, &conn, "tbl", SqlIfExists::Fail);
        assert!(err.is_err());
        assert!(matches!(&err.unwrap_err(), IoError::Sql(msg) if msg.contains("already exists")),);
    }

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

    #[test]
    fn sql_invalid_table_name_rejected() {
        let conn = make_sql_test_conn();
        let err = read_sql_table(&conn, "Robert'; DROP TABLE students; --");
        assert!(err.is_err());
        assert!(
            matches!(&err.unwrap_err(), IoError::Sql(msg) if msg.contains("invalid table name")),
        );
    }

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

    #[test]
    fn sql_empty_result() {
        let conn = make_sql_test_conn();
        conn.execute_batch("CREATE TABLE empty (x INTEGER, y TEXT)")
            .unwrap();
        let frame = read_sql_table(&conn, "empty").unwrap();
        assert_eq!(frame.index().len(), 0);
        assert_eq!(frame.column_names().len(), 2);
    }

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
        assert_eq!(frame.column("b").unwrap().values()[0], Scalar::Int64(2));
        assert!(frame.column("c").unwrap().values()[0].is_missing());
        // Row 1: a=3, b=null, c=4.
        assert_eq!(frame.column("a").unwrap().values()[1], Scalar::Int64(3));
        assert!(frame.column("b").unwrap().values()[1].is_missing());
        assert_eq!(frame.column("c").unwrap().values()[1], Scalar::Int64(4));
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
}
