//! GroupBy parity-matrix conformance suite (br-frankenpandas-yzh4).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares an
//! existing Series GroupBy aggregate operation with live upstream pandas for
//! edge-case inputs: empty groups, single groups, NaN-heavy values, duplicate
//! keys, mixed key dtypes, multi-key grouping, and ordered larger inputs.

use super::{
    CaseStatus, HarnessConfig, HarnessError, OracleMode, PacketFixture, ResolvedExpected,
    SuiteOptions, capture_live_oracle_expected,
};

fn strict_config() -> HarnessConfig {
    let mut cfg = HarnessConfig::default_paths();
    // br-frankenpandas-l7r1p: without this every fixture in this file SKIPS.
    // The legacy oracle root (`legacy_pandas_code/pandas`) does not exist on
    // this host, so `capture_live_oracle_expected` returns OracleUnavailable
    // and the check helper returns before comparing anything -- green while
    // executing no differential at all. The live_oracle_* suites already opt
    // into the SYSTEM pandas (2.2.3); the conformance_* suites never did.
    cfg.allow_system_pandas_fallback = true;
    cfg
}

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Series(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping GroupBy conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_groupby_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("groupby oracle") {
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
        "pandas GroupBy parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

#[test]
fn conformance_groupby_sum_empty_input() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-SUM-001",
        "case_id": "groupby_sum_empty_input",
        "mode": "strict",
        "operation": "groupby_sum",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "key", "index": [], "values": [] },
        "right": { "name": "value", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_sum_single_group() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-SUM-002",
        "case_id": "groupby_sum_single_group",
        "mode": "strict",
        "operation": "groupby_sum",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "utf8", "value": "only" },
                { "kind": "utf8", "value": "only" },
                { "kind": "utf8", "value": "only" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "values": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_sum_nan_heavy_values() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-SUM-003",
        "case_id": "groupby_sum_nan_heavy_values",
        "mode": "strict",
        "operation": "groupby_sum",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 5 },
                { "kind": "null", "value": "na_n" },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "na_n" }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_mean_duplicate_keys_float_values() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-MEAN-001",
        "case_id": "groupby_mean_duplicate_keys_float_values",
        "mode": "strict",
        "operation": "groupby_mean",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "float64", "value": 1.5 },
                { "kind": "float64", "value": 2.5 },
                { "kind": "float64", "value": 10.0 },
                { "kind": "float64", "value": 14.0 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_count_null_heavy_values() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-COUNT-001",
        "case_id": "groupby_count_null_heavy_values",
        "mode": "strict",
        "operation": "groupby_count",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 3.0 },
                { "kind": "null", "value": "null" },
                { "kind": "float64", "value": 4.0 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_min_negative_values() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-MIN-001",
        "case_id": "groupby_min_negative_values",
        "mode": "strict",
        "operation": "groupby_min",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": -1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 5 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_max_duplicate_keys() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-MAX-001",
        "case_id": "groupby_max_duplicate_keys",
        "mode": "strict",
        "operation": "groupby_max",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "values": [
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": -1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 5 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

#[test]
fn conformance_groupby_first_and_last_skip_missing_values() {
    let first_fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-FIRST-001",
        "case_id": "groupby_first_skip_missing_values",
        "mode": "strict",
        "operation": "groupby_first",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "c" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 6.0 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(first_fixture);

    let last_fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-LAST-001",
        "case_id": "groupby_last_skip_missing_values",
        "mode": "strict",
        "operation": "groupby_last",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "key",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" },
                { "kind": "utf8", "value": "c" }
            ]
        },
        "right": {
            "name": "value",
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "values": [
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 },
                { "kind": "null", "value": "null" },
                { "kind": "null", "value": "na_n" },
                { "kind": "float64", "value": 6.0 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(last_fixture);
}

#[test]
fn conformance_groupby_sum_multi_key_mixed_dtypes() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-GROUPBY-MULTI-001",
        "case_id": "groupby_sum_multi_key_mixed_dtypes",
        "mode": "strict",
        "operation": "groupby_sum",
        "oracle_source": "live_legacy_pandas",
        "groupby_keys": [
            {
                "name": "region",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "values": [
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "west" },
                    { "kind": "utf8", "value": "west" }
                ]
            },
            {
                "name": "bucket",
                "index": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "values": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ]
            }
        ],
        "right": {
            "name": "value",
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
                { "kind": "int64", "value": 40 }
            ]
        }
    }))
    .expect("fixture");
    check_groupby_fixture(fixture);
}

fn run_pandas_oracle_eval(code: &str) -> Option<serde_json::Value> {
    use std::io::Write;
    let cfg = strict_config();
    if !cfg.oracle_root.exists() && !cfg.allows_live_oracle_fallback() {
        return None;
    }
    let mut child = std::process::Command::new(&cfg.python_bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(code.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No module named 'pandas'") {
            return None;
        }
        assert!(
            output.status.success(),
            "pandas oracle script failed:\n{stderr}"
        );
    }
    Some(serde_json::from_slice(&output.stdout).expect("valid oracle json"))
}

fn series_f64_vec(series: &fp_frame::Series) -> Vec<Option<f64>> {
    series
        .values()
        .iter()
        .map(|s| {
            if s.is_missing() {
                None
            } else {
                s.to_f64().ok().filter(|v| !v.is_nan())
            }
        })
        .collect()
}

fn df_col_f64_vec(df: &fp_frame::DataFrame, col: &str) -> Vec<Option<f64>> {
    df.column(col)
        .expect(col)
        .values()
        .iter()
        .map(|s| {
            if s.is_missing() {
                None
            } else {
                s.to_f64().ok().filter(|v| !v.is_nan())
            }
        })
        .collect()
}

fn assert_opt_f64_slice_close(actual: &[Option<f64>], expected: &serde_json::Value, ctx: &str) {
    assert!(expected.is_array(), "{ctx}: expected json array");
    let arr = expected.as_array().expect("checked array");
    assert_eq!(actual.len(), arr.len(), "{ctx}: length mismatch");
    for (idx, (a, e)) in actual.iter().zip(arr.iter()).enumerate() {
        match (a, e.as_f64()) {
            (None, None) => {}
            (Some(av), Some(ev)) => {
                assert!(
                    (av - ev).abs() <= 1e-9,
                    "{ctx}[{idx}]: actual={av} != expected={ev}"
                );
            }
            (other_a, other_e) => {
                assert_eq!(
                    other_a.is_some(),
                    other_e.is_some(),
                    "{ctx}[{idx}]: nullability mismatch actual={other_a:?} expected={other_e:?}"
                );
            }
        }
    }
}

