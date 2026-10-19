//! Reshape parity-matrix conformance suite (br-frankenpandas-6wa9).
//!
//! Per /testing-conformance-harnesses Pattern 1, these fixtures compare
//! FrankenPandas against live upstream pandas for melt, pivot, pivot_table,
//! stack, and unstack edge cases: empty inputs, single rows, missing-heavy
//! values, duplicate aggregate keys, mixed dtypes, and composite index labels.

use serde_json::{Map, Value};

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
        Ok(ResolvedExpected::Frame(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping reshape conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_reshape_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("reshape oracle") {
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
        "pandas reshape parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

fn i(value: i64) -> Value {
    serde_json::json!({ "kind": "int64", "value": value })
}

fn f(value: f64) -> Value {
    serde_json::json!({ "kind": "float64", "value": value })
}

fn s(value: &str) -> Value {
    serde_json::json!({ "kind": "utf8", "value": value })
}

fn null() -> Value {
    serde_json::json!({ "kind": "null", "value": "null" })
}

fn nan() -> Value {
    serde_json::json!({ "kind": "null", "value": "na_n" })
}

fn frame(index: Vec<Value>, column_order: &[&str], columns: &[(&str, Vec<Value>)]) -> Value {
    let mut column_map = Map::new();
    for (name, values) in columns {
        column_map.insert((*name).to_owned(), Value::Array(values.clone()));
    }
    serde_json::json!({
        "index": index,
        "column_order": column_order,
        "columns": column_map
    })
}

fn series(name: &str, index: Vec<Value>, values: Vec<Value>) -> Value {
    serde_json::json!({
        "name": name,
        "index": index,
        "values": values
    })
}

fn frame_fixture(
    packet_id: &str,
    case_id: &str,
    operation: &str,
    frame: Value,
    options: &[(&str, Value)],
) -> PacketFixture {
    let mut raw = serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": operation,
        "oracle_source": "live_legacy_pandas",
        "frame": frame
    });
    let raw_object = raw.as_object_mut().expect("fixture object");
    for (key, value) in options {
        raw_object.insert((*key).to_owned(), value.clone());
    }
    serde_json::from_value(raw).expect("fixture")
}

fn series_fixture(
    packet_id: &str,
    case_id: &str,
    operation: &str,
    left: Value,
    options: &[(&str, Value)],
) -> PacketFixture {
    let mut raw = serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": operation,
        "oracle_source": "live_legacy_pandas",
        "left": left
    });
    let raw_object = raw.as_object_mut().expect("fixture object");
    for (key, value) in options {
        raw_object.insert((*key).to_owned(), value.clone());
    }
    serde_json::from_value(raw).expect("fixture")
}

