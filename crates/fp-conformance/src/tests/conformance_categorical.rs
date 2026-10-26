//! Categorical parity-matrix conformance suite (br-frankenpandas-vic4).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares
//! `pd.Categorical.from_codes` materialization behavior with live upstream
//! pandas for edge-case inputs: empty categoricals, single values, repeated
//! categories, missing codes, ordered categories, unused categories, numeric
//! categories, and mixed label categories.

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
                "live pandas unavailable; skipping Categorical conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_categorical_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("categorical oracle") {
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
        "pandas Categorical parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

fn scalar_utf8(value: &str) -> serde_json::Value {
    serde_json::json!({ "kind": "utf8", "value": value })
}

fn scalar_int(value: i64) -> serde_json::Value {
    serde_json::json!({ "kind": "int64", "value": value })
}

fn scalar_bool(value: bool) -> serde_json::Value {
    serde_json::json!({ "kind": "bool", "value": value })
}

fn categorical_fixture(
    packet_id: &str,
    case_id: &str,
    codes: &[i64],
    categories: Vec<serde_json::Value>,
    ordered: bool,
) -> PacketFixture {
    let index = (0..codes.len())
        .map(|idx| serde_json::json!({ "kind": "int64", "value": idx as i64 }))
        .collect::<Vec<_>>();
    let values = codes
        .iter()
        .map(|code| serde_json::json!({ "kind": "int64", "value": *code }))
        .collect::<Vec<_>>();

    serde_json::from_value(serde_json::json!({
        "packet_id": packet_id,
        "case_id": case_id,
        "mode": "strict",
        "operation": "series_categorical_from_codes",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "cat",
            "index": index,
            "values": values
        },
        "categorical_categories": categories,
        "categorical_ordered": ordered
    }))
    .expect("fixture")
}

#[test]
fn conformance_categorical_empty_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-001",
        "categorical_empty_categories",
        &[],
        Vec::new(),
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_single_value() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-002",
        "categorical_single_value",
        &[0],
        vec![scalar_utf8("only")],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_repeated_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-003",
        "categorical_repeated_categories",
        &[0, 1, 0, 2, 1],
        vec![
            scalar_utf8("red"),
            scalar_utf8("green"),
            scalar_utf8("blue"),
        ],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_missing_codes() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-004",
        "categorical_missing_codes",
        &[0, -1, 1, -1],
        vec![scalar_utf8("seen"), scalar_utf8("other")],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_ordered_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-005",
        "categorical_ordered_categories",
        &[2, 0, 1, 2],
        vec![
            scalar_utf8("low"),
            scalar_utf8("medium"),
            scalar_utf8("high"),
        ],
        true,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_unused_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-006",
        "categorical_unused_categories",
        &[0, 0, 2],
        vec![
            scalar_utf8("active"),
            scalar_utf8("unused"),
            scalar_utf8("also_active"),
        ],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_numeric_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-007",
        "categorical_numeric_categories",
        &[1, 0, 1, -1],
        vec![scalar_int(10), scalar_int(20)],
        false,
    );
    check_categorical_fixture(fixture);
}

#[test]
fn conformance_categorical_mixed_label_categories() {
    let fixture = categorical_fixture(
        "FP-CONF-CATEGORICAL-008",
        "categorical_mixed_label_categories",
        &[0, 1, 2, -1, 0],
        vec![scalar_utf8("x"), scalar_int(7), scalar_bool(true)],
        false,
    );
    check_categorical_fixture(fixture);
}

