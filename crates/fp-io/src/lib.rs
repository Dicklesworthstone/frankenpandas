#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::Path;

use csv::{ReaderBuilder, WriterBuilder};
use fp_columnar::{Column, ColumnError};
use fp_frame::{DataFrame, FrameError};
use fp_index::{Index, IndexLabel};
use fp_types::{NullKind, Scalar};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("csv input has no headers")]
    MissingHeaders,
    #[error("csv index column '{0}' not found in headers")]
    MissingIndexColumn(String),
    #[error("json format error: {0}")]
    JsonFormat(String),
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonOrient {
    Records,
    Columns,
    Index,
    Split,
}

#[derive(Debug, Clone)]
pub struct CsvReadOptions {
    pub delimiter: u8,
    pub has_headers: bool,
    pub na_values: Vec<String>,
    pub index_col: Option<String>,
}

impl Default for CsvReadOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            has_headers: true,
            na_values: Vec::new(),
            index_col: None,
        }
    }
}

pub fn read_csv_str(input: &str) -> Result<DataFrame, IoError> {
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_reader(input.as_bytes());

    let headers = reader.headers().cloned().map_err(IoError::from)?;

    if headers.is_empty() {
        return Err(IoError::MissingHeaders);
    }

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
    for (idx, values) in columns.into_iter().enumerate() {
        let name = headers.get(idx).unwrap_or_default().to_owned();
        out_columns.insert(name, Column::from_values(values)?);
    }

    let index = Index::from_i64((0..row_count).collect());
    Ok(DataFrame::new(index, out_columns)?)
}

pub fn write_csv_string(frame: &DataFrame) -> Result<String, IoError> {
    let mut writer = WriterBuilder::new().from_writer(Vec::new());

    let headers = frame.columns().keys().cloned().collect::<Vec<_>>();
    writer.write_record(&headers)?;

    for row_idx in 0..frame.index().len() {
        let row = headers
            .iter()
            .map(|name| {
                frame
                    .column(name)
                    .and_then(|column| column.value(row_idx))
                    .map_or_else(String::new, scalar_to_csv)
            })
            .collect::<Vec<_>>();
        writer.write_record(&row)?;
    }

    let bytes = writer.into_inner().map_err(|err| err.into_error())?;
    Ok(String::from_utf8(bytes)?)
}

fn parse_scalar(field: &str) -> Scalar {
    let trimmed = field.trim();
    if trimmed.is_empty() {
        return Scalar::Null(NullKind::Null);
    }

    if let Ok(value) = trimmed.parse::<i64>() {
        return Scalar::Int64(value);
    }
    if let Ok(value) = trimmed.parse::<f64>() {
        return Scalar::Float64(value);
    }
    if let Ok(value) = trimmed.parse::<bool>() {
        return Scalar::Bool(value);
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
    }
}

fn parse_scalar_with_na(field: &str, na_values: &[String]) -> Scalar {
    let trimmed = field.trim();
    if trimmed.is_empty() || na_values.iter().any(|na| na == trimmed) {
        return Scalar::Null(NullKind::Null);
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return Scalar::Int64(value);
    }
    if let Ok(value) = trimmed.parse::<f64>() {
        return Scalar::Float64(value);
    }
    if let Ok(value) = trimmed.parse::<bool>() {
        return Scalar::Bool(value);
    }
    Scalar::Utf8(trimmed.to_owned())
}

// ── CSV with options ───────────────────────────────────────────────────

pub fn read_csv_with_options(input: &str, options: &CsvReadOptions) -> Result<DataFrame, IoError> {
    let mut reader = ReaderBuilder::new()
        .has_headers(options.has_headers)
        .delimiter(options.delimiter)
        .from_reader(input.as_bytes());

    let headers = reader.headers().cloned().map_err(IoError::from)?;
    if headers.is_empty() {
        return Err(IoError::MissingHeaders);
    }

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
            col.push(parse_scalar_with_na(field, &options.na_values));
        }
        row_count += 1;
    }

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
                other => fp_index::IndexLabel::Utf8(format!("{other:?}")),
            })
            .collect();
        let index = Index::new(index_labels);

        let mut out_columns = BTreeMap::new();
        let mut col_idx = 0;
        for (orig_idx, _) in headers.iter().enumerate() {
            if orig_idx == idx_pos {
                continue;
            }
            let name = headers.get(orig_idx).unwrap_or_default().to_owned();
            out_columns.insert(name, Column::from_values(columns[col_idx].clone())?);
            col_idx += 1;
        }
        Ok(DataFrame::new(index, out_columns)?)
    } else {
        let mut out_columns = BTreeMap::new();
        for (idx, values) in columns.into_iter().enumerate() {
            let name = headers.get(idx).unwrap_or_default().to_owned();
            out_columns.insert(name, Column::from_values(values)?);
        }
        let index = Index::from_i64((0..row_count).collect());
        Ok(DataFrame::new(index, out_columns)?)
    }
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
    }
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