#[test]
fn live_oracle_dataframe_groupby_aggregations_differential() {
    // @covers: DataFrameGroupBy.sum, DataFrameGroupBy.mean, DataFrameGroupBy.min, DataFrameGroupBy.max
    // @covers: DataFrameGroupBy.count, DataFrameGroupBy.size, DataFrameGroupBy.std, DataFrameGroupBy.var
    // @covers: DataFrameGroupBy.sem, DataFrameGroupBy.skew, DataFrameGroupBy.prod, DataFrameGroupBy.median
    // @covers: DataFrameGroupBy.quantile, DataFrameGroupBy.first, DataFrameGroupBy.last, DataFrameGroupBy.nth
    // @covers: DataFrameGroupBy.nunique, DataFrameGroupBy.idxmin, DataFrameGroupBy.idxmax
    // @covers: DataFrameGroupBy.all, DataFrameGroupBy.any, DataFrameGroupBy.agg, DataFrameGroupBy.aggregate
    // @covers: DataFrameGroupBy.ohlc, DataFrameGroupBy.describe, DataFrameGroupBy.value_counts
    use std::collections::{BTreeMap, HashMap};

    use fp_columnar::Column;
    use fp_frame::DataFrame;
    use fp_index::Index;
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, math, pandas as pd

df = pd.DataFrame(
    {
        "k": ["a", "a", "a", "b", "b", "b"],
        "v": [1.0, 2.0, 6.0, 10.0, float("nan"), 30.0],
        "w": [2.0, 4.0, 4.0, 5.0, 15.0, 25.0],
    },
    index=[10, 20, 30, 40, 50, 60],
)
gb = df.groupby("k")

def clean_list(vals):
    out = []
    for x in vals:
        if x is None or (isinstance(x, float) and (math.isnan(x) or math.isinf(x))) or pd.isna(x):
            out.append(None)
        elif hasattr(x, "item"):
            out.append(float(x.item()))
        else:
            out.append(float(x))
    return out

df_bool = pd.DataFrame(
    {
        "k": ["a", "a", "b", "b"],
        "flag": [True, False, True, True],
    }
)
gb_bool = df_bool.groupby("k")

ohlc_df = gb.ohlc()
desc_df = gb[["v"]].describe()
vc_s = pd.DataFrame({"k": ["a", "a", "a", "b", "b"], "tag": ["x", "x", "y", "z", "z"]}).groupby("k").value_counts()

res = {
    "sum_v": clean_list(gb.sum()["v"].tolist()),
    "mean_v": clean_list(gb.mean()["v"].tolist()),
    "min_v": clean_list(gb.min()["v"].tolist()),
    "max_v": clean_list(gb.max()["v"].tolist()),
    "count_v": clean_list(gb.count()["v"].tolist()),
    "size": clean_list(gb.size().tolist()),
    "std_v": clean_list(gb.std()["v"].tolist()),
    "var_v": clean_list(gb.var()["v"].tolist()),
    "sem_v": clean_list(gb.sem()["v"].tolist()),
    "skew_w": clean_list(gb.skew()["w"].tolist()),
    "prod_v": clean_list(gb.prod()["v"].tolist()),
    "median_v": clean_list(gb.median()["v"].tolist()),
    "quantile_v": clean_list(gb.quantile(0.5)["v"].tolist()),
    "first_v": clean_list(gb.first()["v"].tolist()),
    "last_v": clean_list(gb.last()["v"].tolist()),
    "nth_1_w": clean_list(gb.nth(1)["w"].tolist()),
    "nunique_w": clean_list(gb.nunique()["w"].tolist()),
    "idxmin_v": [int(x) for x in gb.idxmin()["v"].tolist()],
    "idxmax_v": [int(x) for x in gb.idxmax()["v"].tolist()],
    "all_flag": [bool(x) for x in gb_bool.all()["flag"].tolist()],
    "any_flag": [bool(x) for x in gb_bool.any()["flag"].tolist()],
    "agg_v": clean_list(gb.agg({"v": "sum", "w": "mean"})["v"].tolist()),
    "aggregate_w": clean_list(gb.aggregate({"v": "sum", "w": "mean"})["w"].tolist()),
    "ohlc_v_open": clean_list(ohlc_df[("v", "open")].tolist()),
    "ohlc_v_high": clean_list(ohlc_df[("v", "high")].tolist()),
    "ohlc_v_low": clean_list(ohlc_df[("v", "low")].tolist()),
    "ohlc_v_close": clean_list(ohlc_df[("v", "close")].tolist()),
    "describe_v_a_mean": float(desc_df.loc["a", ("v", "mean")]),
    "describe_v_b_max": float(desc_df.loc["b", ("v", "max")]),
    "vc_counts": [int(x) for x in vc_s.tolist()],
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        return;
    };

    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    );
    cols.insert(
        "v".to_string(),
        Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(6.0),
            Scalar::Float64(10.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(30.0),
        ])
        .unwrap(),
    );
    cols.insert(
        "w".to_string(),
        Column::from_values(vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
            Scalar::Float64(15.0),
            Scalar::Float64(25.0),
        ])
        .unwrap(),
    );
    let df = DataFrame::new_with_column_order(
        Index::from_i64(vec![10, 20, 30, 40, 50, 60]),
        cols,
        vec!["k".into(), "v".into(), "w".into()],
    )
    .unwrap();
    let gb = df.groupby(&["k"]).unwrap();

    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.sum().unwrap(), "v"),
        &oracle["sum_v"],
        "df_gb.sum",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.mean().unwrap(), "v"),
        &oracle["mean_v"],
        "df_gb.mean",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.min().unwrap(), "v"),
        &oracle["min_v"],
        "df_gb.min",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.max().unwrap(), "v"),
        &oracle["max_v"],
        "df_gb.max",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.count().unwrap(), "v"),
        &oracle["count_v"],
        "df_gb.count",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.size().unwrap()),
        &oracle["size"],
        "df_gb.size",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.std().unwrap(), "v"),
        &oracle["std_v"],
        "df_gb.std",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.var().unwrap(), "v"),
        &oracle["var_v"],
        "df_gb.var",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.sem().unwrap(), "v"),
        &oracle["sem_v"],
        "df_gb.sem",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.skew().unwrap(), "w"),
        &oracle["skew_w"],
        "df_gb.skew",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.prod().unwrap(), "v"),
        &oracle["prod_v"],
        "df_gb.prod",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.median().unwrap(), "v"),
        &oracle["median_v"],
        "df_gb.median",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.quantile(0.5).unwrap(), "v"),
        &oracle["quantile_v"],
        "df_gb.quantile",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.first().unwrap(), "v"),
        &oracle["first_v"],
        "df_gb.first",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.last().unwrap(), "v"),
        &oracle["last_v"],
        "df_gb.last",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.nth(1).unwrap(), "w"),
        &oracle["nth_1_w"],
        "df_gb.nth",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.nunique().unwrap(), "w"),
        &oracle["nunique_w"],
        "df_gb.nunique",
    );

    let idxmin_df = gb.idxmin().unwrap();
    let idxmin_vals: Vec<i64> = idxmin_df
        .column("v")
        .unwrap()
        .values()
        .iter()
        .map(|s| s.to_i64().unwrap())
        .collect();
    let oracle_idxmin: Vec<i64> = oracle["idxmin_v"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    assert_eq!(idxmin_vals, oracle_idxmin);

    let idxmax_df = gb.idxmax().unwrap();
    let idxmax_vals: Vec<i64> = idxmax_df
        .column("v")
        .unwrap()
        .values()
        .iter()
        .map(|s| s.to_i64().unwrap())
        .collect();
    let oracle_idxmax: Vec<i64> = oracle["idxmax_v"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    assert_eq!(idxmax_vals, oracle_idxmax);

    let mut bcols = BTreeMap::new();
    bcols.insert(
        "k".to_string(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    );
    bcols.insert(
        "flag".to_string(),
        Column::from_values(vec![
            Scalar::Bool(true),
            Scalar::Bool(false),
            Scalar::Bool(true),
            Scalar::Bool(true),
        ])
        .unwrap(),
    );
    let df_bool = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1, 2, 3]),
        bcols,
        vec!["k".into(), "flag".into()],
    )
    .unwrap();
    let gb_bool = df_bool.groupby(&["k"]).unwrap();
    let all_vals: Vec<bool> = gb_bool
        .all()
        .unwrap()
        .column("flag")
        .unwrap()
        .values()
        .iter()
        .map(|s| match s {
            Scalar::Bool(b) => *b,
            other => panic!("expected bool, got {other:?}"),
        })
        .collect();
    let any_vals: Vec<bool> = gb_bool
        .any()
        .unwrap()
        .column("flag")
        .unwrap()
        .values()
        .iter()
        .map(|s| match s {
            Scalar::Bool(b) => *b,
            other => panic!("expected bool, got {other:?}"),
        })
        .collect();
    let oracle_all: Vec<bool> = oracle["all_flag"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_bool().unwrap())
        .collect();
    let oracle_any: Vec<bool> = oracle["any_flag"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_bool().unwrap())
        .collect();
    assert_eq!(all_vals, oracle_all);
    assert_eq!(any_vals, oracle_any);

    let mut func_map = HashMap::new();
    func_map.insert("v".to_string(), "sum".to_string());
    func_map.insert("w".to_string(), "mean".to_string());
    let agg_df = gb.agg(&func_map).unwrap();
    let aggregate_df = gb.aggregate(&func_map).unwrap();
    assert_opt_f64_slice_close(&df_col_f64_vec(&agg_df, "v"), &oracle["agg_v"], "df_gb.agg");
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&aggregate_df, "w"),
        &oracle["aggregate_w"],
        "df_gb.aggregate",
    );

    let ohlc_df = gb.ohlc().unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&ohlc_df, "v_open"),
        &oracle["ohlc_v_open"],
        "ohlc_open",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&ohlc_df, "v_high"),
        &oracle["ohlc_v_high"],
        "ohlc_high",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&ohlc_df, "v_low"),
        &oracle["ohlc_v_low"],
        "ohlc_low",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&ohlc_df, "v_close"),
        &oracle["ohlc_v_close"],
        "ohlc_close",
    );

    let desc_df = gb.describe().unwrap();
    let desc_v = df_col_f64_vec(&desc_df, "v");
    // group 'a' mean is row 1 (count=0, mean=1), group 'b' max is row 15 (8 + 7)
    assert!((desc_v[1].unwrap() - oracle["describe_v_a_mean"].as_f64().unwrap()).abs() <= 1e-9);
    assert!((desc_v[15].unwrap() - oracle["describe_v_b_max"].as_f64().unwrap()).abs() <= 1e-9);

    let mut vccols = BTreeMap::new();
    vccols.insert(
        "k".to_string(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    );
    vccols.insert(
        "tag".to_string(),
        Column::from_values(vec![
            Scalar::Utf8("x".into()),
            Scalar::Utf8("x".into()),
            Scalar::Utf8("y".into()),
            Scalar::Utf8("z".into()),
            Scalar::Utf8("z".into()),
        ])
        .unwrap(),
    );
    let vc_df = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1, 2, 3, 4]),
        vccols,
        vec!["k".into(), "tag".into()],
    )
    .unwrap()
    .groupby(&["k"])
    .unwrap()
    .value_counts()
    .unwrap();
    let vc_counts: Vec<i64> = vc_df
        .column("count")
        .unwrap()
        .values()
        .iter()
        .map(|s| s.to_i64().unwrap())
        .collect();
    let oracle_vc: Vec<i64> = oracle["vc_counts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    assert_eq!(vc_counts, oracle_vc);
}