fn run_pandas_accessor_oracle_eval(script: &str) -> Option<serde_json::Value> {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    let cfg = strict_config();
    let guarded_script = format!(
        "import pandas as _pd, sys as _sys\nif _pd.__version__ != '2.2.3':\n    print('\"__SKIP_VERSION__\"')\n    _sys.exit(0)\n{script}"
    );
    let mut child = match Command::new(&cfg.python_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("skipping live accessor oracle test (spawn failed: {e})");
            return None;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(guarded_script.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("No module named 'pandas'") {
            return None;
        }
        assert!(
            output.status.success(),
            "pandas accessor oracle script failed (status={}):\nstderr={stderr}\nstdout={}",
            output.status,
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let val: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid accessor oracle json");
    if val.as_str() == Some("__SKIP_VERSION__") {
        return None;
    }
    Some(val)
}

#[test]
fn live_oracle_series_cat_accessor_full_surface_differential() {
    use fp_frame::Series;
    use fp_types::Scalar;

    // @covers: series_cat_add_categories
    // @covers: series_cat_as_ordered
    // @covers: series_cat_as_unordered
    // @covers: series_cat_categories
    // @covers: series_cat_codes
    // @covers: series_cat_ordered
    // @covers: series_cat_remove_categories
    // @covers: series_cat_remove_unused_categories
    // @covers: series_cat_rename_categories
    // @covers: series_cat_reorder_categories
    // @covers: series_cat_set_categories

    let python_code = r#"
import pandas as pd, json

s = pd.Series(
    pd.Categorical.from_codes([0, -1, 2, 0], categories=["low", "unused", "high"], ordered=False),
    name="grade",
)
s_ord = s.cat.as_ordered()
s_unord = s_ord.cat.as_unordered()
s_added = s.cat.add_categories(["extra"])
s_unused_rm = s.cat.remove_unused_categories()
s_rm = s_ord.cat.remove_categories(["unused"])
s_renamed = s.cat.rename_categories(["L", "U", "H"])
s_reordered = s.cat.reorder_categories(["high", "low", "unused"])
s_set = s.cat.set_categories(["high", "mid"])

def err_type(fn):
    try:
        fn()
        return None
    except Exception as e:
        return type(e).__name__

res = {
    "categories": list(s.cat.categories),
    "ordered": bool(s.cat.ordered),
    "codes": [int(x) for x in s.cat.codes],
    "codes_dtype": str(s.cat.codes.dtype),
    "as_ordered": bool(s_ord.cat.ordered),
    "as_unordered": bool(s_unord.cat.ordered),
    "ord_min": str(s_ord.min()),
    "ord_max": str(s_ord.max()),
    "added_cats": list(s_added.cat.categories),
    "added_codes": [int(x) for x in s_added.cat.codes],
    "unused_rm_cats": list(s_unused_rm.cat.categories),
    "unused_rm_codes": [int(x) for x in s_unused_rm.cat.codes],
    "rm_cats": list(s_rm.cat.categories),
    "rm_codes": [int(x) for x in s_rm.cat.codes],
    "renamed_cats": list(s_renamed.cat.categories),
    "renamed_codes": [int(x) for x in s_renamed.cat.codes],
    "reordered_cats": list(s_reordered.cat.categories),
    "reordered_codes": [int(x) for x in s_reordered.cat.codes],
    "set_cats": list(s_set.cat.categories),
    "set_codes": [int(x) for x in s_set.cat.codes],
    "rename_mismatch_err": err_type(lambda: s.cat.rename_categories(["only_one"])),
    "reorder_mismatch_err": err_type(lambda: s.cat.reorder_categories(["low", "high"])),
    "remove_unknown_err": err_type(lambda: s.cat.remove_categories(["not_a_cat"])),
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_accessor_oracle_eval(python_code) else {
        eprintln!("skipping Series.cat full-surface live-oracle test");
        return;
    };

    let s = Series::from_categorical_codes(
        "grade",
        vec![0, -1, 2, 0],
        vec![
            Scalar::Utf8("low".to_string()),
            Scalar::Utf8("unused".to_string()),
            Scalar::Utf8("high".to_string()),
        ],
        false,
    )
    .unwrap();
    let cat = s.cat().unwrap();

    let parse_str = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect()
    };
    let parse_i64 = |v: &serde_json::Value| -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect()
    };
    let cats_str = |ser: &Series| -> Vec<String> {
        ser.cat()
            .unwrap()
            .categories()
            .iter()
            .map(|v| match v {
                Scalar::Utf8(st) => st.clone(),
                other => panic!("expected Utf8 category, got {other:?}"),
            })
            .collect()
    };
    let codes_i64 = |ser: &Series| -> Vec<i64> {
        ser.cat()
            .unwrap()
            .codes()
            .unwrap()
            .values()
            .iter()
            .map(|v| v.to_i64().unwrap())
            .collect()
    };

    assert_eq!(cats_str(&s), parse_str(&oracle["categories"]));
    assert_eq!(cat.ordered(), oracle["ordered"].as_bool().unwrap());
    assert_eq!(codes_i64(&s), parse_i64(&oracle["codes"]));
    assert_eq!(oracle["codes_dtype"].as_str(), Some("int8"));

    let s_ord = cat.as_ordered();
    assert_eq!(
        s_ord.cat().unwrap().ordered(),
        oracle["as_ordered"].as_bool().unwrap()
    );
    let s_unord = s_ord.cat().unwrap().as_unordered();
    assert_eq!(
        s_unord.cat().unwrap().ordered(),
        oracle["as_unordered"].as_bool().unwrap()
    );
    assert_eq!(
        s_ord.cat().unwrap().min().unwrap(),
        Scalar::Utf8(oracle["ord_min"].as_str().unwrap().to_string())
    );
    assert_eq!(
        s_ord.cat().unwrap().max().unwrap(),
        Scalar::Utf8(oracle["ord_max"].as_str().unwrap().to_string())
    );

    let s_added = cat
        .add_categories(vec![Scalar::Utf8("extra".to_string())])
        .unwrap();
    assert_eq!(cats_str(&s_added), parse_str(&oracle["added_cats"]));
    assert_eq!(codes_i64(&s_added), parse_i64(&oracle["added_codes"]));

    let s_unused_rm = cat.remove_unused_categories().unwrap();
    assert_eq!(cats_str(&s_unused_rm), parse_str(&oracle["unused_rm_cats"]));
    assert_eq!(
        codes_i64(&s_unused_rm),
        parse_i64(&oracle["unused_rm_codes"])
    );

    let s_rm = s_ord
        .cat()
        .unwrap()
        .remove_categories(&[Scalar::Utf8("unused".to_string())])
        .unwrap();
    assert_eq!(cats_str(&s_rm), parse_str(&oracle["rm_cats"]));
    assert_eq!(codes_i64(&s_rm), parse_i64(&oracle["rm_codes"]));

    let s_renamed = cat
        .rename_categories(vec![
            Scalar::Utf8("L".to_string()),
            Scalar::Utf8("U".to_string()),
            Scalar::Utf8("H".to_string()),
        ])
        .unwrap();
    assert_eq!(cats_str(&s_renamed), parse_str(&oracle["renamed_cats"]));
    assert_eq!(codes_i64(&s_renamed), parse_i64(&oracle["renamed_codes"]));

    let s_reordered = cat
        .reorder_categories(vec![
            Scalar::Utf8("high".to_string()),
            Scalar::Utf8("low".to_string()),
            Scalar::Utf8("unused".to_string()),
        ])
        .unwrap();
    assert_eq!(cats_str(&s_reordered), parse_str(&oracle["reordered_cats"]));
    assert_eq!(
        codes_i64(&s_reordered),
        parse_i64(&oracle["reordered_codes"])
    );

    let s_set = cat
        .set_categories(vec![
            Scalar::Utf8("high".to_string()),
            Scalar::Utf8("mid".to_string()),
        ])
        .unwrap();
    assert_eq!(cats_str(&s_set), parse_str(&oracle["set_cats"]));
    assert_eq!(codes_i64(&s_set), parse_i64(&oracle["set_codes"]));

    // Negative / error paths + ordered/rename options (br-frankenpandas-lu478)
    assert!(
        cat.rename_categories(vec![Scalar::Utf8("only_one".to_string())])
            .is_err()
    );
    assert!(
        cat.reorder_categories(vec![
            Scalar::Utf8("low".to_string()),
            Scalar::Utf8("high".to_string()),
        ])
        .is_err()
    );
    assert!(
        cat.remove_categories(&[Scalar::Utf8("not_a_cat".to_string())])
            .is_err()
    );
    let s_reord_ord = cat
        .reorder_categories_with_options(
            vec![
                Scalar::Utf8("high".to_string()),
                Scalar::Utf8("low".to_string()),
                Scalar::Utf8("unused".to_string()),
            ],
            Some(true),
        )
        .unwrap();
    assert!(s_reord_ord.cat().unwrap().ordered());
    let s_set_rename = cat
        .set_categories_with_options(
            vec![
                Scalar::Utf8("L".to_string()),
                Scalar::Utf8("U".to_string()),
                Scalar::Utf8("H".to_string()),
            ],
            Some(true),
            true,
        )
        .unwrap();
    assert!(s_set_rename.cat().unwrap().ordered());
    assert_eq!(cats_str(&s_set_rename), parse_str(&oracle["renamed_cats"]));
    assert_eq!(
        codes_i64(&s_set_rename),
        parse_i64(&oracle["renamed_codes"])
    );
    assert_eq!(oracle["rename_mismatch_err"].as_str(), Some("ValueError"));
    assert_eq!(oracle["reorder_mismatch_err"].as_str(), Some("ValueError"));
    assert_eq!(oracle["remove_unknown_err"].as_str(), Some("ValueError"));
}

#[test]
fn live_oracle_series_str_sparse_list_struct_accessors_differential() {
    use fp_columnar::Column;
    use fp_frame::Series;
    use fp_index::{Index, IndexLabel};
    use fp_types::{DType, NullKind, Scalar};

    // @covers: series_str_cat
    // @covers: series_str_join
    // @covers: series_str_partition
    // @covers: series_str_rpartition
    // @covers: series_str_slice_replace
    // @covers: series_str_contains
    // @covers: series_str_len
    // @covers: series_str_count
    // @covers: series_str_find
    // @covers: series_str_rfind
    // @covers: series_str_index
    // @covers: series_str_rindex
    // @covers: series_str_replace
    // @covers: series_str_startswith
    // @covers: series_str_endswith
    // @covers: series_str_isalpha
    // @covers: series_str_isdigit
    // @covers: series_sparse_density
    // @covers: series_sparse_fill_value
    // @covers: series_sparse_from_coo
    // @covers: series_sparse_npoints
    // @covers: series_sparse_sp_values
    // @covers: series_sparse_to_coo
    // @covers: series_sparse_to_dense
    // @covers: series_list_flatten
    // @covers: series_list_len
    // @covers: series_struct_dtypes
    // @covers: series_struct_explode
    // @covers: series_struct_field

    let python_code = r#"
import pandas as pd, numpy as np, json

s = pd.Series(["a-b-c", "d-e", "xyz"], name="txt")
s_null = pd.Series(["Abc", None, "123"], name="txt")
s_other = pd.Series(["1", "2", None], name="oth")

part = s_null.str.partition("-")
rpart = s.str.rpartition("-")

sp = pd.Series(pd.arrays.SparseArray([0.0, 2.5, 0.0, -4.0], fill_value=0.0), name="sp")
sp_empty = pd.Series(pd.arrays.SparseArray([], dtype=np.float64, fill_value=0.0), name="sp")

def opt_str_list(ser):
    return [None if pd.isna(x) else str(x) for x in ser]

def opt_f64_list(ser):
    return [None if pd.isna(x) else float(x) for x in ser]

def opt_bool_list(ser):
    return [None if pd.isna(x) else bool(x) for x in ser]

def err_type(fn):
    try:
        fn()
        return None
    except Exception as e:
        return type(e).__name__

res = {
    "cat_self": s_null.str.cat(sep="|"),
    "cat_series_na": opt_str_list(s_null.str.cat(s_other, sep=":", na_rep="NA")),
    "cat_series_null": opt_str_list(s_null.str.cat(s_other, sep=":")),
    "join": opt_str_list(pd.Series(["a,b", None, "c"]).str.split(",").str.join("-")),
    "part_0": opt_str_list(part[0]),
    "part_1": opt_str_list(part[1]),
    "part_2": opt_str_list(part[2]),
    "rpart_0": opt_str_list(rpart[0]),
    "rpart_1": opt_str_list(rpart[1]),
    "rpart_2": opt_str_list(rpart[2]),
    "slice_replace": opt_str_list(s_null.str.slice_replace(1, 2, "Z")),
    "null_len": opt_f64_list(s_null.str.len()),
    "null_len_dtype": str(s_null.str.len().dtype),
    "null_find": opt_f64_list(s_null.str.find("b")),
    "null_find_dtype": str(s_null.str.find("b").dtype),
    "null_contains_icase": opt_bool_list(s_null.str.contains("abc", case=False)),
    "null_contains_na_false": [bool(x) for x in s_null.str.contains("abc", case=False, na=False)],
    "null_contains_na_false_dtype": str(s_null.str.contains("abc", case=False, na=False).dtype),
    "null_isalpha": opt_bool_list(s_null.str.isalpha()),
    "null_isdigit": opt_bool_list(s_null.str.isdigit()),
    "replace_icase": opt_str_list(s_null.str.replace("a", "X", case=False, regex=True)),
    "index_err": err_type(lambda: s.str.index("not_present")),
    "rindex_err": err_type(lambda: s.str.rindex("not_present")),
    "part_empty_err": err_type(lambda: s.str.partition("")),
    "sp_density": float(sp.sparse.density),
    "sp_fill_value": float(sp.sparse.fill_value),
    "sp_npoints": int(sp.sparse.npoints),
    "sp_values": [float(x) for x in sp.sparse.sp_values],
    "sp_dense": [float(x) for x in sp.sparse.to_dense()],
    "sp_empty_density_err": err_type(lambda: sp_empty.sparse.density),
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_accessor_oracle_eval(python_code) else {
        eprintln!("skipping Series.str/sparse/list/struct live-oracle test");
        return;
    };

    let idx = Index::new(vec![
        IndexLabel::Int64(0),
        IndexLabel::Int64(1),
        IndexLabel::Int64(2),
    ]);
    let s = Series::new(
        "txt",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("a-b-c".to_string()),
            Scalar::Utf8("d-e".to_string()),
            Scalar::Utf8("xyz".to_string()),
        ])
        .unwrap(),
    )
    .unwrap();
    let s_null = Series::new(
        "txt",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("Abc".to_string()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("123".to_string()),
        ])
        .unwrap(),
    )
    .unwrap();
    let s_other = Series::new(
        "oth",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("1".to_string()),
            Scalar::Utf8("2".to_string()),
            Scalar::Null(NullKind::Null),
        ])
        .unwrap(),
    )
    .unwrap();

    let parse_opt_str = |v: &serde_json::Value| -> Vec<Option<String>> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().map(ToOwned::to_owned))
            .collect()
    };
    let parse_opt_f64 = |v: &serde_json::Value| -> Vec<Option<f64>> {
        v.as_array().unwrap().iter().map(|x| x.as_f64()).collect()
    };
    let parse_opt_bool = |v: &serde_json::Value| -> Vec<Option<bool>> {
        v.as_array().unwrap().iter().map(|x| x.as_bool()).collect()
    };
    let ser_opt_str = |ser: &Series| -> Vec<Option<String>> {
        ser.values()
            .iter()
            .map(|v| match v {
                Scalar::Utf8(st) => Some(st.clone()),
                _ if v.is_missing() => None,
                other => panic!("expected Utf8 or missing, got {other:?}"),
            })
            .collect()
    };
    let ser_opt_f64 = |ser: &Series| -> Vec<Option<f64>> {
        ser.values()
            .iter()
            .map(|v| {
                if v.is_missing() {
                    None
                } else {
                    v.to_f64().ok().filter(|f| !f.is_nan())
                }
            })
            .collect()
    };
    let ser_opt_bool = |ser: &Series| -> Vec<Option<bool>> {
        ser.values()
            .iter()
            .map(|v| match v {
                Scalar::Bool(b) => Some(*b),
                _ if v.is_missing() => None,
                other => panic!("expected Bool or missing, got {other:?}"),
            })
            .collect()
    };

    // Series.str: cat, join, partition, rpartition, slice_replace
    assert_eq!(
        s_null.str().cat("|").unwrap(),
        oracle["cat_self"].as_str().unwrap()
    );
    assert_eq!(
        ser_opt_str(&s_null.str().cat_series(&s_other, ":", Some("NA")).unwrap()),
        parse_opt_str(&oracle["cat_series_na"])
    );
    assert_eq!(
        ser_opt_str(&s_null.str().cat_series(&s_other, ":", None).unwrap()),
        parse_opt_str(&oracle["cat_series_null"])
    );

    let s_join = Series::new(
        "j",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("a,b".to_string()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("c".to_string()),
        ])
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        ser_opt_str(&s_join.str().join(",", "-").unwrap()),
        parse_opt_str(&oracle["join"])
    );

    let (p0, p1, p2) = s_null.str().partition("-").unwrap();
    assert_eq!(ser_opt_str(&p0), parse_opt_str(&oracle["part_0"]));
    assert_eq!(ser_opt_str(&p1), parse_opt_str(&oracle["part_1"]));
    assert_eq!(ser_opt_str(&p2), parse_opt_str(&oracle["part_2"]));

    let (rp0, rp1, rp2) = s.str().rpartition("-").unwrap();
    assert_eq!(ser_opt_str(&rp0), parse_opt_str(&oracle["rpart_0"]));
    assert_eq!(ser_opt_str(&rp1), parse_opt_str(&oracle["rpart_1"]));
    assert_eq!(ser_opt_str(&rp2), parse_opt_str(&oracle["rpart_2"]));

    assert_eq!(
        ser_opt_str(&s_null.str().slice_replace(Some(1), Some(2), "Z").unwrap()),
        parse_opt_str(&oracle["slice_replace"])
    );

    // Null handling + dtype promotion + regex flags
    let null_len = s_null.str().len().unwrap();
    assert_eq!(null_len.dtype(), DType::Float64);
    assert_eq!(oracle["null_len_dtype"].as_str(), Some("float64"));
    assert_eq!(ser_opt_f64(&null_len), parse_opt_f64(&oracle["null_len"]));

    let null_find = s_null.str().find("b").unwrap();
    assert_eq!(null_find.dtype(), DType::Float64);
    assert_eq!(oracle["null_find_dtype"].as_str(), Some("float64"));
    assert_eq!(ser_opt_f64(&null_find), parse_opt_f64(&oracle["null_find"]));

    assert_eq!(
        ser_opt_bool(
            &s_null
                .str()
                .contains_with_options("abc", false, None, true)
                .unwrap()
        ),
        parse_opt_bool(&oracle["null_contains_icase"])
    );
    let contains_na_false = s_null
        .str()
        .contains_with_options("abc", false, Some(false), true)
        .unwrap();
    assert_eq!(contains_na_false.dtype(), DType::Bool);
    assert_eq!(
        oracle["null_contains_na_false_dtype"].as_str(),
        Some("bool")
    );
    assert_eq!(
        ser_opt_bool(&contains_na_false),
        vec![Some(true), Some(false), Some(false)]
    );
    assert_eq!(
        ser_opt_bool(&s_null.str().isalpha().unwrap()),
        parse_opt_bool(&oracle["null_isalpha"])
    );
    assert_eq!(
        ser_opt_bool(&s_null.str().isdigit().unwrap()),
        parse_opt_bool(&oracle["null_isdigit"])
    );
    assert_eq!(
        ser_opt_str(
            &s_null
                .str()
                .replace_with_options("a", "X", None, false, true)
                .unwrap()
        ),
        parse_opt_str(&oracle["replace_icase"])
    );

    // Negative / error paths for Series.str (br-frankenpandas-wtgnl)
    assert!(s.str().index("not_present").is_err());
    assert!(s.str().rindex("not_present").is_err());
    assert!(s.str().partition("").is_err());
    assert!(s.str().rpartition("").is_err());
    assert!(s.str().contains_regex("[unclosed").is_err());
    let s_idx_null = Series::new(
        "txt",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("abc".to_string()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("abbc".to_string()),
        ])
        .unwrap(),
    )
    .unwrap();
    let idx_res = s_idx_null.str().index("b").unwrap();
    let ridx_res = s_idx_null.str().rindex("b").unwrap();
    assert_eq!(idx_res.dtype(), DType::Float64);
    assert_eq!(ridx_res.dtype(), DType::Float64);
    assert_eq!(ser_opt_f64(&idx_res), vec![Some(1.0), None, Some(1.0)]);
    assert_eq!(ser_opt_f64(&ridx_res), vec![Some(1.0), None, Some(2.0)]);
    assert_eq!(
        s_null.str().cat_with_na_rep("|", Some("NA")).unwrap(),
        "Abc|NA|123"
    );
    assert_eq!(oracle["index_err"].as_str(), Some("ValueError"));
    assert_eq!(oracle["rindex_err"].as_str(), Some("ValueError"));
    assert_eq!(oracle["part_empty_err"].as_str(), Some("ValueError"));

    // Series.sparse
    let sp = Series::from_sparse_dense(
        "sp",
        vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ],
        vec![
            Scalar::Float64(0.0),
            Scalar::Float64(2.5),
            Scalar::Float64(0.0),
            Scalar::Float64(-4.0),
        ],
        DType::Float64,
        Scalar::Float64(0.0),
    )
    .unwrap();
    let sp_acc = sp.sparse().unwrap();
    assert!((sp_acc.density() - oracle["sp_density"].as_f64().unwrap()).abs() < 1e-12);
    assert_eq!(
        sp_acc.fill_value(),
        &Scalar::Float64(oracle["sp_fill_value"].as_f64().unwrap())
    );
    assert_eq!(
        sp_acc.npoints(),
        oracle["sp_npoints"].as_u64().unwrap() as usize
    );
    assert_eq!(
        sp_acc.sp_values(),
        vec![Scalar::Float64(2.5), Scalar::Float64(-4.0)]
    );
    assert_eq!(sp_acc.sp_index(), vec![1, 3]);
    assert_eq!(
        ser_opt_f64(&sp_acc.to_dense()),
        parse_opt_f64(&oracle["sp_dense"])
    );
    let sp_empty = Series::from_sparse_dense(
        "sp",
        Vec::new(),
        Vec::new(),
        DType::Float64,
        Scalar::Float64(0.0),
    )
    .unwrap();
    assert!(sp_empty.sparse().unwrap().density().is_nan());
    let sp_coo = fp_frame::SparseAccessor::from_coo(
        "coo",
        vec![Scalar::Float64(2.5), Scalar::Float64(-4.0)],
        &[0, 1],
        &[1, 0],
        (2, 2),
        DType::Float64,
        false,
    )
    .unwrap();
    let (coo_vals, coo_rows, coo_cols, _, _) = sp_coo.sparse().unwrap().to_coo(true).unwrap();
    assert_eq!(coo_vals, vec![Scalar::Float64(2.5), Scalar::Float64(-4.0)]);
    assert_eq!(coo_rows, vec![0, 1]);
    assert_eq!(coo_cols, vec![1, 0]);
    assert_eq!(
        oracle["sp_empty_density_err"].as_str(),
        Some("ZeroDivisionError")
    );

    // Series.list
    let s_list = Series::new(
        "lst",
        idx.clone(),
        Column::from_values(vec![
            Scalar::Utf8("[10, 20]".to_string()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("[30]".to_string()),
        ])
        .unwrap(),
    )
    .unwrap();
    assert!(s_list.list().is_supported());
    assert_eq!(
        s_list.list().len().unwrap().values(),
        &[
            Scalar::Int64(2),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(1),
        ]
    );
    assert_eq!(
        s_list.list().flatten().unwrap().values(),
        &[Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)]
    );
    assert_eq!(
        s_list.list().get(0).unwrap().values(),
        &[
            Scalar::Int64(10),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(30),
        ]
    );
    let s_bad_list = Series::new(
        "bad",
        Index::new(vec![IndexLabel::Int64(0)]),
        Column::from_values(vec![Scalar::Utf8("not_json".to_string())]).unwrap(),
    )
    .unwrap();
    assert!(s_bad_list.list().len().is_err());

    // Series.struct
    let s_struct = Series::new(
        "st",
        idx,
        Column::from_values(vec![
            Scalar::Utf8(r#"{"a": 1, "b": "x"}"#.to_string()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8(r#"{"a": 2, "b": "y"}"#.to_string()),
        ])
        .unwrap(),
    )
    .unwrap();
    assert!(s_struct.r#struct().is_supported());
    assert_eq!(
        s_struct.r#struct().field_names().unwrap(),
        vec!["a".to_string(), "b".to_string()]
    );
    assert_eq!(
        s_struct.r#struct().field("a").unwrap().values(),
        &[
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(2),
        ]
    );
    assert_eq!(
        s_struct.r#struct().field("b").unwrap().values(),
        &[
            Scalar::Utf8("x".to_string()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("y".to_string()),
        ]
    );
    let exploded = s_struct.r#struct().explode().unwrap();
    assert_eq!(exploded.column_names(), &["a", "b"]);
    let st_dtypes = s_struct.r#struct().dtypes().unwrap();
    assert_eq!(st_dtypes.len(), 2);
    assert!(s_bad_list.r#struct().field("a").is_err());
}
