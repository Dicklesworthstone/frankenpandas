//! IO parity-matrix conformance suite (br-frankenpandas-czmt).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares
//! FrankenPandas IO behavior with live upstream pandas for edge-case inputs:
//! empty CSVs, single rows, missing-heavy values, quoting, bad-line handling,
//! decimal/boolean parsing options, CSV write/reparse behavior, and JSON
//! records serialization with duplicate index labels.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use fp_columnar::Column;
use fp_frame::DataFrame;
use fp_index::{Index, IndexLabel};
use fp_types::{DType, NullKind, Scalar};
use serde_json::Value;

use super::{
    CaseStatus, HarnessConfig, HarnessError, OracleMode, PacketFixture, ResolvedExpected,
    SuiteOptions, capture_live_oracle_expected,
};

fn strict_config() -> HarnessConfig {
    HarnessConfig::default_paths()
}

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(
            ResolvedExpected::Frame(_) | ResolvedExpected::Scalar(_) | ResolvedExpected::Bool(_),
        ) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping IO conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_io_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("IO oracle") {
        return;
    }

    let report = super::run_differential_fixture(
        &cfg,
        &fixture,
        &SuiteOptions {
            packet_filter: None,
            oracle_mode: OracleMode::LiveLegacyPandas,
        },
    )
    .expect("differential report");

    assert_eq!(
        report.status,
        CaseStatus::Pass,
        "pandas IO parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

fn csv_fixture(
    packet_id: &str,
    case_id: &str,
    operation: &str,
    csv_input: &str,
    options: &[(&str, serde_json::Value)],
) -> PacketFixture {
    let mut raw = serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": operation,
        "oracle_source": "live_legacy_pandas",
        "csv_input": csv_input
    });
    let raw_object = raw.as_object_mut().expect("fixture object");
    for (key, value) in options {
        raw_object.insert((*key).to_owned(), value.clone());
    }

    serde_json::from_value(raw).expect("fixture")
}

#[derive(Debug, Clone, Copy)]
struct JsonlEdgeCase<'a> {
    case_id: &'a str,
    input: &'a str,
}

fn pandas_read_jsonl_or_skip(
    config: &HarnessConfig,
    case: JsonlEdgeCase<'_>,
) -> Result<Option<Result<Value, String>>, String> {
    if !config.oracle_root.exists() && !config.allows_live_oracle_fallback() {
        eprintln!(
            "live pandas unavailable; skipping JSONL conformance test {}: legacy oracle root does not exist: {}",
            case.case_id,
            config.oracle_root.display()
        );
        return Ok(None);
    }

    let script = r#"
import importlib
import io
import json
import math
import numbers
import os
import sys

legacy_root = os.path.abspath(sys.argv[1])
allow_system_fallback = sys.argv[2] == "1"
candidate_parent = os.path.dirname(legacy_root)
if os.path.isdir(candidate_parent):
    sys.path.insert(0, candidate_parent)

try:
    import pandas as pd
except Exception:
    if not allow_system_fallback:
        raise
    while candidate_parent in sys.path:
        sys.path.remove(candidate_parent)
    sys.modules.pop("pandas", None)
    pd = importlib.import_module("pandas")

def encode_scalar(value):
    if pd.isna(value):
        return {"kind": "missing"}
    if isinstance(value, bool):
        return {"kind": "bool", "value": value}
    if isinstance(value, numbers.Integral):
        return {"kind": "int64", "value": int(value)}
    if isinstance(value, numbers.Real):
        if math.isnan(value) or math.isinf(value):
            return {"kind": "missing"}
        return {"kind": "float64", "value": float(value)}
    return {"kind": "utf8", "value": str(value)}

payload = sys.stdin.read()
try:
    frame = pd.read_json(io.StringIO(payload), orient="records", lines=True)
except Exception as exc:
    sys.stdout.write(json.dumps({"error": str(exc)}))
    raise SystemExit(0)

rows = []
for row_idx in range(len(frame)):
    rows.append([encode_scalar(frame[column].iloc[row_idx]) for column in frame.columns])

sys.stdout.write(json.dumps({
    "columns": list(frame.columns),
    "dtypes": [str(frame[column].dtype) for column in frame.columns],
    "rows": rows,
}, sort_keys=True))
"#;

    let mut child = Command::new(&config.python_bin)
        .arg("-c")
        .arg(script)
        .arg(&config.oracle_root)
        .arg(if config.allows_live_oracle_fallback() {
            "1"
        } else {
            "0"
        })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("spawn pandas JSONL oracle failed: {err}"))?;

    // Per br-frankenpandas-50b6b0: drain stdin in a worker thread so
    // wait_with_output can drain stdout/stderr concurrently. Synchronous
    // pre-wait write deadlocks if the payload exceeds the OS stdin pipe
    // buffer AND the child emits enough stdout to fill its own buffer.
    let stdin_handle = child
        .stdin
        .take()
        .ok_or_else(|| "pandas JSONL oracle stdin unavailable".to_owned())?;
    let payload: Vec<u8> = case.input.as_bytes().to_vec();
    let stdin_writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut stdin = stdin_handle;
        stdin.write_all(&payload)
    });
    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait for pandas JSONL oracle failed: {err}"))?;
    stdin_writer
        .join()
        .map_err(|_| "pandas JSONL oracle stdin writer panicked".to_owned())?
        .map_err(|err| format!("write pandas JSONL oracle payload failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "pandas JSONL oracle failed for {}: {}",
            case.case_id,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("pandas JSONL oracle emitted invalid JSON: {err}"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Ok(Some(Err(error.to_owned())));
    }
    Ok(Some(Ok(value)))
}

fn dtype_to_pandas_jsonl_name(dtype: DType) -> &'static str {
    match dtype {
        DType::Bool | DType::BoolNullable => "bool",
        DType::Float64 => "float64",
        DType::Int64 | DType::Int64Nullable => "int64",
        DType::Utf8 | DType::Categorical | DType::Sparse | DType::Timedelta64 => "object",
        DType::Datetime64 => "datetime64[ns]",
        DType::Period => "period",
        DType::Interval => "interval",
        DType::Null => "float64",
    }
}