#[test]
fn live_oracle_dataframe_groupby_cumulative_and_window_differential() {
    // @covers: DataFrameGroupBy.cumsum, DataFrameGroupBy.cumprod, DataFrameGroupBy.cummin, DataFrameGroupBy.cummax
    // @covers: DataFrameGroupBy.cumcount, DataFrameGroupBy.ngroup, DataFrameGroupBy.rank
    // @covers: DataFrameGroupBy.shift, DataFrameGroupBy.diff, DataFrameGroupBy.pct_change
    // @covers: DataFrameGroupBy.ffill, DataFrameGroupBy.bfill, DataFrameGroupBy.fillna
    // @covers: DataFrameGroupBy.head, DataFrameGroupBy.tail, DataFrameGroupBy.take, DataFrameGroupBy.sample
    // @covers: DataFrameGroupBy.rolling, DataFrameGroupBy.expanding, DataFrameGroupBy.ewm, DataFrameGroupBy.resample
    use std::collections::BTreeMap;

    use fp_columnar::Column;
    use fp_frame::DataFrame;
    use fp_index::{Index, IndexLabel};
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, math, pandas as pd

df = pd.DataFrame(
    {
        "k": ["a", "a", "a", "b", "b", "b"],
        "v": [2.0, 4.0, 8.0, 10.0, 20.0, 5.0],
        "na_col": [1.0, float("nan"), 3.0, float("nan"), 5.0, float("nan")],
    }
)
gb = df.groupby("k")

def clean_list(vals):
    out = []
    for x in vals:
        if x is None or (isinstance(x, float) and (math.isnan(x) or math.isinf(x))) or pd.isna(x):
            out.append(None)
        elif hasattr(x, "item"):
            out.append(float(x.item()))
        else:
            out.append(float(x))
    return out

ts_df = pd.DataFrame(
    {
        "k": ["a", "a", "b", "b"],
        "v": [1.0, 3.0, 10.0, 20.0],
    },
    index=pd.to_datetime([
        "2024-01-01 00:00:00",
        "2024-01-01 00:30:00",
        "2024-01-01 00:00:00",
        "2024-01-01 00:30:00",
    ]),
)

res = {
    "cumsum_v": clean_list(gb.cumsum()["v"].tolist()),
    "cumprod_v": clean_list(gb.cumprod()["v"].tolist()),
    "cummin_v": clean_list(gb.cummin()["v"].tolist()),
    "cummax_v": clean_list(gb.cummax()["v"].tolist()),
    "cumcount": clean_list(gb.cumcount().tolist()),
    "ngroup": clean_list(gb.ngroup().tolist()),
    "rank_v": clean_list(gb.rank(method="average", ascending=True, na_option="keep")["v"].tolist()),
    "shift_v": clean_list(gb.shift(1)["v"].tolist()),
    "diff_v": clean_list(gb.diff(1)["v"].tolist()),
    "pct_change_v": clean_list(gb[["v"]].pct_change(1)["v"].tolist()),
    "ffill_na": clean_list(gb.ffill()["na_col"].tolist()),
    "bfill_na": clean_list(gb.bfill()["na_col"].tolist()),
    "head_1_v": clean_list(gb.head(1)["v"].tolist()),
    "tail_1_v": clean_list(gb.tail(1)["v"].tolist()),
    "take_0_2_v": clean_list(gb.take([0, 2])["v"].tolist()),
    "sample_1_len": int(len(gb.sample(n=1, random_state=42))),
    "rolling_2_sum_v": clean_list(gb["v"].rolling(2).sum().tolist()),
    "expanding_sum_v": clean_list(gb["v"].expanding(1).sum().tolist()),
    "ewm_mean_v": clean_list(gb["v"].ewm(span=2.0).mean().tolist()),
    "resample_1d_sum_v": clean_list(ts_df.groupby("k").resample("1D").sum()["v"].tolist()),
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        return;
    };

    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    );
    cols.insert(
        "v".to_string(),
        Column::from_values(vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(8.0),
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
            Scalar::Float64(5.0),
        ])
        .unwrap(),
    );
    cols.insert(
        "na_col".to_string(),
        Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(5.0),
            Scalar::Null(NullKind::NaN),
        ])
        .unwrap(),
    );
    let df = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1, 2, 3, 4, 5]),
        cols,
        vec!["k".into(), "v".into(), "na_col".into()],
    )
    .unwrap();
    let gb = df.groupby(&["k"]).unwrap();

    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.cumsum().unwrap(), "v"),
        &oracle["cumsum_v"],
        "cumsum",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.cumprod().unwrap(), "v"),
        &oracle["cumprod_v"],
        "cumprod",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.cummin().unwrap(), "v"),
        &oracle["cummin_v"],
        "cummin",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.cummax().unwrap(), "v"),
        &oracle["cummax_v"],
        "cummax",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.cumcount().unwrap()),
        &oracle["cumcount"],
        "cumcount",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.ngroup().unwrap()),
        &oracle["ngroup"],
        "ngroup",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.rank("average", true, "keep").unwrap(), "v"),
        &oracle["rank_v"],
        "rank",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.shift(1).unwrap(), "v"),
        &oracle["shift_v"],
        "shift",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.diff(1).unwrap(), "v"),
        &oracle["diff_v"],
        "diff",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.pct_change(1).unwrap(), "v"),
        &oracle["pct_change_v"],
        "pct_change",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.ffill(None).unwrap(), "na_col"),
        &oracle["ffill_na"],
        "ffill",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.bfill(None).unwrap(), "na_col"),
        &oracle["bfill_na"],
        "bfill",
    );

    let filled = gb.fillna(&Scalar::Float64(99.0)).unwrap();
    assert_eq!(
        df_col_f64_vec(&filled, "na_col"),
        vec![
            Some(1.0),
            Some(99.0),
            Some(3.0),
            Some(99.0),
            Some(5.0),
            Some(99.0)
        ]
    );

    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.head(1).unwrap(), "v"),
        &oracle["head_1_v"],
        "head",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.tail(1).unwrap(), "v"),
        &oracle["tail_1_v"],
        "tail",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.take(&[0, 2]).unwrap(), "v"),
        &oracle["take_0_2_v"],
        "take",
    );

    let sampled = gb.sample(Some(1), None, false, Some(42)).unwrap();
    assert_eq!(
        sampled.len(),
        oracle["sample_1_len"].as_u64().unwrap() as usize
    );

    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.rolling(2).sum().unwrap(), "v"),
        &oracle["rolling_2_sum_v"],
        "rolling_sum",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.expanding(Some(1)).sum().unwrap(), "v"),
        &oracle["expanding_sum_v"],
        "expanding_sum",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.ewm(Some(2.0), None).mean().unwrap(), "v"),
        &oracle["ewm_mean_v"],
        "ewm_mean",
    );

    let mut ts_cols = BTreeMap::new();
    ts_cols.insert(
        "k".to_string(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    );
    ts_cols.insert(
        "v".to_string(),
        Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Float64(3.0),
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
        ])
        .unwrap(),
    );
    let ts_df = DataFrame::new_with_column_order(
        Index::new(vec![
            IndexLabel::Utf8("2024-01-01 00:00:00".into()),
            IndexLabel::Utf8("2024-01-01 00:30:00".into()),
            IndexLabel::Utf8("2024-01-01 00:00:00".into()),
            IndexLabel::Utf8("2024-01-01 00:30:00".into()),
        ]),
        ts_cols,
        vec!["k".into(), "v".into()],
    )
    .unwrap();
    let resampled = ts_df.groupby(&["k"]).unwrap().resample("1D").sum().unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&resampled, "v"),
        &oracle["resample_1d_sum_v"],
        "resample_sum",
    );
}