fn index_label_to_json(label: &IndexLabel) -> serde_json::Value {
    match label {
        IndexLabel::Int64(v) => serde_json::json!(*v),
        IndexLabel::Utf8(v) => serde_json::Value::String(v.clone()),
    }
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

            // Collect column names from first record
            let first = arr[0]
                .as_object()
                .ok_or_else(|| IoError::JsonFormat("records must be objects".into()))?;
            let col_names: Vec<String> = first.keys().cloned().collect();

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
                        .unwrap()
                        .push(json_value_to_scalar(val));
                }
            }

            let row_count = arr.len() as i64;
            let mut out = BTreeMap::new();
            for (name, vals) in columns {
                out.insert(name, Column::from_values(vals)?);
            }
            let index = Index::from_i64((0..row_count).collect());
            Ok(DataFrame::new(index, out)?)
        }
        JsonOrient::Columns => {
            let obj = parsed
                .as_object()
                .ok_or_else(|| IoError::JsonFormat("expected object for columns orient".into()))?;

            let mut out = BTreeMap::new();
            let mut row_count = 0_i64;

            for (col_name, col_data) in obj {
                let col_obj = col_data
                    .as_object()
                    .ok_or_else(|| IoError::JsonFormat("column data must be {idx: val}".into()))?;
                let mut values: Vec<(i64, Scalar)> = Vec::with_capacity(col_obj.len());
                for (idx_str, val) in col_obj {
                    let idx: i64 = idx_str.parse().map_err(|_| {
                        IoError::JsonFormat(format!("non-integer index: {idx_str}"))
                    })?;
                    values.push((idx, json_value_to_scalar(val)));
                }
                values.sort_by_key(|(idx, _)| *idx);
                let scalars: Vec<Scalar> = values.into_iter().map(|(_, v)| v).collect();
                if scalars.len() as i64 > row_count {
                    row_count = scalars.len() as i64;
                }
                out.insert(col_name.clone(), Column::from_values(scalars)?);
            }

            let index = Index::from_i64((0..row_count).collect());
            Ok(DataFrame::new(index, out)?)
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
            Ok(DataFrame::new(Index::new(index_labels), out)?)
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
                .map(|v| v.as_str().unwrap_or_default().to_owned())
                .collect();

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

            for row in data {
                let arr = row
                    .as_array()
                    .ok_or_else(|| IoError::JsonFormat("each data row must be an array".into()))?;
                for (i, name) in col_names.iter().enumerate() {
                    let val = arr.get(i).unwrap_or(&serde_json::Value::Null);
                    columns
                        .get_mut(name)
                        .unwrap()
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
            Ok(DataFrame::new(index, out)?)
        }
    }
}

pub fn write_json_string(frame: &DataFrame, orient: JsonOrient) -> Result<String, IoError> {
    let headers: Vec<String> = frame.columns().keys().cloned().collect();
    let row_count = frame.index().len();

    match orient {
        JsonOrient::Records => {
            let mut records = Vec::with_capacity(row_count);
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
                records.push(serde_json::Value::Object(obj));
            }
            Ok(serde_json::to_string(&records)?)
        }
        JsonOrient::Columns => {
            let mut outer = serde_json::Map::new();
            for name in &headers {
                let mut col_obj = serde_json::Map::new();
                if let Some(col) = frame.column(name) {
                    for (i, val) in col.values().iter().enumerate() {
                        col_obj.insert(i.to_string(), scalar_to_json(val));
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use fp_columnar::Column;
    use fp_frame::DataFrame;
    use fp_index::{Index, IndexLabel};
    use fp_types::{NullKind, Scalar};

    use super::{read_csv_str, write_csv_string};

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
        CsvReadOptions, IoError, JsonOrient, read_csv_with_options, read_json_str,
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
    fn json_columns_read_write_roundtrip() {
        let input = r#"{"name":{"0":"Alice","1":"Bob"},"age":{"0":30,"1":25}}"#;
        let frame = read_json_str(input, JsonOrient::Columns).expect("read json columns");
        assert_eq!(frame.index().len(), 2);

        let output = write_json_string(&frame, JsonOrient::Columns).expect("write");
        let frame2 = read_json_str(&output, JsonOrient::Columns).expect("re-read");
        assert_eq!(frame2.index().len(), 2);
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
    fn json_records_with_nulls() {
        let input = r#"[{"a":1,"b":null},{"a":null,"b":"hello"}]"#;
        let frame = read_json_str(input, JsonOrient::Records).expect("parse");
        assert!(frame.column("a").unwrap().values()[1].is_missing());
        assert!(frame.column("b").unwrap().values()[0].is_missing());
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
    fn json_records_incompatible_types_errors() {
        let input = r#"[{"v":1},{"v":"text"}]"#;
        assert!(read_json_str(input, JsonOrient::Records).is_err());
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
        assert_eq!(frame.column("val").unwrap().values()[1], Scalar::Int64(2));

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
}