fn scalar_to_jsonl_cell(value: &Scalar) -> Value {
    match value {
        Scalar::Null(_) => serde_json::json!({"kind": "missing"}),
        Scalar::Bool(value) => serde_json::json!({"kind": "bool", "value": value}),
        Scalar::Int64(value) => serde_json::json!({"kind": "int64", "value": value}),
        Scalar::Float64(value) => {
            if value.is_finite() {
                serde_json::json!({"kind": "float64", "value": value})
            } else {
                serde_json::json!({"kind": "missing"})
            }
        }
        Scalar::Utf8(value) => serde_json::json!({"kind": "utf8", "value": value}),
        Scalar::Timedelta64(value) => {
            serde_json::json!({"kind": "utf8", "value": value.to_string()})
        }
        Scalar::Datetime64(ns) => serde_json::json!({"kind": "datetime64", "value": ns}),
        Scalar::Period(p) => serde_json::json!({"kind": "period", "value": p.ordinal}),
        Scalar::Interval(iv) => serde_json::json!({"kind": "interval", "value": iv.to_string()}),
    }
}

fn franken_jsonl_frame_to_oracle_json(frame: &DataFrame) -> Value {
    let columns = frame
        .column_names()
        .iter()
        .map(|name| Value::String((*name).clone()))
        .collect::<Vec<_>>();
    let dtypes = frame
        .column_names()
        .iter()
        .map(|name| {
            Value::String(
                dtype_to_pandas_jsonl_name(frame.column(name).expect("column exists").dtype())
                    .to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let rows = (0..frame.index().len())
        .map(|row_idx| {
            Value::Array(
                frame
                    .column_names()
                    .iter()
                    .map(|name| {
                        frame
                            .column(name)
                            .and_then(|column| column.value(row_idx))
                            .map_or_else(
                                || serde_json::json!({"kind": "missing"}),
                                scalar_to_jsonl_cell,
                            )
                    })
                    .collect(),
            )
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "columns": columns,
        "dtypes": dtypes,
        "rows": rows,
    })
}

fn assert_jsonl_read_matches_pandas(case: JsonlEdgeCase<'_>) {
    let config = HarnessConfig::default_paths();
    let Some(expected) = pandas_read_jsonl_or_skip(&config, case).expect("pandas JSONL oracle")
    else {
        return;
    };

    match expected {
        Ok(expected_frame) => {
            let actual = fp_io::read_jsonl_str(case.input).expect("frankenpandas JSONL read");
            assert_eq!(
                franken_jsonl_frame_to_oracle_json(&actual),
                expected_frame,
                "pandas JSONL read parity drift for {}",
                case.case_id
            );
        }
        Err(expected_error) => {
            let actual = fp_io::read_jsonl_str(case.input);
            assert!(
                actual.is_err(),
                "expected pandas-compatible JSONL error for {} containing {expected_error:?}",
                case.case_id
            );
        }
    }
}

fn nullable_int64_frame(values: Vec<Scalar>) -> DataFrame {
    let row_count = values.len();
    let mut columns = std::collections::BTreeMap::new();
    columns.insert(
        "a".to_owned(),
        Column::new(DType::Int64, values).expect("nullable Int64 column"),
    );
    let labels = (0..row_count)
        .map(|idx| IndexLabel::Int64(idx as i64))
        .collect::<Vec<_>>();
    DataFrame::new_with_column_order(Index::new(labels), columns, vec!["a".to_owned()])
        .expect("nullable Int64 frame")
}

fn assert_nullable_int64_parquet_round_trip(values: Vec<Scalar>) {
    let frame = nullable_int64_frame(values);
    let encoded = fp_io::write_parquet_bytes(&frame).expect("write parquet");
    let decoded = fp_io::read_parquet_bytes(&encoded).expect("read parquet");
    let column = decoded.column("a").expect("decoded column");

    assert_eq!(column.dtype(), DType::Int64);
    assert_eq!(
        column.values(),
        frame.column("a").expect("source column").values()
    );
}

#[test]
fn conformance_io_read_csv_empty_header_only() {
    let fixture = csv_fixture(
        "FP-CONF-IO-001",
        "io_read_csv_empty_header_only",
        "csv_read_frame",
        "a,b\n",
        &[],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_single_row_mixed_dtypes() {
    let fixture = csv_fixture(
        "FP-CONF-IO-002",
        "io_read_csv_single_row_mixed_dtypes",
        "csv_read_frame",
        "id,name,score\n1,Ada,3.5\n",
        &[],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_missing_heavy_values() {
    let fixture = csv_fixture(
        "FP-CONF-IO-003",
        "io_read_csv_missing_heavy_values",
        "csv_read_frame",
        "a,b,c\n,NA,NaN\n,,x\n",
        &[],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_missing_heavy_numeric_column() {
    let fixture = csv_fixture(
        "FP-CONF-IO-011",
        "io_read_csv_missing_heavy_numeric_column",
        "csv_read_frame",
        "a,b,c\n,NA,NaN\n1,,x\n",
        &[],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_quoted_commas_and_newlines() {
    let fixture = csv_fixture(
        "FP-CONF-IO-004",
        "io_read_csv_quoted_commas_and_newlines",
        "csv_read_frame",
        "name,note\nAda,\"hello, world\"\nBob,\"line1\nline2\"\n",
        &[],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_on_bad_lines_skip() {
    let fixture = csv_fixture(
        "FP-CONF-IO-005",
        "io_read_csv_on_bad_lines_skip",
        "csv_read_frame",
        "a,b\n1,2\n3,4,5\n6,7\n",
        &[("csv_on_bad_lines", serde_json::json!("skip"))],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_true_false_values() {
    let fixture = csv_fixture(
        "FP-CONF-IO-006",
        "io_read_csv_true_false_values",
        "csv_read_frame",
        "flag\nyes\nno\n",
        &[
            ("csv_true_values", serde_json::json!(["yes"])),
            ("csv_false_values", serde_json::json!(["no"])),
        ],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_numeric_true_false_values_remain_ints() {
    let fixture = csv_fixture(
        "FP-CONF-IO-012",
        "io_read_csv_numeric_true_false_values_remain_ints",
        "csv_read_frame",
        "flag\n1\n0\n",
        &[
            ("csv_true_values", serde_json::json!(["1"])),
            ("csv_false_values", serde_json::json!(["0"])),
        ],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_decimal_comma_quoted_values() {
    let fixture = csv_fixture(
        "FP-CONF-IO-013",
        "io_read_csv_decimal_comma_quoted_values",
        "csv_read_frame",
        "price\n\"1,50\"\n\"3,75\"\n",
        &[("csv_decimal", serde_json::json!(","))],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_parse_dates_iso_dates() {
    let fixture = csv_fixture(
        "FP-CONF-IO-007",
        "io_read_csv_parse_dates_iso_dates",
        "csv_read_frame",
        "ts,value\n2024-01-15,1\n2024-01-16,2\n",
        &[("csv_parse_dates", serde_json::json!(["ts"]))],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_parse_dates_combined_columns() {
    let fixture = csv_fixture(
        "FP-CONF-IO-008",
        "io_read_csv_parse_dates_combined_columns",
        "csv_read_frame",
        "date,time,value\n2024-01-15,10:30:00,1\n2024-01-16,11:45:30,2\n",
        &[(
            "csv_parse_date_combinations",
            serde_json::json!([["date", "time"]]),
        )],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_read_csv_parse_dates_mixed_inferred_format_stays_object() {
    let fixture = csv_fixture(
        "FP-CONF-IO-014",
        "io_read_csv_parse_dates_mixed_inferred_format_stays_object",
        "csv_read_frame",
        "ts,value\n2024-01-15 10:30:00,1\n2024-01-15T10:30:00Z,2\n",
        &[("csv_parse_dates", serde_json::json!(["ts"]))],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_csv_round_trip_quoted_missing_values() {
    let fixture = csv_fixture(
        "FP-CONF-IO-009",
        "io_csv_round_trip_quoted_missing_values",
        "csv_round_trip",
        "name,note,value\nAda,\"hello, world\",1\nBob,,\n",
        &[],
    );
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_dataframe_to_json_records_ignores_duplicate_index() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-IO-010",
        "case_id": "io_dataframe_to_json_records_ignores_duplicate_index",
        "mode": "strict",
        "operation": "dataframe_to_json_records",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "row" },
                { "kind": "utf8", "value": "row" }
            ],
            "columns": {
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "utf8", "value": "y" }
                ],
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ],
                "flag": [
                    { "kind": "bool", "value": true },
                    { "kind": "bool", "value": false }
                ]
            },
            "column_order": ["b", "a", "flag"]
        }
    }))
    .expect("fixture");
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_dataframe_to_json_records_nullable_numeric_duplicate_index() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-IO-015",
        "case_id": "io_dataframe_to_json_records_nullable_numeric_duplicate_index",
        "mode": "strict",
        "operation": "dataframe_to_json_records",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "row" },
                { "kind": "utf8", "value": "row" }
            ],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "null", "value": "null" }
                ]
            },
            "column_order": ["a"]
        }
    }))
    .expect("fixture");
    check_io_fixture(fixture);
}

#[test]
fn conformance_io_jsonl_read_blank_lines_and_trailing_newline() {
    assert_jsonl_read_matches_pandas(JsonlEdgeCase {
        case_id: "io_jsonl_read_blank_lines_and_trailing_newline",
        input: "{\"a\":1}\n\n{\"a\":2}\n",
    });
}

#[test]
fn conformance_io_jsonl_read_mixed_schema_promotes_missing_numeric_columns() {
    assert_jsonl_read_matches_pandas(JsonlEdgeCase {
        case_id: "io_jsonl_read_mixed_schema_promotes_missing_numeric_columns",
        input: "{\"a\":1,\"b\":2}\n{\"a\":3,\"c\":4}\n",
    });
}

#[test]
fn conformance_io_jsonl_read_nan_and_none_records() {
    assert_jsonl_read_matches_pandas(JsonlEdgeCase {
        case_id: "io_jsonl_read_nan_and_none_records",
        input: "{\"a\":1,\"b\":null}\n{\"a\":NaN,\"b\":\"x\"}\n",
    });
}

#[test]
fn conformance_io_jsonl_read_empty_file() {
    assert_jsonl_read_matches_pandas(JsonlEdgeCase {
        case_id: "io_jsonl_read_empty_file",
        input: "",
    });
}

#[test]
fn conformance_io_jsonl_read_single_record_without_trailing_newline() {
    assert_jsonl_read_matches_pandas(JsonlEdgeCase {
        case_id: "io_jsonl_read_single_record_without_trailing_newline",
        input: "{\"a\":1}",
    });
}

#[test]
fn conformance_io_jsonl_read_utf8_bom_errors_like_pandas() {
    assert_jsonl_read_matches_pandas(JsonlEdgeCase {
        case_id: "io_jsonl_read_utf8_bom_errors_like_pandas",
        input: "\u{feff}{\"a\":1}\n",
    });
}

#[test]
fn conformance_io_parquet_nullable_int64_round_trip_preserves_dtype_and_nulls() {
    assert_nullable_int64_parquet_round_trip(vec![
        Scalar::Int64(1),
        Scalar::Null(NullKind::Null),
        Scalar::Int64(3),
    ]);
}

#[test]
fn conformance_io_parquet_all_null_int64_round_trip_preserves_dtype() {
    assert_nullable_int64_parquet_round_trip(vec![
        Scalar::Null(NullKind::Null),
        Scalar::Null(NullKind::Null),
    ]);
}