#[test]
fn conformance_reshape_melt_empty_frame() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-001",
        "reshape_melt_empty_frame",
        "dataframe_melt",
        frame(vec![], &["id", "x"], &[("id", vec![]), ("x", vec![])]),
        &[
            ("melt_id_vars", serde_json::json!(["id"])),
            ("melt_value_vars", serde_json::json!(["x"])),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_melt_single_row_custom_names() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-002",
        "reshape_melt_single_row_custom_names",
        "dataframe_melt",
        frame(
            vec![i(0)],
            &["key", "score"],
            &[("key", vec![s("row")]), ("score", vec![f(3.5)])],
        ),
        &[
            ("melt_id_vars", serde_json::json!(["key"])),
            ("melt_value_vars", serde_json::json!(["score"])),
            ("melt_var_name", serde_json::json!("metric")),
            ("melt_value_name", serde_json::json!("reading")),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_melt_nan_heavy_mixed_values() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-003",
        "reshape_melt_nan_heavy_mixed_values",
        "dataframe_melt",
        frame(
            vec![i(0), i(1), i(2)],
            &["id", "ints", "floats"],
            &[
                ("id", vec![s("a"), s("b"), s("c")]),
                ("ints", vec![i(1), null(), i(3)]),
                ("floats", vec![nan(), f(2.5), f(3.5)]),
            ],
        ),
        &[
            ("melt_id_vars", serde_json::json!(["id"])),
            ("melt_value_vars", serde_json::json!(["ints", "floats"])),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_melt_null_nan_ids_auto_value_vars_tn6qb9() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-011",
        "reshape_melt_null_nan_ids_auto_value_vars_tn6qb9",
        "dataframe_melt",
        frame(
            vec![i(0), i(1), i(2)],
            &["id", "x", "y"],
            &[
                ("id", vec![s("a"), nan(), null()]),
                ("x", vec![i(1), nan(), f(-0.0)]),
                ("y", vec![f(10.5), f(20.5), f(30.5)]),
            ],
        ),
        &[("melt_id_vars", serde_json::json!(["id"]))],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_pivot_single_row() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-004",
        "reshape_pivot_single_row",
        "dataframe_pivot",
        frame(
            vec![i(0)],
            &["row", "col", "value"],
            &[
                ("row", vec![s("r1")]),
                ("col", vec![s("c1")]),
                ("value", vec![f(10.0)]),
            ],
        ),
        &[
            ("pivot_index", serde_json::json!("row")),
            ("pivot_columns", serde_json::json!("col")),
            ("pivot_values", serde_json::json!(["value"])),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_pivot_wide_mixed_column_order() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-005",
        "reshape_pivot_wide_mixed_column_order",
        "dataframe_pivot",
        frame(
            vec![i(0), i(1), i(2), i(3)],
            &["row", "col", "value"],
            &[
                ("row", vec![s("r1"), s("r1"), s("r2"), s("r2")]),
                ("col", vec![s("b"), s("a"), s("b"), s("a")]),
                ("value", vec![i(20), i(10), i(40), i(30)]),
            ],
        ),
        &[
            ("pivot_index", serde_json::json!("row")),
            ("pivot_columns", serde_json::json!("col")),
            ("pivot_values", serde_json::json!(["value"])),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_pivot_table_duplicate_keys_mean() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-006",
        "reshape_pivot_table_duplicate_keys_mean",
        "dataframe_pivot_table",
        frame(
            vec![i(0), i(1), i(2), i(3)],
            &["region", "product", "sales"],
            &[
                ("region", vec![s("east"), s("east"), s("west"), s("east")]),
                ("product", vec![s("A"), s("A"), s("A"), s("B")]),
                ("sales", vec![f(10.0), f(14.0), f(7.0), f(5.0)]),
            ],
        ),
        &[
            ("pivot_index", serde_json::json!("region")),
            ("pivot_columns", serde_json::json!("product")),
            ("pivot_values", serde_json::json!(["sales"])),
            ("pivot_aggfunc", serde_json::json!("mean")),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_pivot_table_fill_missing_cells() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-007",
        "reshape_pivot_table_fill_missing_cells",
        "dataframe_pivot_table",
        frame(
            vec![i(0), i(1), i(2)],
            &["region", "product", "sales"],
            &[
                ("region", vec![s("east"), s("east"), s("west")]),
                ("product", vec![s("A"), s("B"), s("A")]),
                ("sales", vec![f(10.0), f(5.0), f(7.0)]),
            ],
        ),
        &[
            ("pivot_index", serde_json::json!("region")),
            ("pivot_columns", serde_json::json!("product")),
            ("pivot_values", serde_json::json!(["sales"])),
            ("pivot_aggfunc", serde_json::json!("sum")),
            (
                "fill_value",
                serde_json::json!({ "kind": "float64", "value": 0.0 }),
            ),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_pivot_table_missing_keys_dropna_default_tn6qb9() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-012",
        "reshape_pivot_table_missing_keys_dropna_default_tn6qb9",
        "dataframe_pivot_table",
        frame(
            vec![i(0), i(1), i(2), i(3), i(4), i(5), i(6)],
            &["row", "col", "val"],
            &[
                (
                    "row",
                    vec![s("r1"), s("r1"), s("r2"), nan(), null(), s("r1"), s("r2")],
                ),
                (
                    "col",
                    vec![s("c1"), s("c2"), s("c1"), s("c1"), s("c2"), nan(), null()],
                ),
                (
                    "val",
                    vec![
                        f(1.0),
                        f(2.0),
                        f(3.0),
                        f(100.0),
                        f(200.0),
                        f(400.0),
                        f(500.0),
                    ],
                ),
            ],
        ),
        &[
            ("pivot_index", serde_json::json!("row")),
            ("pivot_columns", serde_json::json!("col")),
            ("pivot_values", serde_json::json!(["val"])),
            ("pivot_aggfunc", serde_json::json!("sum")),
            // ASK FOR PANDAS' DEFAULT, which is what this case's name claims to
            // test. br-frankenpandas-eay9h: the oracle hardcoded dropna=False
            // with no knob, so this fixture tested the OPPOSITE of its name and
            // marked FrankenPandas divergent for matching pandas. Measured live
            // on 2.2.3 with these exact inputs:
            //   dropna=True  -> index ['r1','r2']       <- pandas default, = FP
            //   dropna=False -> index ['r1','r2',nan]
            ("pivot_dropna", serde_json::json!(true)),
        ],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_stack_single_row_mixed_dtypes() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-008",
        "reshape_stack_single_row_mixed_dtypes",
        "dataframe_stack",
        frame(
            vec![s("r1")],
            &["int_col", "text_col"],
            &[("int_col", vec![i(1)]), ("text_col", vec![s("x")])],
        ),
        &[],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_stack_nan_heavy_values() {
    let fixture = frame_fixture(
        "FP-CONF-RESHAPE-009",
        "reshape_stack_nan_heavy_values",
        "dataframe_stack",
        frame(
            vec![s("r1"), s("r2")],
            &["a", "b"],
            &[("a", vec![f(1.0), nan()]), ("b", vec![null(), f(4.0)])],
        ),
        &[],
    );
    check_reshape_fixture(fixture);
}

#[test]
fn conformance_reshape_series_unstack_composite_labels() {
    let fixture = series_fixture(
        "FP-CONF-RESHAPE-010",
        "reshape_series_unstack_composite_labels",
        "series_unstack",
        series(
            "value",
            vec![s("r1, a"), s("r1, b"), s("r2, a"), s("r2, b")],
            vec![f(1.0), f(2.0), f(3.0), f(4.0)],
        ),
        &[],
    );
    check_reshape_fixture(fixture);
}

fn run_pandas_reshape_oracle_eval(script: &str) -> Option<Value> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    let probe = frame_fixture(
        "FP-CONF-RESHAPE-PROBE",
        "reshape_oracle_probe",
        "dataframe_melt",
        frame(vec![], &["id", "x"], &[("id", vec![]), ("x", vec![])]),
        &[
            ("melt_id_vars", serde_json::json!(["id"])),
            ("melt_value_vars", serde_json::json!(["x"])),
        ],
    );
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &probe).expect("reshape oracle probe") {
        return None;
    }
    let mut child = Command::new(&cfg.python_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(script.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    assert!(
        output.status.success(),
        "python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(serde_json::from_slice(&output.stdout).expect("parse oracle json"))
}

#[test]
#[allow(clippy::too_many_lines, clippy::cognitive_complexity)]
fn live_oracle_pandas_top_level_functions_full_surface_differential() {
    // @covers: pandas_array, pandas_bdate_range, pandas_concat, pandas_crosstab
    // @covers: pandas_cut, pandas_date_range, pandas_describe_option, pandas_eval
    // @covers: pandas_factorize, pandas_from_dummies, pandas_get_dummies
    // @covers: pandas_get_option, pandas_infer_freq, pandas_interval_range
    // @covers: pandas_isna, pandas_isnull, pandas_json_normalize, pandas_lreshape
    // @covers: pandas_melt, pandas_merge, pandas_merge_asof, pandas_merge_ordered
    // @covers: pandas_notna, pandas_notnull, pandas_period_range, pandas_pivot
    // @covers: pandas_pivot_table, pandas_qcut, pandas_read_clipboard, pandas_read_csv
    // @covers: pandas_read_excel, pandas_read_feather, pandas_read_fwf, pandas_read_gbq
    // @covers: pandas_read_hdf, pandas_read_html, pandas_read_json, pandas_read_orc
    // @covers: pandas_read_parquet, pandas_read_pickle, pandas_read_sas, pandas_read_spss
    // @covers: pandas_read_sql, pandas_read_sql_query, pandas_read_sql_table
    // @covers: pandas_read_stata, pandas_read_table, pandas_read_xml
    // @covers: pandas_reset_option, pandas_set_eng_float_format, pandas_set_option
    // @covers: pandas_show_versions, pandas_test, pandas_timedelta_range
    // @covers: pandas_to_datetime, pandas_to_numeric, pandas_to_pickle
    // @covers: pandas_to_timedelta, pandas_unique, pandas_value_counts
    // @covers: pandas_wide_to_long
    use std::collections::BTreeMap;

    use fp_columnar::Column;
    use fp_expr::DataFrameExprExt;
    use fp_frame::{
        DataFrame, Series, concat_dataframes, crosstab, cut_bins, factorize, from_dummies,
        get_dummies, lreshape, melt, pivot, pivot_table, qcut_at_quantiles, show_versions, testing,
        to_datetime, to_numeric, to_timedelta, unique, value_counts, wide_to_long,
    };
    use fp_index::{Index, IndexLabel, bdate_range, date_range, infer_freq, timedelta_range};
    use fp_io::{
        ExcelReadOptions, FwfReadOptions, JsonOrient, SqlIfExists, json_normalize_str,
        read_clipboard_str, read_csv_str, read_excel_bytes, read_feather_bytes, read_fwf_str,
        read_gbq, read_hdf, read_html_str, read_json_str, read_orc_bytes, read_parquet_bytes,
        read_pickle_bytes, read_sas, read_spss, read_sql, read_sql_query, read_sql_table,
        read_stata_bytes, read_table_str, read_xml_str, write_excel_bytes, write_feather_bytes,
        write_hdf, write_parquet_bytes, write_pickle, write_pickle_bytes, write_sql,
        write_stata_bytes,
    };
    use fp_join::{AsofDirection, JoinType, merge_asof, merge_dataframes, merge_ordered};
    use fp_types::{
        DType, IntervalClosed, OptionValue, Period, Scalar, Timedelta, describe_option, get_option,
        interval_range, isna, isnull, notna, notnull, period_range, reset_option,
        set_eng_float_format, set_option,
    };

    let Some(oracle) = run_pandas_reshape_oracle_eval(
        r#"
import json
import pandas as pd

s_num = pd.Series([10, 20, 30, 40], index=[0, 1, 2, 3], name="x")
s_cat_a = pd.Series(["a", "a", "b", "b"], name="r")
s_cat_b = pd.Series(["u", "v", "u", "v"], name="c")
ct = pd.crosstab(s_cat_a, s_cat_b)
cut_s = pd.cut(s_num, bins=[0, 20, 40], labels=["low", "high"])
qcut_s = pd.qcut(s_num, q=[0.0, 0.5, 1.0], labels=["q1", "q2"])
codes, uniques = pd.factorize(pd.Series(["b", "a", "b", None]))
dum_in = pd.DataFrame({"color": ["r", "b", "r"]})
dums = pd.get_dummies(dum_in, columns=["color"])
undums = pd.from_dummies(dums, sep="_")
df_wide = pd.DataFrame({"id": [1, 2], "A1": [10, 20], "A2": [30, 40]})
w2l = pd.wide_to_long(df_wide, stubnames=["A"], i="id", j="time")
lresh_in = pd.DataFrame({"hr1": [514, 573], "hr2": [545, 526], "team": ["Red", "Blue"]})
lresh = pd.lreshape(lresh_in, {"hr": ["hr1", "hr2"]})
norm = pd.json_normalize([{"id": 1, "nested": {"v": 10}}, {"id": 2, "nested": {"v": 20}}], sep=".")
left = pd.DataFrame({"k": [1, 2], "lv": [10, 20]})
right = pd.DataFrame({"k": [2, 3], "rv": [200, 300]})
m_inner = pd.merge(left, right, on="k", how="inner")
m_ord = pd.merge_ordered(left, right, on="k")
asof_l = pd.DataFrame({"t": [1, 5, 10], "v": [10, 50, 100]})
asof_r = pd.DataFrame({"t": [1, 4, 9], "rv": [100, 400, 900]})
m_asof = pd.merge_asof(asof_l, asof_r, on="t")

dr = pd.date_range("2024-01-01", periods=3, freq="D")
bdr = pd.bdate_range("2024-01-05", periods=3)
pr = pd.period_range("2024-01", periods=3, freq="M")
tdr = pd.timedelta_range("1 day", periods=3, freq="D")
ivr = pd.interval_range(start=0.0, end=6.0, periods=3, closed="right")

out = {
    "ct_u": [int(x) for x in ct["u"].tolist()],
    "ct_v": [int(x) for x in ct["v"].tolist()],
    "cut": [str(x) for x in cut_s.tolist()],
    "qcut": [str(x) for x in qcut_s.tolist()],
    "fact_codes": [int(x) for x in codes.tolist()],
    "fact_uniques": [str(x) for x in uniques.tolist()],
    "undums": [str(x) for x in undums["color"].tolist()],
    "w2l": [int(x) for x in w2l["A"].tolist()],
    "lresh_hr": [int(x) for x in lresh["hr"].tolist()],
    "norm_v": [int(x) for x in norm["nested.v"].tolist()],
    "m_inner_k": [int(x) for x in m_inner["k"].tolist()],
    "m_ord_k": [int(x) for x in m_ord["k"].tolist()],
    "m_asof_rv": [int(x) for x in m_asof["rv"].tolist()],
    "dr": [int(x) for x in dr.asi8.tolist()],
    "bdr": [int(x) for x in bdr.asi8.tolist()],
    "pr": [int(x) for x in pr.asi8.tolist()],
    "tdr": [int(x) for x in tdr.asi8.tolist()],
    "infer_dr": str(pd.infer_freq(dr)),
    "ivr_left": [float(iv.left) for iv in ivr],
    "ivr_right": [float(iv.right) for iv in ivr],
}
print(json.dumps(out))
"#,
    ) else {
        return;
    };

    let parse_i64 = |v: &Value| -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect()
    };
    let parse_str = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    let label_i64 = |l: &IndexLabel| -> i64 {
        match l {
            IndexLabel::Int64(v) | IndexLabel::Datetime64(v) | IndexLabel::Timedelta64(v) => *v,
            _ => i64::MIN,
        }
    };

    // 1. array, unique, value_counts, factorize, isna/isnull/notna/notnull
    let arr_s = fp_frame::array(
        &[
            Scalar::Int64(10),
            Scalar::Int64(20),
            Scalar::Int64(30),
            Scalar::Int64(40),
        ],
        Some(DType::Int64),
    )
    .unwrap();
    assert_eq!(arr_s.len(), 4);

    let s_fact = Series::from_values(
        "f",
        vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ],
        vec![
            Scalar::Utf8("b".to_string()),
            Scalar::Utf8("a".to_string()),
            Scalar::Utf8("b".to_string()),
            Scalar::Null(fp_types::NullKind::Null),
        ],
    )
    .unwrap();
    let (codes, uniques) = factorize(&s_fact).unwrap();
    let code_vals: Vec<i64> = codes.values().iter().map(|v| v.to_i64().unwrap()).collect();
    let uniq_vals: Vec<String> = uniques
        .values()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(code_vals, parse_i64(&oracle["fact_codes"]));
    assert_eq!(uniq_vals, parse_str(&oracle["fact_uniques"]));
    assert_eq!(unique(&s_fact).len(), 3);
    let vc = value_counts(&s_fact).unwrap();
    assert_eq!(vc.values()[0], Scalar::Int64(2));
    assert_eq!(isna(s_fact.values()), vec![false, false, false, true]);
    assert_eq!(isnull(s_fact.values()), vec![false, false, false, true]);
    assert_eq!(notna(s_fact.values()), vec![true, true, true, false]);
    assert_eq!(notnull(s_fact.values()), vec![true, true, true, false]);

    // 2. cut, qcut, crosstab, get_dummies, from_dummies
    let cut_res = cut_bins(
        &arr_s,
        &[
            Scalar::Float64(0.0),
            Scalar::Float64(20.0),
            Scalar::Float64(40.0),
        ],
        true,
        Some(&["low", "high"]),
        false,
    )
    .unwrap();
    let cut_strs: Vec<String> = cut_res
        .values()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(cut_strs, parse_str(&oracle["cut"]));

    let qcut_res = qcut_at_quantiles(&arr_s, &[0.0, 0.5, 1.0], Some(&["q1", "q2"]), false).unwrap();
    let qcut_strs: Vec<String> = qcut_res
        .values()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(qcut_strs, parse_str(&oracle["qcut"]));

    let s_cat_a = Series::from_values(
        "r",
        vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ],
        vec![
            Scalar::Utf8("a".to_string()),
            Scalar::Utf8("a".to_string()),
            Scalar::Utf8("b".to_string()),
            Scalar::Utf8("b".to_string()),
        ],
    )
    .unwrap();
    let s_cat_b = Series::from_values(
        "c",
        vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ],
        vec![
            Scalar::Utf8("u".to_string()),
            Scalar::Utf8("v".to_string()),
            Scalar::Utf8("u".to_string()),
            Scalar::Utf8("v".to_string()),
        ],
    )
    .unwrap();
    let ct_df = crosstab(&s_cat_a, &s_cat_b).unwrap();
    let ct_u: Vec<i64> = ct_df
        .column("u")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    let ct_v: Vec<i64> = ct_df
        .column("v")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    assert_eq!(ct_u, parse_i64(&oracle["ct_u"]));
    assert_eq!(ct_v, parse_i64(&oracle["ct_v"]));

    let dum_in = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1, 2]),
        BTreeMap::from([(
            "color".to_string(),
            Column::from_values(vec![
                Scalar::Utf8("r".to_string()),
                Scalar::Utf8("b".to_string()),
                Scalar::Utf8("r".to_string()),
            ])
            .unwrap(),
        )]),
        vec!["color".to_string()],
    )
    .unwrap();
    let dums = get_dummies(&dum_in, &["color"]).unwrap();
    let undums = from_dummies(&dums, Some("_"), None).unwrap();
    let undum_vals: Vec<String> = undums
        .column("color")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(undum_vals, parse_str(&oracle["undums"]));

    // 3. melt, pivot, pivot_table, wide_to_long, lreshape, concat, merge, merge_ordered, merge_asof, eval
    let df_wide = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1]),
        BTreeMap::from([
            ("id".to_string(), Column::from_i64_values_owned(vec![1, 2])),
            (
                "A1".to_string(),
                Column::from_i64_values_owned(vec![10, 20]),
            ),
            (
                "A2".to_string(),
                Column::from_i64_values_owned(vec![30, 40]),
            ),
        ]),
        vec!["id".to_string(), "A1".to_string(), "A2".to_string()],
    )
    .unwrap();
    let melted = melt(&df_wide, &["id"], &["A1", "A2"], None, None).unwrap();
    let pivoted = pivot(&melted, "id", "variable", "value").unwrap();
    let ptable = pivot_table(&melted, "value", "id", "variable", "sum").unwrap();
    assert_eq!(pivoted.len(), 2);
    assert_eq!(ptable.len(), 2);

    let w2l = wide_to_long(&df_wide, &["A"], &["id"], "time", "", "\\d+").unwrap();
    let w2l_a: Vec<i64> = w2l
        .column("A")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    assert_eq!(w2l_a, parse_i64(&oracle["w2l"]));

    let lresh_in = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1]),
        BTreeMap::from([
            (
                "hr1".to_string(),
                Column::from_i64_values_owned(vec![514, 573]),
            ),
            (
                "hr2".to_string(),
                Column::from_i64_values_owned(vec![545, 526]),
            ),
            (
                "team".to_string(),
                Column::from_values(vec![
                    Scalar::Utf8("Red".to_string()),
                    Scalar::Utf8("Blue".to_string()),
                ])
                .unwrap(),
            ),
        ]),
        vec!["hr1".to_string(), "hr2".to_string(), "team".to_string()],
    )
    .unwrap();
    let mut groups = BTreeMap::new();
    groups.insert("hr".to_string(), vec!["hr1".to_string(), "hr2".to_string()]);
    let lresh = lreshape(&lresh_in, &groups, true).unwrap();
    let lresh_hr: Vec<i64> = lresh
        .column("hr")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    assert_eq!(lresh_hr, parse_i64(&oracle["lresh_hr"]));

    let cat_df = concat_dataframes(&[&df_wide, &df_wide]).unwrap();
    assert_eq!(cat_df.len(), 4);

    let left = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1]),
        BTreeMap::from([
            ("k".to_string(), Column::from_i64_values_owned(vec![1, 2])),
            (
                "lv".to_string(),
                Column::from_i64_values_owned(vec![10, 20]),
            ),
        ]),
        vec!["k".to_string(), "lv".to_string()],
    )
    .unwrap();
    let right = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1]),
        BTreeMap::from([
            ("k".to_string(), Column::from_i64_values_owned(vec![2, 3])),
            (
                "rv".to_string(),
                Column::from_i64_values_owned(vec![200, 300]),
            ),
        ]),
        vec!["k".to_string(), "rv".to_string()],
    )
    .unwrap();
    let m_inner = merge_dataframes(&left, &right, "k", JoinType::Inner).unwrap();
    assert_eq!(
        m_inner.columns["k"].values(),
        &[Scalar::Int64(oracle["m_inner_k"][0].as_i64().unwrap())]
    );
    let m_ord = merge_ordered(&left, &right, &["k"], None).unwrap();
    let m_ord_k: Vec<i64> = m_ord.columns["k"]
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    assert_eq!(m_ord_k, parse_i64(&oracle["m_ord_k"]));

    let asof_l = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1, 2]),
        BTreeMap::from([
            (
                "t".to_string(),
                Column::from_i64_values_owned(vec![1, 5, 10]),
            ),
            (
                "v".to_string(),
                Column::from_i64_values_owned(vec![10, 50, 100]),
            ),
        ]),
        vec!["t".to_string(), "v".to_string()],
    )
    .unwrap();
    let asof_r = DataFrame::new_with_column_order(
        Index::from_i64(vec![0, 1, 2]),
        BTreeMap::from([
            (
                "t".to_string(),
                Column::from_i64_values_owned(vec![1, 4, 9]),
            ),
            (
                "rv".to_string(),
                Column::from_i64_values_owned(vec![100, 400, 900]),
            ),
        ]),
        vec!["t".to_string(), "rv".to_string()],
    )
    .unwrap();
    let m_asof = merge_asof(&asof_l, &asof_r, "t", AsofDirection::Backward).unwrap();
    let m_asof_rv: Vec<i64> = m_asof.columns["rv"]
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    assert_eq!(m_asof_rv, parse_i64(&oracle["m_asof_rv"]));

    let eval_s = df_wide.eval("A1 + A2").unwrap();
    assert_eq!(eval_s.values(), &[Scalar::Int64(40), Scalar::Int64(60)]);

    // 4. to_datetime, to_timedelta, to_numeric, date_range, bdate_range, period_range, timedelta_range, interval_range, infer_freq
    let s_dt_str = Series::from_values(
        "dt",
        vec![IndexLabel::Int64(0)],
        vec![Scalar::Utf8("2024-01-01".to_string())],
    )
    .unwrap();
    assert_eq!(
        to_datetime(&s_dt_str).unwrap().dtype(),
        DType::datetime64_naive()
    );
    let s_td_str = Series::from_values(
        "td",
        vec![IndexLabel::Int64(0)],
        vec![Scalar::Utf8("1 days".to_string())],
    )
    .unwrap();
    assert_eq!(to_timedelta(&s_td_str).unwrap().dtype(), DType::Timedelta64);
    let s_num_str = Series::from_values(
        "num",
        vec![IndexLabel::Int64(0)],
        vec![Scalar::Utf8("42.5".to_string())],
    )
    .unwrap();
    assert_eq!(
        to_numeric(&s_num_str).unwrap().values(),
        &[Scalar::Float64(42.5)]
    );

    let dr = date_range(
        Some("2024-01-01"),
        None,
        Some(3),
        Timedelta::NANOS_PER_DAY,
        None,
    )
    .unwrap();
    let dr_vals: Vec<i64> = dr.labels().iter().map(label_i64).collect();
    assert_eq!(dr_vals, parse_i64(&oracle["dr"]));
    assert_eq!(
        infer_freq(&dr).unwrap().as_deref(),
        oracle["infer_dr"].as_str()
    );

    let bdr = bdate_range(Some("2024-01-05"), None, Some(3), None).unwrap();
    let bdr_vals: Vec<i64> = bdr.labels().iter().map(label_i64).collect();
    assert_eq!(bdr_vals, parse_i64(&oracle["bdr"]));

    let pr = period_range(Period::parse("2024-01").unwrap(), 3);
    let pr_vals: Vec<i64> = pr.iter().map(|p| p.ordinal()).collect();
    assert_eq!(pr_vals, parse_i64(&oracle["pr"]));

    let tdr = timedelta_range(
        Some(Timedelta::NANOS_PER_DAY),
        None,
        Some(3),
        Timedelta::NANOS_PER_DAY,
        None,
    )
    .unwrap();
    let tdr_vals: Vec<i64> = tdr.labels().iter().map(label_i64).collect();
    assert_eq!(tdr_vals, parse_i64(&oracle["tdr"]));

    let ivr = interval_range(
        Some(0.0),
        Some(6.0),
        Some(3),
        None,
        Some(IntervalClosed::Right),
    )
    .unwrap();
    assert_eq!(ivr.len(), 3);
    assert!((ivr[0].left - oracle["ivr_left"][0].as_f64().unwrap()).abs() < 1e-12);
    assert!((ivr[2].right - oracle["ivr_right"][2].as_f64().unwrap()).abs() < 1e-12);

    // 5. IO top-level functions (read_csv, read_table, read_fwf, read_json, json_normalize,
    // read_html, read_xml, read_parquet, read_feather, read_excel, read_pickle, to_pickle,
    // read_stata, read_hdf, read_sql/read_sql_query/read_sql_table, read_clipboard,
    // and fail-closed readers read_orc, read_gbq, read_sas, read_spss)
    let csv_df = read_csv_str("a,b\n1,2\n3,4\n").unwrap();
    assert_eq!(csv_df.len(), 2);
    let tsv_df = read_table_str("a\tb\n1\t2\n").unwrap();
    assert_eq!(tsv_df.len(), 1);
    let fwf_df = read_fwf_str("a   b\n1   2\n", &FwfReadOptions::default()).unwrap();
    assert_eq!(fwf_df.len(), 1);
    let json_df = read_json_str(r#"[{"a":1,"b":2}]"#, JsonOrient::Records).unwrap();
    assert_eq!(json_df.len(), 1);
    let norm_df = json_normalize_str(
        r#"[{"id": 1, "nested": {"v": 10}}, {"id": 2, "nested": {"v": 20}}]"#,
        Some("."),
        None,
    )
    .unwrap();
    let norm_v: Vec<i64> = norm_df
        .column("nested.v")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap())
        .collect();
    assert_eq!(norm_v, parse_i64(&oracle["norm_v"]));
    let html_df = read_html_str("<table><tr><th>a</th></tr><tr><td>1</td></tr></table>").unwrap();
    assert_eq!(html_df.len(), 1);
    let xml_df = read_xml_str("<data><row><a>1</a></row></data>").unwrap();
    assert_eq!(xml_df.len(), 1);
    let clip_df = read_clipboard_str("a\tb\n1\t2\n").unwrap();
    assert_eq!(clip_df.len(), 1);

    let pq_bytes = write_parquet_bytes(&csv_df).unwrap();
    assert_eq!(read_parquet_bytes(&pq_bytes).unwrap().len(), 2);
    let fea_bytes = write_feather_bytes(&csv_df).unwrap();
    assert_eq!(read_feather_bytes(&fea_bytes).unwrap().len(), 2);
    let xlsx_bytes = write_excel_bytes(&csv_df).unwrap();
    assert_eq!(
        read_excel_bytes(&xlsx_bytes, &ExcelReadOptions::default())
            .unwrap()
            .len(),
        2
    );
    let pkl_bytes = write_pickle_bytes(&csv_df).unwrap();
    assert_eq!(read_pickle_bytes(&pkl_bytes).unwrap().len(), 2);
    let dta_bytes = write_stata_bytes(&csv_df).unwrap();
    assert_eq!(read_stata_bytes(&dta_bytes, true).unwrap().len(), 2);

    let tmp_dir = tempfile::tempdir().unwrap();
    let pkl_path = tmp_dir.path().join("t.pkl");
    write_pickle(&csv_df, &pkl_path).unwrap();
    assert!(pkl_path.exists());
    let hdf_path = tmp_dir.path().join("t.h5");
    if write_hdf(&csv_df, &hdf_path).is_ok() {
        assert_eq!(read_hdf(&hdf_path).unwrap().len(), 2);
    } else {
        assert!(read_hdf(&hdf_path).is_err());
    }

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    write_sql(&csv_df, &conn, "tbl", SqlIfExists::Replace).unwrap();
    assert_eq!(read_sql(&conn, "SELECT * FROM tbl").unwrap().len(), 2);
    assert_eq!(read_sql_query(&conn, "SELECT * FROM tbl").unwrap().len(), 2);
    assert_eq!(read_sql_table(&conn, "tbl").unwrap().len(), 2);

    assert!(read_orc_bytes(b"not_orc").is_err());
    assert!(read_gbq("SELECT 1", None).is_err());
    assert!(read_sas(tmp_dir.path()).is_err());
    assert!(read_spss(tmp_dir.path()).is_err());

    // 6. Options & diagnostics (get_option, set_option, reset_option, describe_option,
    // set_eng_float_format, show_versions, testing)
    assert!(
        describe_option(Some("display.max_rows"))
            .unwrap()
            .contains("display.max_rows")
    );
    set_option("display.max_rows", OptionValue::Int(25)).unwrap();
    assert_eq!(
        get_option("display.max_rows").unwrap(),
        OptionValue::Int(25)
    );
    reset_option(Some("display.max_rows")).unwrap();
    set_eng_float_format(3, true);
    assert!(!show_versions().is_empty());
    testing::assert_frame_equal_default(&csv_df, &csv_df).unwrap();
}