#[test]
fn live_oracle_dataframe_groupby_transform_filter_apply_pipe_stats_differential() {
    // @covers: DataFrameGroupBy.transform, DataFrameGroupBy.filter, DataFrameGroupBy.apply, DataFrameGroupBy.pipe
    // @covers: DataFrameGroupBy.get_group, DataFrameGroupBy.corr, DataFrameGroupBy.cov, DataFrameGroupBy.corrwith
    // @covers: DataFrameGroupBy.groups, DataFrameGroupBy.indices, DataFrameGroupBy.ngroups, DataFrameGroupBy.ndim
    // @covers: DataFrameGroupBy.dtypes, DataFrameGroupBy.keys, DataFrameGroupBy.level, DataFrameGroupBy.grouper
    // @covers: DataFrameGroupBy.plot, DataFrameGroupBy.hist, DataFrameGroupBy.boxplot
    use std::collections::{BTreeMap, HashMap};

    use fp_columnar::Column;
    use fp_frame::DataFrame;
    use fp_index::{Index, IndexLabel};
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, math, pandas as pd

df = pd.DataFrame(
    {
        "k": ["b", "a", "b", "a"],
        "x": [1.0, 2.0, 3.0, 6.0],
        "y": [2.0, 10.0, 6.0, 30.0],
    }
)
gb = df.groupby("k")

def clean_list(vals):
    out = []
    for x in vals:
        if x is None or (isinstance(x, float) and (math.isnan(x) or math.isinf(x))) or pd.isna(x):
            out.append(None)
        elif hasattr(x, "item"):
            out.append(float(x.item()))
        else:
            out.append(float(x))
    return out

other = pd.DataFrame({"x": [2.0, 4.0, 6.0, 12.0], "y": [1.0, 5.0, 3.0, 15.0]})
df_na_key = pd.DataFrame({"k": ["a", None, "b", None], "x": [1.0, 2.0, 3.0, 4.0]})

neg_err = False
try:
    gb.get_group("nonexistent_key")
except KeyError:
    neg_err = True

neg_agg_err = False
try:
    gb.agg({"x": "not_a_valid_pandas_agg"})
except AttributeError:
    neg_agg_err = True

res = {
    "transform_mean_x": clean_list(gb.transform("mean")["x"].tolist()),
    "filter_x": clean_list(gb.filter(lambda g: g["x"].sum() > 5.0)["x"].tolist()),
    "apply_x": clean_list(gb.apply(lambda g: g[["x"]], include_groups=False)["x"].tolist()),
    "pipe_sum_x": clean_list(gb.pipe(lambda g: g.sum())["x"].tolist()),
    "get_group_a_x": clean_list(gb.get_group("a")["x"].tolist()),
    "corr_x_y_a": float(gb.corr().loc[("a", "x"), "y"]),
    "cov_x_y_a": float(gb.cov().loc[("a", "x"), "y"]),
    "corrwith_x": clean_list(gb.corrwith(other[["x"]])["x"].tolist()),
    "ngroups": int(gb.ngroups),
    "ndim": int(gb.ndim),
    "indices_a": [int(i) for i in gb.indices["a"]],
    "groups_b": [int(i) for i in gb.groups["b"]],
    "sort_false_sum_x": clean_list(df.groupby("k", sort=False).sum()["x"].tolist()),
    "as_index_false_cols": list(df.groupby("k", as_index=False).sum().columns),
    "dropna_false_sum_x": clean_list(df_na_key.groupby("k", dropna=False).sum()["x"].tolist()),
    "neg_err": neg_err,
    "neg_agg_err": neg_agg_err,
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        return;
    };

    let mut cols = BTreeMap::new();
    cols.insert(
        "k".to_string(),
        Column::from_values(vec![
            Scalar::Utf8("b".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("a".into()),
        ])
        .unwrap(),
    );
    cols.insert(
        "x".to_string(),
        Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
            Scalar::Float64(6.0),
        ])
        .unwrap(),
    );
    cols.insert(
        "y".to_string(),
        Column::from_values(vec![
            Scalar::Float64(2.0),
            Scalar::Float64(10.0),
            Scalar::Float64(6.0),
            Scalar::Float64(30.0),
        ])
        .unwrap(),
    );
    let df = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1, 2, 3]),
        cols,
        vec!["k".into(), "x".into(), "y".into()],
    )
    .unwrap();
    let gb = df.groupby(&["k"]).unwrap();

    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb.transform("mean").unwrap(), "x"),
        &oracle["transform_mean_x"],
        "transform_mean",
    );

    let filtered = gb
        .filter(|g| {
            let s: f64 = g
                .column("x")
                .unwrap()
                .values()
                .iter()
                .filter_map(|v| v.to_f64().ok())
                .sum();
            Ok(s > 5.0)
        })
        .unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&filtered, "x"),
        &oracle["filter_x"],
        "filter",
    );

    let applied = gb
        .apply_with_include_groups(false, |g| Ok(g.clone()))
        .unwrap();
    assert_opt_f64_slice_close(&df_col_f64_vec(&applied, "x"), &oracle["apply_x"], "apply");

    let piped = gb.pipe(|g| g.sum()).unwrap();
    assert_opt_f64_slice_close(&df_col_f64_vec(&piped, "x"), &oracle["pipe_sum_x"], "pipe");

    let group_a = gb.get_group("a").unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&group_a, "x"),
        &oracle["get_group_a_x"],
        "get_group",
    );

    let corr_df = gb.corr().unwrap();
    let corr_y = df_col_f64_vec(&corr_df, "y");
    assert!((corr_y[0].unwrap() - oracle["corr_x_y_a"].as_f64().unwrap()).abs() <= 1e-9);

    let cov_df = gb.cov().unwrap();
    let cov_y = df_col_f64_vec(&cov_df, "y");
    assert!((cov_y[0].unwrap() - oracle["cov_x_y_a"].as_f64().unwrap()).abs() <= 1e-9);

    let mut ocols = BTreeMap::new();
    ocols.insert(
        "x".to_string(),
        Column::from_values(vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(6.0),
            Scalar::Float64(12.0),
        ])
        .unwrap(),
    );
    let other_df = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1, 2, 3]),
        ocols,
        vec!["x".into()],
    )
    .unwrap();
    let corrwith_df = gb.corrwith(&other_df).unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&corrwith_df, "x"),
        &oracle["corrwith_x"],
        "corrwith",
    );

    assert_eq!(gb.ngroups(), oracle["ngroups"].as_u64().unwrap() as usize);
    assert_eq!(gb.ndim(), oracle["ndim"].as_u64().unwrap() as usize);
    assert_eq!(
        gb.keys(),
        vec![IndexLabel::Utf8("a".into()), IndexLabel::Utf8("b".into())]
    );
    assert_eq!(gb.grouper(), vec!["k".to_string()]);
    assert_eq!(gb.level(), vec!["k".to_string()]);
    assert_eq!(gb.dtypes().unwrap().len(), 2);

    let indices = gb.indices();
    let oracle_idx_a: Vec<usize> = oracle["indices_a"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(indices[&IndexLabel::Utf8("a".into())], oracle_idx_a);

    let groups = gb.groups();
    let oracle_grp_b: Vec<usize> = oracle["groups_b"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(groups[&IndexLabel::Utf8("b".into())], oracle_grp_b);

    assert!(gb.plot().unwrap().to_svg().unwrap().contains("<svg"));
    assert!(gb.hist().unwrap().to_svg().unwrap().contains("<svg"));
    assert!(gb.boxplot().unwrap().to_svg().unwrap().contains("<svg"));

    // Variants: sort=False, as_index=False, dropna=False
    let gb_sort_false = df.groupby_full(&["k"], true, false).unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb_sort_false.sum().unwrap(), "x"),
        &oracle["sort_false_sum_x"],
        "sort_false_sum",
    );

    let gb_as_index_false = df.groupby_with_as_index(&["k"], false).unwrap();
    let as_index_false_df = gb_as_index_false.sum().unwrap();
    let oracle_cols: Vec<String> = oracle["as_index_false_cols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let actual_cols: Vec<String> = as_index_false_df
        .column_names()
        .into_iter()
        .cloned()
        .collect();
    assert_eq!(actual_cols, oracle_cols);

    let mut nacols = BTreeMap::new();
    nacols.insert(
        "k".to_string(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("b".into()),
            Scalar::Null(NullKind::Null),
        ])
        .unwrap(),
    );
    nacols.insert(
        "x".to_string(),
        Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
            Scalar::Float64(4.0),
        ])
        .unwrap(),
    );
    let df_na_key = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1, 2, 3]),
        nacols,
        vec!["k".into(), "x".into()],
    )
    .unwrap();
    let gb_dropna_false = df_na_key
        .groupby_full_options(&["k"], true, true, false)
        .unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&gb_dropna_false.sum().unwrap(), "x"),
        &oracle["dropna_false_sum_x"],
        "dropna_false_sum",
    );

    // NEGATIVE cases: both pandas and FrankenPandas reject nonexistent group and invalid agg function
    assert!(oracle["neg_err"].as_bool().unwrap());
    assert!(gb.get_group("nonexistent_key").is_err());

    assert!(oracle["neg_agg_err"].as_bool().unwrap());
    let mut bad_agg = HashMap::new();
    bad_agg.insert("x".to_string(), "not_a_valid_pandas_agg".to_string());
    assert!(gb.agg(&bad_agg).is_err());
}

