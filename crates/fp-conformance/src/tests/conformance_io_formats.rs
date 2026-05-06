//! pandas.io.formats parity-matrix conformance suite (frankenpandas-i49b).
//!
//! Per /testing-conformance-harnesses Pattern 1, these tests compare
//! `DataFrame::to_html` against live upstream pandas for representative
//! HTML rendering edge cases.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use fp_frame::DataFrame;
use fp_index::IndexLabel;
use fp_types::{NullKind, Scalar};
use serde_json::Value;

use super::HarnessConfig;

#[derive(Debug, Clone, Copy)]
enum HtmlCell<'a> {
    Bool(bool),
    Float(f64),
    Int(i64),
    NaN,
    Text(&'a str),
}

#[derive(Debug, Clone, Copy)]
struct HtmlCase<'a> {
    case_id: &'a str,
    columns: &'a [&'a str],
    values: &'a [(&'a str, &'a [HtmlCell<'a>])],
    index: Option<&'a [&'a str]>,
    include_index: bool,
}

fn cell_to_scalar(cell: HtmlCell<'_>) -> Scalar {
    match cell {
        HtmlCell::Bool(value) => Scalar::Bool(value),
        HtmlCell::Float(value) => Scalar::Float64(value),
        HtmlCell::Int(value) => Scalar::Int64(value),
        HtmlCell::NaN => Scalar::Null(NullKind::NaN),
        HtmlCell::Text(value) => Scalar::Utf8(value.to_owned()),
    }
}

fn cell_to_json(cell: HtmlCell<'_>) -> Value {
    match cell {
        HtmlCell::Bool(value) => serde_json::json!(value),
        HtmlCell::Float(value) => serde_json::json!(value),
        HtmlCell::Int(value) => serde_json::json!(value),
        HtmlCell::NaN => serde_json::json!({"__fp_nan__": true}),
        HtmlCell::Text(value) => serde_json::json!(value),
    }
}

fn row_count(case: HtmlCase<'_>) -> usize {
    case.index.map_or_else(
        || case.values.first().map_or(0, |(_, values)| values.len()),
        <[_]>::len,
    )
}

fn rust_dataframe(case: HtmlCase<'_>) -> DataFrame {
    let index = case.index.map_or_else(
        || {
            (0..row_count(case))
                .map(|value| IndexLabel::Int64(value as i64))
                .collect::<Vec<_>>()
        },
        |labels| {
            labels
                .iter()
                .map(|label| IndexLabel::Utf8((*label).to_owned()))
                .collect::<Vec<_>>()
        },
    );
    let data = case
        .values
        .iter()
        .map(|(name, values)| {
            (
                *name,
                values
                    .iter()
                    .copied()
                    .map(cell_to_scalar)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    DataFrame::from_dict_with_index(data, index).expect("html conformance dataframe")
}

fn pandas_to_html_or_skip(
    config: &HarnessConfig,
    case: HtmlCase<'_>,
) -> Result<Option<String>, String> {
    if !config.oracle_root.exists() && !config.allows_live_oracle_fallback() {
        eprintln!(
            "live pandas unavailable; skipping io.formats conformance test {}: legacy oracle root does not exist: {}",
            case.case_id,
            config.oracle_root.display()
        );
        return Ok(None);
    }

    let rows = (0..row_count(case))
        .map(|row_idx| {
            case.columns
                .iter()
                .map(|column| {
                    case.values
                        .iter()
                        .find(|(name, _)| name == column)
                        .and_then(|(_, values)| values.get(row_idx).copied())
                        .map_or(Value::Null, cell_to_json)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let payload = serde_json::json!({
        "columns": case.columns,
        "rows": rows,
        "index": case.index,
        "include_index": case.include_index,
    });

    let script = r#"
import importlib
import json
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

def decode(value):
    if isinstance(value, dict) and value.get("__fp_nan__") is True:
        return float("nan")
    return value

request = json.loads(sys.stdin.read())
rows = [[decode(value) for value in row] for row in request["rows"]]
df = pd.DataFrame(rows, columns=request["columns"], index=request["index"])
sys.stdout.write(df.to_html(index=request["include_index"]))
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
        .map_err(|err| format!("spawn pandas to_html oracle failed: {err}"))?;

    // Per br-frankenpandas-50b6b0: drain stdin in a worker thread.
    let stdin_handle = child
        .stdin
        .take()
        .ok_or_else(|| "pandas to_html oracle stdin unavailable".to_owned())?;
    let payload_bytes = payload.to_string().into_bytes();
    let stdin_writer = std::thread::spawn(move || -> std::io::Result<()> {
        let mut stdin = stdin_handle;
        stdin.write_all(&payload_bytes)
    });
    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait for pandas to_html oracle failed: {err}"))?;
    stdin_writer
        .join()
        .map_err(|_| "pandas to_html oracle stdin writer panicked".to_owned())?
        .map_err(|err| format!("write pandas to_html oracle payload failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "pandas to_html oracle failed for {}: {}",
            case.case_id,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    String::from_utf8(output.stdout)
        .map(Some)
        .map_err(|err| format!("pandas to_html oracle emitted non-utf8 output: {err}"))
}

fn assert_to_html_matches_pandas(case: HtmlCase<'_>) {
    let config = HarnessConfig::default_paths();
    let Some(expected) = pandas_to_html_or_skip(&config, case).expect("pandas to_html oracle")
    else {
        return;
    };
    let actual = rust_dataframe(case).to_html(case.include_index);

    assert_eq!(
        actual, expected,
        "pandas io.formats to_html parity drift for {}",
        case.case_id
    );
}

#[test]
fn conformance_io_formats_to_html_matrix() {
    let cases = [
        HtmlCase {
            case_id: "to_html_empty_columns_no_index",
            columns: &["a", "b"],
            values: &[("a", &[]), ("b", &[])],
            index: None,
            include_index: false,
        },
        HtmlCase {
            case_id: "to_html_single_row_string_index",
            columns: &["value"],
            values: &[("value", &[HtmlCell::Int(10)])],
            index: Some(&["row0"]),
            include_index: true,
        },
        HtmlCase {
            case_id: "to_html_nan_heavy_float_columns",
            columns: &["a", "b"],
            values: &[
                ("a", &[HtmlCell::Float(1.0), HtmlCell::NaN]),
                ("b", &[HtmlCell::NaN, HtmlCell::Float(3.5)]),
            ],
            index: None,
            include_index: false,
        },
        HtmlCase {
            case_id: "to_html_duplicate_string_index",
            columns: &["v"],
            values: &[("v", &[HtmlCell::Int(1), HtmlCell::Int(2), HtmlCell::Int(3)])],
            index: Some(&["x", "x", "y"]),
            include_index: true,
        },
        HtmlCase {
            case_id: "to_html_mixed_dtypes_escaped_strings",
            columns: &["text", "flag", "n"],
            values: &[
                (
                    "text",
                    &[HtmlCell::Text("<b>&x</b>"), HtmlCell::Text("plain")],
                ),
                ("flag", &[HtmlCell::Bool(true), HtmlCell::Bool(false)]),
                ("n", &[HtmlCell::Int(7), HtmlCell::Int(-2)]),
            ],
            index: None,
            include_index: false,
        },
    ];

    for case in cases {
        assert_to_html_matches_pandas(case);
    }
}
