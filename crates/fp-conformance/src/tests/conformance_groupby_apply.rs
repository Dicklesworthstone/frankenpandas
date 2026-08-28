//! Live pandas shape contracts for `DataFrameGroupBy.apply` (frankenpandas-0kx7).

use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn pandas_groupby_apply_shapes_or_skip() -> Result<Option<Value>, String> {
    // br-frankenpandas-l7r1p: opt into the SYSTEM pandas. Without it the
    // legacy oracle root is absent, the helper below returns early, and the
    // test reports green while comparing nothing.
    let mut config = super::HarnessConfig::default_paths();
    config.allow_system_pandas_fallback = true;
    if !config.oracle_root.exists() && !config.allows_live_oracle_fallback() {
        eprintln!(
            "live pandas unavailable; skipping GroupBy.apply shape conformance: legacy oracle root does not exist: {}",
            config.oracle_root.display()
        );
        return Ok(None);
    }

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

def label(value):
    if pd.isna(value):
        return "<NA>"
    return str(value)

def scalar(value):
    if pd.isna(value):
        return None
    if hasattr(value, "item"):
        value = value.item()
    return value

def index_values(index):
    if isinstance(index, pd.MultiIndex):
        return [[label(part) for part in item] for item in index.tolist()]
    return [label(item) for item in index.tolist()]

def encode(result):
    payload = {
        "kind": "dataframe" if isinstance(result, pd.DataFrame) else "series",
        "index_kind": type(result.index).__name__,
        "index": index_values(result.index),
    }
    if isinstance(result, pd.DataFrame):
        payload["columns"] = [str(column) for column in result.columns.tolist()]
        payload["values"] = [
            [scalar(value) for value in row]
            for row in result.astype(object).to_numpy().tolist()
        ]
    else:
        payload["values"] = [
            scalar(value) for value in result.astype(object).tolist()
        ]
    return payload

base = pd.DataFrame({"grp": ["a", "a", "b", "b"], "val": [10.0, 20.0, 30.0, 40.0]})

cases = {
    "scalar_return": base.groupby("grp").apply(lambda g: g["val"].sum(), include_groups=False),
    "series_same_labels": base.groupby("grp").apply(
        lambda g: pd.Series({"first": g["val"].iloc[0], "last": g["val"].iloc[-1]}),
        include_groups=False,
    ),
    "series_variable_labels": base.groupby("grp").apply(
        lambda g: pd.Series({"first": g["val"].iloc[0]})
        if g["val"].iloc[0] == 10.0
        else pd.Series({"first": g["val"].iloc[0], "last": g["val"].iloc[-1]}),
        include_groups=False,
    ),
    "dataframe_single_row": base.groupby("grp").apply(
        lambda g: pd.DataFrame({"total": [g["val"].sum()]}),
        include_groups=False,
    ),
    "dataframe_multirow": pd.DataFrame(
        {"grp": ["a", "a", "b", "b"], "val": [10.0, 20.0, 30.0, 40.0]},
        index=["r0", "r1", "r2", "r3"],
    ).groupby("grp").apply(lambda g: g[["val"]], include_groups=False),
    "include_groups_false_columns": pd.DataFrame({"grp": ["a", "b"], "val": [1, 2]})
        .groupby("grp")
        .apply(lambda g: pd.DataFrame({"has_grp": ["grp" in g.columns]}), include_groups=False),
    "include_groups_true_columns": pd.DataFrame({"grp": ["a", "b"], "val": [1, 2]})
        .groupby("grp")
        .apply(lambda g: pd.DataFrame({"has_grp": ["grp" in g.columns]}), include_groups=True),
    "sort_false_order": pd.DataFrame({"grp": ["b", "a", "b"], "val": [1, 2, 3]})
        .groupby("grp", sort=False)
        .apply(lambda g: g["val"].sum(), include_groups=False),
    "duplicate_index_labels": pd.DataFrame(
        {"grp": ["a", "a", "b"], "val": [1, 2, 3]},
        index=["r", "r", "r"],
    ).groupby("grp").apply(lambda g: g[["val"]], include_groups=False),
    "dropna_false_nan_key": pd.DataFrame({"grp": ["a", float("nan"), "b"], "val": [1, 2, 3]})
        .groupby("grp", dropna=False)
        .apply(lambda g: g["val"].sum(), include_groups=False),
}

print(json.dumps({name: encode(result) for name, result in cases.items()}, separators=(",", ":")))
"#;

    let child = Command::new(&config.python_bin)
        .arg("-c")
        .arg(script)
        .arg(&config.oracle_root)
        .arg(if config.allows_live_oracle_fallback() {
            "1"
        } else {
            "0"
        })
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("spawn pandas GroupBy.apply oracle failed: {err}"))?;

    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait for pandas GroupBy.apply oracle failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "pandas GroupBy.apply oracle failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    serde_json::from_slice(&output.stdout)
        .map(Some)
        .map_err(|err| format!("parse pandas GroupBy.apply shape json failed: {err}"))
}

#[test]
fn conformance_groupby_apply_live_shape_contracts() -> Result<(), String> {
    let Some(actual) = pandas_groupby_apply_shapes_or_skip()? else {
        return Ok(());
    };

    let expected = json!({
        "scalar_return": {
            "kind": "series",
            "index_kind": "Index",
            "index": ["a", "b"],
            "values": [30.0, 70.0]
        },
        "series_same_labels": {
            "kind": "dataframe",
            "index_kind": "Index",
            "index": ["a", "b"],
            "columns": ["first", "last"],
            "values": [[10.0, 20.0], [30.0, 40.0]]
        },
        "series_variable_labels": {
            "kind": "series",
            "index_kind": "MultiIndex",
            "index": [["a", "first"], ["b", "first"], ["b", "last"]],
            "values": [10.0, 30.0, 40.0]
        },
        "dataframe_single_row": {
            "kind": "dataframe",
            "index_kind": "MultiIndex",
            "index": [["a", "0"], ["b", "0"]],
            "columns": ["total"],
            "values": [[30.0], [70.0]]
        },
        "dataframe_multirow": {
            "kind": "dataframe",
            "index_kind": "MultiIndex",
            "index": [["a", "r0"], ["a", "r1"], ["b", "r2"], ["b", "r3"]],
            "columns": ["val"],
            "values": [[10.0], [20.0], [30.0], [40.0]]
        },
        "include_groups_false_columns": {
            "kind": "dataframe",
            "index_kind": "MultiIndex",
            "index": [["a", "0"], ["b", "0"]],
            "columns": ["has_grp"],
            "values": [[false], [false]]
        },
        "include_groups_true_columns": {
            "kind": "dataframe",
            "index_kind": "MultiIndex",
            "index": [["a", "0"], ["b", "0"]],
            "columns": ["has_grp"],
            "values": [[true], [true]]
        },
        "sort_false_order": {
            "kind": "series",
            "index_kind": "Index",
            "index": ["b", "a"],
            "values": [4, 2]
        },
        "duplicate_index_labels": {
            "kind": "dataframe",
            "index_kind": "MultiIndex",
            "index": [["a", "r"], ["a", "r"], ["b", "r"]],
            "columns": ["val"],
            "values": [[1], [2], [3]]
        },
        "dropna_false_nan_key": {
            "kind": "series",
            "index_kind": "Index",
            "index": ["a", "b", "<NA>"],
            "values": [1, 3, 2]
        }
    });

    assert_eq!(actual, expected);
    Ok(())
}