#[test]
fn live_oracle_series_groupby_aggregations_differential() {
    // @covers: SeriesGroupBy.sum, SeriesGroupBy.mean, SeriesGroupBy.min, SeriesGroupBy.max
    // @covers: SeriesGroupBy.count, SeriesGroupBy.size, SeriesGroupBy.std, SeriesGroupBy.var
    // @covers: SeriesGroupBy.sem, SeriesGroupBy.skew, SeriesGroupBy.prod, SeriesGroupBy.median
    // @covers: SeriesGroupBy.quantile, SeriesGroupBy.first, SeriesGroupBy.last, SeriesGroupBy.nth
    // @covers: SeriesGroupBy.nunique, SeriesGroupBy.unique, SeriesGroupBy.idxmin, SeriesGroupBy.idxmax
    // @covers: SeriesGroupBy.all, SeriesGroupBy.any, SeriesGroupBy.agg, SeriesGroupBy.aggregate
    // @covers: SeriesGroupBy.ohlc, SeriesGroupBy.describe, SeriesGroupBy.value_counts
    // @covers: SeriesGroupBy.nlargest, SeriesGroupBy.nsmallest
    use fp_columnar::Column;
    use fp_frame::Series;
    use fp_index::Index;
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, math, pandas as pd

k = pd.Series(["a", "a", "a", "b", "b", "b"], index=[10, 20, 30, 40, 50, 60], name="k")
s = pd.Series([1.0, 2.0, 6.0, 10.0, float("nan"), 30.0], index=[10, 20, 30, 40, 50, 60], name="v")
w = pd.Series([2.0, 4.0, 4.0, 5.0, 15.0, 25.0], index=[10, 20, 30, 40, 50, 60], name="w")
gb = s.groupby(k)
gb_w = w.groupby(k)

def clean_list(vals):
    out = []
    for x in vals:
        if x is None or (isinstance(x, float) and (math.isnan(x) or math.isinf(x))) or pd.isna(x):
            out.append(None)
        elif hasattr(x, "item"):
            out.append(float(x.item()))
        else:
            out.append(float(x))
    return out

kb = pd.Series(["a", "a", "b", "b"], name="k")
sb = pd.Series([True, False, True, True], name="flag")
gb_bool = sb.groupby(kb)

ohlc_df = gb.ohlc()
desc_df = gb.describe()

res = {
    "sum": clean_list(gb.sum().tolist()),
    "mean": clean_list(gb.mean().tolist()),
    "min": clean_list(gb.min().tolist()),
    "max": clean_list(gb.max().tolist()),
    "count": clean_list(gb.count().tolist()),
    "size": clean_list(gb.size().tolist()),
    "std": clean_list(gb.std().tolist()),
    "var": clean_list(gb.var().tolist()),
    "sem": clean_list(gb.sem().tolist()),
    "skew_w": clean_list(gb_w.skew().tolist()),
    "prod": clean_list(gb.prod().tolist()),
    "median": clean_list(gb.median().tolist()),
    "quantile": clean_list(gb.quantile(0.5).tolist()),
    "first": clean_list(gb.first().tolist()),
    "last": clean_list(gb.last().tolist()),
    "nth_1_w": clean_list(gb_w.nth(1).tolist()),
    "nunique_w": clean_list(gb_w.nunique().tolist()),
    "unique_w": clean_list(gb_w.unique().explode().tolist()),
    "idxmin": [int(x) for x in gb.idxmin().tolist()],
    "idxmax": [int(x) for x in gb.idxmax().tolist()],
    "all_flag": [bool(x) for x in gb_bool.all().tolist()],
    "any_flag": [bool(x) for x in gb_bool.any().tolist()],
    "agg_sum": clean_list(gb.agg(["sum", "mean"])["sum"].tolist()),
    "aggregate_mean": clean_list(gb.aggregate(["sum", "mean"])["mean"].tolist()),
    "ohlc_open": clean_list(ohlc_df["open"].tolist()),
    "ohlc_high": clean_list(ohlc_df["high"].tolist()),
    "ohlc_low": clean_list(ohlc_df["low"].tolist()),
    "ohlc_close": clean_list(ohlc_df["close"].tolist()),
    "describe_mean": clean_list(desc_df["mean"].tolist()),
    "describe_max": clean_list(desc_df["max"].tolist()),
    "vc_w": [int(x) for x in gb_w.value_counts().tolist()],
    "nlargest_2_w": clean_list(gb_w.nlargest(2).tolist()),
    "nsmallest_2_w": clean_list(gb_w.nsmallest(2).tolist()),
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        return;
    };

    let idx = Index::from_i64(vec![10, 20, 30, 40, 50, 60]);
    let k = Series::new(
        "k",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    )
    .unwrap();
    let s = Series::new(
        "v",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(6.0),
            Scalar::Float64(10.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(30.0),
        ])
        .unwrap(),
    )
    .unwrap();
    let w = Series::new(
        "w",
        idx,
        Column::from_values(vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
            Scalar::Float64(15.0),
            Scalar::Float64(25.0),
        ])
        .unwrap(),
    )
    .unwrap();

    let gb = s.groupby(&k).unwrap();
    let gb_w = w.groupby(&k).unwrap();

    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.sum().unwrap()),
        &oracle["sum"],
        "s_gb.sum",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.mean().unwrap()),
        &oracle["mean"],
        "s_gb.mean",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.min().unwrap()),
        &oracle["min"],
        "s_gb.min",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.max().unwrap()),
        &oracle["max"],
        "s_gb.max",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.count().unwrap()),
        &oracle["count"],
        "s_gb.count",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.size().unwrap()),
        &oracle["size"],
        "s_gb.size",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.std().unwrap()),
        &oracle["std"],
        "s_gb.std",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.var().unwrap()),
        &oracle["var"],
        "s_gb.var",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.sem().unwrap()),
        &oracle["sem"],
        "s_gb.sem",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb_w.skew().unwrap()),
        &oracle["skew_w"],
        "s_gb.skew",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.prod().unwrap()),
        &oracle["prod"],
        "s_gb.prod",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.median().unwrap()),
        &oracle["median"],
        "s_gb.median",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.quantile(0.5).unwrap()),
        &oracle["quantile"],
        "s_gb.quantile",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.first().unwrap()),
        &oracle["first"],
        "s_gb.first",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.last().unwrap()),
        &oracle["last"],
        "s_gb.last",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb_w.nth(1).unwrap()),
        &oracle["nth_1_w"],
        "s_gb.nth",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb_w.nunique().unwrap()),
        &oracle["nunique_w"],
        "s_gb.nunique",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb_w.unique().unwrap()),
        &oracle["unique_w"],
        "s_gb.unique",
    );

    let idxmin_vals: Vec<i64> = gb
        .idxmin()
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    let oracle_idxmin: Vec<i64> = oracle["idxmin"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    assert_eq!(idxmin_vals, oracle_idxmin);

    let idxmax_vals: Vec<i64> = gb
        .idxmax()
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    let oracle_idxmax: Vec<i64> = oracle["idxmax"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    assert_eq!(idxmax_vals, oracle_idxmax);

    let kb = Series::new(
        "k",
        Index::from_i64(vec![0, 1, 2, 3]),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    )
    .unwrap();
    let sb = Series::new(
        "flag",
        Index::from_i64(vec![0, 1, 2, 3]),
        Column::from_values(vec![
            Scalar::Bool(true),
            Scalar::Bool(false),
            Scalar::Bool(true),
            Scalar::Bool(true),
        ])
        .unwrap(),
    )
    .unwrap();
    let gb_bool = sb.groupby(&kb).unwrap();
    let all_vals: Vec<bool> = gb_bool
        .all()
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Bool(b) => *b,
            other => panic!("expected bool, got {other:?}"),
        })
        .collect();
    let any_vals: Vec<bool> = gb_bool
        .any()
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Bool(b) => *b,
            other => panic!("expected bool, got {other:?}"),
        })
        .collect();
    let oracle_all: Vec<bool> = oracle["all_flag"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_bool().unwrap())
        .collect();
    let oracle_any: Vec<bool> = oracle["any_flag"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_bool().unwrap())
        .collect();
    assert_eq!(all_vals, oracle_all);
    assert_eq!(any_vals, oracle_any);

    let agg_df = gb.agg(&["sum", "mean"]).unwrap();
    let aggregate_df = gb.aggregate(&["sum", "mean"]).unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&agg_df, "sum"),
        &oracle["agg_sum"],
        "s_gb.agg",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&aggregate_df, "mean"),
        &oracle["aggregate_mean"],
        "s_gb.aggregate",
    );

    let ohlc_df = gb.ohlc().unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&ohlc_df, "open"),
        &oracle["ohlc_open"],
        "s_gb.ohlc_open",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&ohlc_df, "high"),
        &oracle["ohlc_high"],
        "s_gb.ohlc_high",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&ohlc_df, "low"),
        &oracle["ohlc_low"],
        "s_gb.ohlc_low",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&ohlc_df, "close"),
        &oracle["ohlc_close"],
        "s_gb.ohlc_close",
    );

    let desc_df = gb.describe().unwrap();
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&desc_df, "mean"),
        &oracle["describe_mean"],
        "s_gb.describe_mean",
    );
    assert_opt_f64_slice_close(
        &df_col_f64_vec(&desc_df, "max"),
        &oracle["describe_max"],
        "s_gb.describe_max",
    );

    let vc_vals: Vec<i64> = gb_w
        .value_counts()
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    let oracle_vc: Vec<i64> = oracle["vc_w"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    assert_eq!(vc_vals, oracle_vc);

    assert_opt_f64_slice_close(
        &series_f64_vec(&gb_w.nlargest(2).unwrap()),
        &oracle["nlargest_2_w"],
        "s_gb.nlargest",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb_w.nsmallest(2).unwrap()),
        &oracle["nsmallest_2_w"],
        "s_gb.nsmallest",
    );
}

#[test]
fn live_oracle_series_groupby_cumulative_and_window_differential() {
    // @covers: SeriesGroupBy.cumsum, SeriesGroupBy.cumprod, SeriesGroupBy.cummin, SeriesGroupBy.cummax
    // @covers: SeriesGroupBy.cumcount, SeriesGroupBy.ngroup, SeriesGroupBy.rank
    // @covers: SeriesGroupBy.shift, SeriesGroupBy.diff, SeriesGroupBy.pct_change
    // @covers: SeriesGroupBy.ffill, SeriesGroupBy.bfill, SeriesGroupBy.fillna
    // @covers: SeriesGroupBy.head, SeriesGroupBy.tail, SeriesGroupBy.take, SeriesGroupBy.sample
    // @covers: SeriesGroupBy.rolling, SeriesGroupBy.expanding, SeriesGroupBy.ewm, SeriesGroupBy.resample
    use fp_columnar::Column;
    use fp_frame::Series;
    use fp_index::{Index, IndexLabel};
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, math, pandas as pd

k = pd.Series(["a", "a", "a", "b", "b", "b"], name="k")
s = pd.Series([2.0, 4.0, 8.0, 10.0, 20.0, 5.0], name="v")
na_s = pd.Series([1.0, float("nan"), 3.0, float("nan"), 5.0, float("nan")], name="na_col")
gb = s.groupby(k)
gb_na = na_s.groupby(k)

def clean_list(vals):
    out = []
    for x in vals:
        if x is None or (isinstance(x, float) and (math.isnan(x) or math.isinf(x))) or pd.isna(x):
            out.append(None)
        elif hasattr(x, "item"):
            out.append(float(x.item()))
        else:
            out.append(float(x))
    return out

ts_idx = pd.to_datetime([
    "2024-01-01 00:00:00",
    "2024-01-01 00:30:00",
    "2024-01-01 00:00:00",
    "2024-01-01 00:30:00",
])
ts_k = pd.Series(["a", "a", "b", "b"], index=ts_idx, name="k")
ts_s = pd.Series([1.0, 3.0, 10.0, 20.0], index=ts_idx, name="v")

res = {
    "cumsum": clean_list(gb.cumsum().tolist()),
    "cumprod": clean_list(gb.cumprod().tolist()),
    "cummin": clean_list(gb.cummin().tolist()),
    "cummax": clean_list(gb.cummax().tolist()),
    "cumcount": clean_list(gb.cumcount().tolist()),
    "ngroup": clean_list(gb.ngroup().tolist()),
    "rank": clean_list(gb.rank(method="average", ascending=True, na_option="keep").tolist()),
    "shift": clean_list(gb.shift(1).tolist()),
    "diff": clean_list(gb.diff(1).tolist()),
    "pct_change": clean_list(gb.pct_change(1).tolist()),
    "ffill": clean_list(gb_na.ffill().tolist()),
    "bfill": clean_list(gb_na.bfill().tolist()),
    "head_1": clean_list(gb.head(1).tolist()),
    "tail_1": clean_list(gb.tail(1).tolist()),
    "take_0_2": clean_list(gb.take([0, 2]).tolist()),
    "sample_1_len": int(len(gb.sample(n=1, random_state=42))),
    "rolling_2_sum": clean_list(gb.rolling(2).sum().tolist()),
    "expanding_sum": clean_list(gb.expanding(1).sum().tolist()),
    "ewm_mean": clean_list(gb.ewm(span=2.0).mean().tolist()),
    "resample_1d_sum": clean_list(ts_s.groupby(ts_k).resample("1D").sum().tolist()),
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        return;
    };

    let idx = Index::from_i64(vec![0, 1, 2, 3, 4, 5]);
    let k = Series::new(
        "k",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    )
    .unwrap();
    let s = Series::new(
        "v",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Float64(2.0),
            Scalar::Float64(4.0),
            Scalar::Float64(8.0),
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
            Scalar::Float64(5.0),
        ])
        .unwrap(),
    )
    .unwrap();
    let na_s = Series::new(
        "na_col",
        idx,
        Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(5.0),
            Scalar::Null(NullKind::NaN),
        ])
        .unwrap(),
    )
    .unwrap();

    let gb = s.groupby(&k).unwrap();
    let gb_na = na_s.groupby(&k).unwrap();

    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.cumsum().unwrap()),
        &oracle["cumsum"],
        "s_gb.cumsum",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.cumprod().unwrap()),
        &oracle["cumprod"],
        "s_gb.cumprod",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.cummin().unwrap()),
        &oracle["cummin"],
        "s_gb.cummin",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.cummax().unwrap()),
        &oracle["cummax"],
        "s_gb.cummax",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.cumcount().unwrap()),
        &oracle["cumcount"],
        "s_gb.cumcount",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.ngroup().unwrap()),
        &oracle["ngroup"],
        "s_gb.ngroup",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.rank("average", true, "keep").unwrap()),
        &oracle["rank"],
        "s_gb.rank",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.shift(1).unwrap()),
        &oracle["shift"],
        "s_gb.shift",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.diff(1).unwrap()),
        &oracle["diff"],
        "s_gb.diff",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.pct_change(1).unwrap()),
        &oracle["pct_change"],
        "s_gb.pct_change",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb_na.ffill(None).unwrap()),
        &oracle["ffill"],
        "s_gb.ffill",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb_na.bfill(None).unwrap()),
        &oracle["bfill"],
        "s_gb.bfill",
    );

    let filled = gb_na.fillna(&Scalar::Float64(99.0)).unwrap();
    assert_eq!(
        series_f64_vec(&filled),
        vec![
            Some(1.0),
            Some(99.0),
            Some(3.0),
            Some(99.0),
            Some(5.0),
            Some(99.0)
        ]
    );

    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.head(1).unwrap()),
        &oracle["head_1"],
        "s_gb.head",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.tail(1).unwrap()),
        &oracle["tail_1"],
        "s_gb.tail",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.take(&[0, 2]).unwrap()),
        &oracle["take_0_2"],
        "s_gb.take",
    );

    let sampled = gb.sample(Some(1), None, false, Some(42)).unwrap();
    assert_eq!(
        sampled.len(),
        oracle["sample_1_len"].as_u64().unwrap() as usize
    );

    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.rolling(2).sum().unwrap()),
        &oracle["rolling_2_sum"],
        "s_gb.rolling",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.expanding(Some(1)).sum().unwrap()),
        &oracle["expanding_sum"],
        "s_gb.expanding",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.ewm(Some(2.0), None).mean().unwrap()),
        &oracle["ewm_mean"],
        "s_gb.ewm",
    );

    let ts_idx = Index::new(vec![
        IndexLabel::Utf8("2024-01-01 00:00:00".into()),
        IndexLabel::Utf8("2024-01-01 00:30:00".into()),
        IndexLabel::Utf8("2024-01-01 00:00:00".into()),
        IndexLabel::Utf8("2024-01-01 00:30:00".into()),
    ]);
    let ts_k = Series::new(
        "k",
        ts_idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    )
    .unwrap();
    let ts_s = Series::new(
        "v",
        ts_idx,
        Column::from_values(vec![
            Scalar::Float64(1.0),
            Scalar::Float64(3.0),
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
        ])
        .unwrap(),
    )
    .unwrap();
    assert_opt_f64_slice_close(
        &series_f64_vec(&ts_s.groupby(&ts_k).unwrap().resample("1D").sum().unwrap()),
        &oracle["resample_1d_sum"],
        "s_gb.resample",
    );
}

#[test]
fn live_oracle_series_groupby_transform_filter_apply_pipe_stats_differential() {
    // @covers: SeriesGroupBy.transform, SeriesGroupBy.filter, SeriesGroupBy.apply, SeriesGroupBy.pipe
    // @covers: SeriesGroupBy.get_group, SeriesGroupBy.corr, SeriesGroupBy.cov
    // @covers: SeriesGroupBy.groups, SeriesGroupBy.indices, SeriesGroupBy.ngroups, SeriesGroupBy.ndim
    // @covers: SeriesGroupBy.dtype, SeriesGroupBy.keys, SeriesGroupBy.level, SeriesGroupBy.grouper
    // @covers: SeriesGroupBy.is_monotonic_increasing, SeriesGroupBy.is_monotonic_decreasing
    // @covers: SeriesGroupBy.plot, SeriesGroupBy.hist, SeriesGroupBy.boxplot
    use fp_columnar::Column;
    use fp_frame::{Series, ValueCountsOrder};
    use fp_index::{Index, IndexLabel};
    use fp_types::{DType, Scalar};

    let python_code = r#"
import json, math, pandas as pd

k = pd.Series(["a", "a", "b", "b"], name="k")
s = pd.Series([2.0, 6.0, 3.0, 1.0], name="x")
y = pd.Series([10.0, 30.0, 6.0, 2.0], name="y")
gb = s.groupby(k)

def clean_list(vals):
    out = []
    for x in vals:
        if x is None or (isinstance(x, float) and (math.isnan(x) or math.isinf(x))) or pd.isna(x):
            out.append(None)
        elif hasattr(x, "item"):
            out.append(float(x.item()))
        else:
            out.append(float(x))
    return out

cat_k = pd.Series(pd.Categorical(["a", "a", "b"], categories=["a", "b", "unused"]))
cat_v = pd.Series(["x", "y", "x"], name="val")
vc_obs_false = cat_v.groupby(cat_k, observed=False).value_counts().tolist()

neg_err = False
try:
    gb.get_group("nonexistent_key")
except KeyError:
    neg_err = True

neg_agg_err = False
try:
    gb.agg(["not_a_valid_pandas_agg"])
except AttributeError:
    neg_agg_err = True

res = {
    "transform_mean": clean_list(gb.transform("mean").tolist()),
    "filter": clean_list(gb.filter(lambda g: g.sum() > 5.0).tolist()),
    "apply": clean_list(gb.apply(lambda g: g * 2.0).tolist()),
    "pipe_sum": clean_list(gb.pipe(lambda g: g.sum()).tolist()),
    "get_group_a": clean_list(gb.get_group("a").tolist()),
    "corr": clean_list(gb.corr(y).tolist()),
    "cov": clean_list(gb.cov(y).tolist()),
    "is_mono_inc": [bool(x) for x in gb.is_monotonic_increasing.tolist()],
    "is_mono_dec": [bool(x) for x in gb.is_monotonic_decreasing.tolist()],
    "ngroups": int(gb.ngroups),
    "ndim": int(gb.ndim),
    "indices_a": [int(i) for i in gb.indices["a"]],
    "groups_b": [int(i) for i in gb.groups["b"]],
    "vc_obs_false": [int(x) for x in vc_obs_false],
    "neg_err": neg_err,
    "neg_agg_err": neg_agg_err,
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        return;
    };

    let idx = Index::from_i64(vec![0, 1, 2, 3]);
    let k = Series::new(
        "k",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("b".into()),
        ])
        .unwrap(),
    )
    .unwrap();
    let s = Series::new(
        "x",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Float64(2.0),
            Scalar::Float64(6.0),
            Scalar::Float64(3.0),
            Scalar::Float64(1.0),
        ])
        .unwrap(),
    )
    .unwrap();
    let y = Series::new(
        "y",
        idx,
        Column::from_values(vec![
            Scalar::Float64(10.0),
            Scalar::Float64(30.0),
            Scalar::Float64(6.0),
            Scalar::Float64(2.0),
        ])
        .unwrap(),
    )
    .unwrap();

    let gb = s.groupby(&k).unwrap();

    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.transform("mean").unwrap()),
        &oracle["transform_mean"],
        "s_gb.transform",
    );

    let filtered = gb
        .filter(|g| {
            let sum: f64 = g.values().iter().filter_map(|v| v.to_f64().ok()).sum();
            Ok(sum > 5.0)
        })
        .unwrap();
    assert_opt_f64_slice_close(&series_f64_vec(&filtered), &oracle["filter"], "s_gb.filter");

    let applied = gb
        .apply(|g| {
            let doubled: Vec<Scalar> = g
                .values()
                .iter()
                .map(|v| Scalar::Float64(v.to_f64().unwrap() * 2.0))
                .collect();
            Series::from_values(g.name(), g.index().labels().to_vec(), doubled)
        })
        .unwrap();
    assert_opt_f64_slice_close(&series_f64_vec(&applied), &oracle["apply"], "s_gb.apply");

    let piped = gb.pipe(|g| g.sum()).unwrap();
    assert_opt_f64_slice_close(&series_f64_vec(&piped), &oracle["pipe_sum"], "s_gb.pipe");

    let grp_a = gb.get_group("a").unwrap();
    assert_opt_f64_slice_close(
        &series_f64_vec(&grp_a),
        &oracle["get_group_a"],
        "s_gb.get_group",
    );

    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.corr(&y).unwrap()),
        &oracle["corr"],
        "s_gb.corr",
    );
    assert_opt_f64_slice_close(
        &series_f64_vec(&gb.cov(&y).unwrap()),
        &oracle["cov"],
        "s_gb.cov",
    );

    let mono_inc: Vec<bool> = gb
        .is_monotonic_increasing()
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Bool(b) => *b,
            other => panic!("expected bool, got {other:?}"),
        })
        .collect();
    let mono_dec: Vec<bool> = gb
        .is_monotonic_decreasing()
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Bool(b) => *b,
            other => panic!("expected bool, got {other:?}"),
        })
        .collect();
    let oracle_inc: Vec<bool> = oracle["is_mono_inc"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_bool().unwrap())
        .collect();
    let oracle_dec: Vec<bool> = oracle["is_mono_dec"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_bool().unwrap())
        .collect();
    assert_eq!(mono_inc, oracle_inc);
    assert_eq!(mono_dec, oracle_dec);

    assert_eq!(gb.ngroups(), oracle["ngroups"].as_u64().unwrap() as usize);
    assert_eq!(gb.ndim(), oracle["ndim"].as_u64().unwrap() as usize);
    assert_eq!(gb.dtype(), DType::Float64);
    assert_eq!(
        gb.keys(),
        vec![IndexLabel::Utf8("a".into()), IndexLabel::Utf8("b".into())]
    );
    assert_eq!(gb.grouper(), "k");
    assert_eq!(gb.level(), None);

    let indices = gb.indices();
    let oracle_idx_a: Vec<usize> = oracle["indices_a"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(indices[&IndexLabel::Utf8("a".into())], oracle_idx_a);

    let groups = gb.groups();
    let oracle_grp_b: Vec<usize> = oracle["groups_b"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(groups[&IndexLabel::Utf8("b".into())], oracle_grp_b);

    assert!(gb.plot().unwrap().to_svg().unwrap().contains("<svg"));
    assert!(gb.hist().unwrap().to_svg().unwrap().contains("<svg"));
    assert!(gb.boxplot().unwrap().to_svg().unwrap().contains("<svg"));

    // Categorical observed=False variant
    let cat_k = Series::from_categorical_codes(
        "k",
        vec![0, 0, 1],
        vec![
            Scalar::Utf8("a".into()),
            Scalar::Utf8("b".into()),
            Scalar::Utf8("unused".into()),
        ],
        false,
    )
    .unwrap();
    let cat_v = Series::new(
        "val",
        Index::from_i64(vec![0, 1, 2]),
        Column::from_values(vec![
            Scalar::Utf8("x".into()),
            Scalar::Utf8("y".into()),
            Scalar::Utf8("x".into()),
        ])
        .unwrap(),
    )
    .unwrap();
    let vc_obs = cat_v
        .groupby(&cat_k)
        .unwrap()
        .value_counts_over_categories(false, true, false, true, ValueCountsOrder::GroupsSorted)
        .unwrap();
    let vc_obs_vals: Vec<i64> = vc_obs
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    let oracle_vc_obs: Vec<i64> = oracle["vc_obs_false"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    assert_eq!(vc_obs_vals, oracle_vc_obs);

    // NEGATIVE cases: both pandas and FrankenPandas reject nonexistent group and invalid agg function
    assert!(oracle["neg_err"].as_bool().unwrap());
    assert!(gb.get_group("nonexistent_key").is_err());

    assert!(oracle["neg_agg_err"].as_bool().unwrap());
    assert!(gb.agg(&["not_a_valid_pandas_agg"]).is_err());
}
