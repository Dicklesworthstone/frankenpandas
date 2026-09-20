//! Series parity-matrix conformance suite (br-frankenpandas-3qt8).
//!
//! Per /testing-conformance-harnesses skill Pattern 4 (spec-derived test
//! matrix). Each test picks one Series-family operation and runs it
//! through an edge-case input (empty, single-row, all-NaN, duplicate
//! labels, misaligned indexes). The live pandas oracle is the reference
//! implementation; our Rust result must match via the standard
//! `compare_series_expected` / scalar comparators.
//!
//! Each test skips gracefully (no failure) when the live oracle isn't
//! available — matches the convention of sibling `live_oracle_*` tests.

use super::{
    EvidenceLedger, FixtureExpectedSeries, FrameError, HarnessConfig, HarnessError, IndexLabel,
    NullKind, PacketFixture, ResolvedExpected, RuntimePolicy, Scalar, Series, build_series,
    capture_live_oracle_expected, compare_scalar, compare_series_expected,
};

fn oracle_series_expected(
    cfg: &HarnessConfig,
    fixture: &PacketFixture,
) -> Result<Option<FixtureExpectedSeries>, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Series(series)) => Ok(Some(series)),
        Ok(other) => Err(format!("expected series payload, got {other:?}")),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping Series conformance test {}: {message}",
                fixture.case_id
            );
            Ok(None)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn oracle_scalar_expected(
    cfg: &HarnessConfig,
    fixture: &PacketFixture,
) -> Result<Option<Scalar>, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Scalar(scalar)) => Ok(Some(scalar)),
        Ok(other) => Err(format!("expected scalar payload, got {other:?}")),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping Series conformance test {}: {message}",
                fixture.case_id
            );
            Ok(None)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

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

/// Helper: run series_add against the oracle + compare.
fn check_series_add(fixture: PacketFixture) {
    let cfg = strict_config();
    let Some(expected) = oracle_series_expected(&cfg, &fixture).expect("series oracle") else {
        return;
    };
    let left = build_series(fixture.left.as_ref().expect("left series")).expect("left build");
    let right = build_series(fixture.right.as_ref().expect("right series")).expect("right build");
    let policy = RuntimePolicy::strict();
    let mut ledger = EvidenceLedger::new();
    let actual = left
        .add_with_policy(&right, &policy, &mut ledger)
        .expect("series_add");
    compare_series_expected(&actual, &expected).expect("pandas series_add parity");
}

fn check_series_mode(fixture: PacketFixture) {
    let cfg = strict_config();
    let Some(expected) = oracle_series_expected(&cfg, &fixture).expect("series oracle") else {
        return;
    };
    let series = build_series(fixture.left.as_ref().expect("left series")).expect("series build");
    // Default dropna=true matches pandas Series.mode default.
    let actual = series.mode().expect("series_mode");
    compare_series_expected(&actual, &expected).expect("pandas series_mode parity");
}

fn check_series_nunique(fixture: PacketFixture) {
    let cfg = strict_config();
    let Some(expected) = oracle_scalar_expected(&cfg, &fixture).expect("scalar oracle") else {
        return;
    };
    let series = build_series(fixture.left.as_ref().expect("left series")).expect("series build");
    let actual = Scalar::Int64(series.nunique() as i64);
    compare_scalar(&actual, &expected, "series_nunique").expect("pandas series_nunique parity");
}

// ── series_add edge matrix ────────────────────────────────────────────

#[test]
fn conformance_series_add_empty_pair() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-001",
        "case_id": "series_add_empty_pair",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [], "values": [] },
        "right": { "name": "r", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

#[test]
fn conformance_series_add_single_row() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-002",
        "case_id": "series_add_single_row",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [{ "kind": "int64", "value": 0 }],
                   "values": [{ "kind": "float64", "value": 42.0 }] },
        "right": { "name": "r", "index": [{ "kind": "int64", "value": 0 }],
                   "values": [{ "kind": "float64", "value": 8.0 }] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

#[test]
fn conformance_series_add_all_nan() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-003",
        "case_id": "series_add_all_nan",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [
                       { "kind": "int64", "value": 0 },
                       { "kind": "int64", "value": 1 },
                       { "kind": "int64", "value": 2 }],
                   "values": [
                       { "kind": "null", "value": "na_n" },
                       { "kind": "null", "value": "na_n" },
                       { "kind": "null", "value": "na_n" }] },
        "right": { "name": "r", "index": [
                       { "kind": "int64", "value": 0 },
                       { "kind": "int64", "value": 1 },
                       { "kind": "int64", "value": 2 }],
                   "values": [
                       { "kind": "null", "value": "na_n" },
                       { "kind": "float64", "value": 1.0 },
                       { "kind": "null", "value": "na_n" }] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

#[test]
fn conformance_series_add_duplicate_labels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-004",
        "case_id": "series_add_duplicate_labels",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [
                       { "kind": "int64", "value": 1 },
                       { "kind": "int64", "value": 1 }],
                   "values": [
                       { "kind": "float64", "value": 10.0 },
                       { "kind": "float64", "value": 20.0 }] },
        "right": { "name": "r", "index": [
                       { "kind": "int64", "value": 1 },
                       { "kind": "int64", "value": 1 }],
                   "values": [
                       { "kind": "float64", "value": 100.0 },
                       { "kind": "float64", "value": 200.0 }] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

#[test]
fn conformance_series_add_misaligned_indexes() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-ADD-005",
        "case_id": "series_add_misaligned_indexes",
        "mode": "strict",
        "operation": "series_add",
        "oracle_source": "live_legacy_pandas",
        "left":  { "name": "l", "index": [
                       { "kind": "int64", "value": 0 },
                       { "kind": "int64", "value": 1 }],
                   "values": [
                       { "kind": "int64", "value": 10 },
                       { "kind": "int64", "value": 20 }] },
        "right": { "name": "r", "index": [
                       { "kind": "int64", "value": 2 },
                       { "kind": "int64", "value": 3 }],
                   "values": [
                       { "kind": "int64", "value": 100 },
                       { "kind": "int64", "value": 200 }] }
    }))
    .expect("fixture");
    check_series_add(fixture);
}

// ── series_mode edge matrix ───────────────────────────────────────────

#[test]
fn conformance_series_mode_empty() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-MODE-001",
        "case_id": "series_mode_empty",
        "mode": "strict",
        "operation": "series_mode",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_series_mode(fixture);
}

#[test]
fn conformance_series_mode_unique_no_mode() {
    // All values distinct → pandas returns every value (everything ties at 1).
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-MODE-002",
        "case_id": "series_mode_unique_no_mode",
        "mode": "strict",
        "operation": "series_mode",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [
                      { "kind": "int64", "value": 0 },
                      { "kind": "int64", "value": 1 },
                      { "kind": "int64", "value": 2 }],
                  "values": [
                      { "kind": "int64", "value": 1 },
                      { "kind": "int64", "value": 2 },
                      { "kind": "int64", "value": 3 }] }
    }))
    .expect("fixture");
    check_series_mode(fixture);
}

// ── series_nunique edge matrix ───────────────────────────────────────

#[test]
fn conformance_series_nunique_empty() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-NUNIQUE-001",
        "case_id": "series_nunique_empty",
        "mode": "strict",
        "operation": "series_nunique",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_series_nunique(fixture);
}

#[test]
fn conformance_series_nunique_all_nan() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-NUNIQUE-002",
        "case_id": "series_nunique_all_nan",
        "mode": "strict",
        "operation": "series_nunique",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [
                      { "kind": "int64", "value": 0 },
                      { "kind": "int64", "value": 1 },
                      { "kind": "int64", "value": 2 }],
                  "values": [
                      { "kind": "null", "value": "na_n" },
                      { "kind": "null", "value": "na_n" },
                      { "kind": "null", "value": "na_n" }] }
    }))
    .expect("fixture");
    check_series_nunique(fixture);
}

#[test]
fn conformance_series_nunique_all_duplicates() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-SERIES-NUNIQUE-003",
        "case_id": "series_nunique_all_duplicates",
        "mode": "strict",
        "operation": "series_nunique",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "s", "index": [
                      { "kind": "int64", "value": 0 },
                      { "kind": "int64", "value": 1 },
                      { "kind": "int64", "value": 2 }],
                  "values": [
                      { "kind": "utf8", "value": "x" },
                      { "kind": "utf8", "value": "x" },
                      { "kind": "utf8", "value": "x" }] }
    }))
    .expect("fixture");
    check_series_nunique(fixture);
}

// ── list/struct accessor compatibility contracts ─────────────────────

#[test]
fn conformance_series_list_json_accessor_contract_zzbqc() {
    let series = Series::from_values(
        "items",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
        ],
        vec![
            Scalar::Utf8(r#"[1,"x",null]"#.into()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("[]".into()),
            Scalar::Utf8("[true,2.5]".into()),
        ],
    )
    .expect("list contract series");

    let accessor = series.list();
    assert!(accessor.is_supported());

    let lengths = accessor.len().expect("list lengths");
    assert_eq!(
        lengths.values(),
        &[
            Scalar::Int64(3),
            Scalar::Null(NullKind::Null),
            Scalar::Int64(0),
            Scalar::Int64(2),
        ]
    );
    assert_eq!(lengths.index().labels(), series.index().labels());

    let second = accessor.get(1).expect("list get");
    assert_eq!(
        second.values(),
        &[
            Scalar::Utf8("x".into()),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Float64(2.5),
        ]
    );

    let missing = accessor.get(9).expect("list missing index");
    assert_eq!(
        missing.values(),
        &[
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
        ]
    );

    let flattened = accessor.flatten().expect("list flatten");
    assert_eq!(
        flattened.values(),
        &[
            Scalar::Int64(1),
            Scalar::Utf8("x".into()),
            Scalar::Null(NullKind::Null),
            Scalar::Bool(true),
            Scalar::Float64(2.5),
        ]
    );

    let scalar_series =
        Series::from_values("bad", vec![IndexLabel::Int64(0)], vec![Scalar::Int64(1)])
            .expect("bad list series");
    let err = scalar_series.list().len().expect_err("non-list rejects");
    assert!(
        matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("UTF-8 JSON arrays") && msg.contains("position 0"))
    );

    let nested_series = Series::from_values(
        "nested",
        vec![IndexLabel::Int64(0)],
        vec![Scalar::Utf8("[[1]]".into())],
    )
    .expect("nested list series");
    let err = nested_series
        .list()
        .flatten()
        .expect_err("nested list rejects");
    assert!(
        matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("nested JSON arrays/objects"))
    );
}

#[test]
fn conformance_series_struct_json_accessor_contract_zzbqc() {
    let series = Series::from_values(
        "records",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
        ],
        vec![
            Scalar::Utf8(r#"{"id":1,"name":"Ada"}"#.into()),
            Scalar::Utf8(r#"{"id":null}"#.into()),
            Scalar::Null(NullKind::Null),
            Scalar::Utf8("{}".into()),
        ],
    )
    .expect("struct contract series");

    let accessor = series.r#struct();
    assert!(accessor.is_supported());
    assert_eq!(
        accessor.field_names().expect("field names"),
        vec!["id", "name"]
    );

    let ids = accessor.field("id").expect("id field");
    assert_eq!(ids.name(), "id");
    assert_eq!(
        ids.values(),
        &[
            Scalar::Int64(1),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
        ]
    );
    assert_eq!(ids.index().labels(), series.index().labels());

    let names = accessor.field("name").expect("name field");
    assert_eq!(
        names.values(),
        &[
            Scalar::Utf8("Ada".into()),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
        ]
    );

    let missing = accessor.field("missing").expect("missing field");
    assert_eq!(
        missing.values(),
        &[
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
            Scalar::Null(NullKind::Null),
        ]
    );

    let array_series = Series::from_values(
        "array",
        vec![IndexLabel::Int64(0)],
        vec![Scalar::Utf8("[1]".into())],
    )
    .expect("array struct series");
    let err = array_series
        .r#struct()
        .field_names()
        .expect_err("non-struct rejects");
    assert!(
        matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("non-object JSON") && msg.contains("position 0"))
    );

    let nested_series = Series::from_values(
        "nested",
        vec![IndexLabel::Int64(0)],
        vec![Scalar::Utf8(r#"{"payload":{"x":1}}"#.into())],
    )
    .expect("nested struct series");
    let err = nested_series
        .r#struct()
        .field("payload")
        .expect_err("nested struct rejects");
    assert!(
        matches!(err, FrameError::CompatibilityRejected(msg) if msg.contains("nested JSON arrays/objects"))
    );
}

fn run_pandas_oracle_eval(code: &str) -> Option<serde_json::Value> {
    use std::io::Write;
    let cfg = strict_config();
    let python = &cfg.python_bin;
    let mut child = std::process::Command::new(python)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(code.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

#[test]
fn conformance_series_get_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([10.5, 20.0, 35.5], index=["x", "y", "z"], name="nums")
val_x = s.get("x")
val_missing = s.get("missing", default="fallback")
res = {
    "x": float(val_x),
    "missing": str(val_missing),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series get differential test");
            return;
        }
    };

    let s = Series::from_values(
        "nums",
        vec![
            IndexLabel::Utf8("x".into()),
            IndexLabel::Utf8("y".into()),
            IndexLabel::Utf8("z".into()),
        ],
        vec![
            Scalar::Float64(10.5),
            Scalar::Float64(20.0),
            Scalar::Float64(35.5),
        ],
    )
    .expect("s");

    let val_x = s.get(&IndexLabel::Utf8("x".into())).expect("val_x");
    assert_eq!(val_x.to_f64().unwrap(), oracle["x"].as_f64().unwrap());

    let val_missing = s.get(&IndexLabel::Utf8("missing".into()));
    assert!(val_missing.is_none());

    let val_fallback = s.get_or(
        &IndexLabel::Utf8("missing".into()),
        Scalar::Utf8("fallback".into()),
    );
    assert_eq!(
        val_fallback,
        Scalar::Utf8(oracle["missing"].as_str().unwrap().into())
    );
}

#[test]
fn conformance_series_truncate_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([1.0, 2.0, 3.0, 4.0, 5.0], index=["a", "b", "c", "d", "e"], name="data")
t1 = s.truncate(before="b", after="d")
t2 = s.truncate(before="c")
t3 = s.truncate(after="b")
res = {
    "t1_idx": [str(x) for x in t1.index],
    "t1_vals": t1.tolist(),
    "t2_idx": [str(x) for x in t2.index],
    "t2_vals": t2.tolist(),
    "t3_idx": [str(x) for x in t3.index],
    "t3_vals": t3.tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series truncate differential test");
            return;
        }
    };

    let s = Series::from_values(
        "data",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
            IndexLabel::Utf8("e".into()),
        ],
        vec![
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Float64(3.0),
            Scalar::Float64(4.0),
            Scalar::Float64(5.0),
        ],
    )
    .expect("s");

    let t1 = s
        .truncate(
            Some(&IndexLabel::Utf8("b".into())),
            Some(&IndexLabel::Utf8("d".into())),
        )
        .expect("truncate t1");
    let actual_t1_idx: Vec<String> = t1.index().labels().iter().map(|l| l.to_string()).collect();
    let oracle_t1_idx: Vec<String> = oracle["t1_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_t1_idx, oracle_t1_idx);
    let actual_t1_vals: Vec<f64> = t1
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let oracle_t1_vals: Vec<f64> = oracle["t1_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_t1_vals, oracle_t1_vals);

    let t2 = s
        .truncate(Some(&IndexLabel::Utf8("c".into())), None)
        .expect("truncate t2");
    let actual_t2_idx: Vec<String> = t2.index().labels().iter().map(|l| l.to_string()).collect();
    let oracle_t2_idx: Vec<String> = oracle["t2_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_t2_idx, oracle_t2_idx);
    let actual_t2_vals: Vec<f64> = t2
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let oracle_t2_vals: Vec<f64> = oracle["t2_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_t2_vals, oracle_t2_vals);

    let t3 = s
        .truncate(None, Some(&IndexLabel::Utf8("b".into())))
        .expect("truncate t3");
    let actual_t3_idx: Vec<String> = t3.index().labels().iter().map(|l| l.to_string()).collect();
    let oracle_t3_idx: Vec<String> = oracle["t3_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_t3_idx, oracle_t3_idx);
    let actual_t3_vals: Vec<f64> = t3
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let oracle_t3_vals: Vec<f64> = oracle["t3_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_t3_vals, oracle_t3_vals);
}

#[test]
fn conformance_series_set_axis_and_rename_axis_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([100, 200, 300], index=["i0", "i1", "i2"], name="s")
s_new_axis = s.set_axis(["k0", "k1", "k2"])
s_renamed_axis = s.rename_axis("new_axis_name")
res = {
    "new_axis_idx": [str(x) for x in s_new_axis.index],
    "renamed_axis_idx": [str(x) for x in s_renamed_axis.index],
    "renamed_axis_name": s_renamed_axis.index.name,
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping Series set_axis/rename_axis differential test"
            );
            return;
        }
    };

    let s = Series::from_values(
        "s",
        vec![
            IndexLabel::Utf8("i0".into()),
            IndexLabel::Utf8("i1".into()),
            IndexLabel::Utf8("i2".into()),
        ],
        vec![Scalar::Int64(100), Scalar::Int64(200), Scalar::Int64(300)],
    )
    .expect("s");

    let s_new_axis = s
        .set_axis(vec![
            IndexLabel::Utf8("k0".into()),
            IndexLabel::Utf8("k1".into()),
            IndexLabel::Utf8("k2".into()),
        ])
        .expect("set_axis");
    let actual_new_idx: Vec<String> = s_new_axis
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_new_idx: Vec<String> = oracle["new_axis_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_new_idx, oracle_new_idx);

    let s_renamed_axis = s.rename_axis("new_axis_name").expect("rename_axis");
    let actual_renamed_idx: Vec<String> = s_renamed_axis
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_renamed_idx: Vec<String> = oracle["renamed_axis_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_renamed_idx, oracle_renamed_idx);
    assert_eq!(
        s_renamed_axis.index().name(),
        oracle["renamed_axis_name"].as_str()
    );
}

#[test]
fn conformance_series_first_last_valid_index_differential() {
    let python_code = r#"
import json, pandas as pd, numpy as np
s = pd.Series([np.nan, 2.0, np.nan, 4.0, np.nan], index=["a", "b", "c", "d", "e"])
s_all_nan = pd.Series([np.nan, np.nan], index=["x", "y"])
res = {
    "first": str(s.first_valid_index()),
    "last": str(s.last_valid_index()),
    "all_nan_first": s_all_nan.first_valid_index(),
    "all_nan_last": s_all_nan.last_valid_index(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping Series first/last_valid_index differential test"
            );
            return;
        }
    };

    let s = Series::from_values(
        "data",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
            IndexLabel::Utf8("e".into()),
        ],
        vec![
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(2.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(4.0),
            Scalar::Null(NullKind::NaN),
        ],
    )
    .expect("s");

    let first = s.first_valid_index().expect("first valid index");
    let last = s.last_valid_index().expect("last valid index");
    assert_eq!(first.to_string(), oracle["first"].as_str().unwrap());
    assert_eq!(last.to_string(), oracle["last"].as_str().unwrap());

    let s_all_nan = Series::from_values(
        "nan_data",
        vec![IndexLabel::Utf8("x".into()), IndexLabel::Utf8("y".into())],
        vec![Scalar::Null(NullKind::NaN), Scalar::Null(NullKind::NaN)],
    )
    .expect("s_all_nan");

    assert!(s_all_nan.first_valid_index().is_none());
    assert!(s_all_nan.last_valid_index().is_none());
    assert!(oracle["all_nan_first"].is_null());
    assert!(oracle["all_nan_last"].is_null());
}

#[test]
fn conformance_series_isin_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([10, 20, 30, 40, 20], index=["r0", "r1", "r2", "r3", "r4"])
isin_res = s.isin([20, 40, 99])
res = {
    "isin_vals": isin_res.tolist(),
    "isin_idx": [str(x) for x in isin_res.index],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series isin differential test");
            return;
        }
    };

    let s = Series::from_values(
        "nums",
        vec![
            IndexLabel::Utf8("r0".into()),
            IndexLabel::Utf8("r1".into()),
            IndexLabel::Utf8("r2".into()),
            IndexLabel::Utf8("r3".into()),
            IndexLabel::Utf8("r4".into()),
        ],
        vec![
            Scalar::Int64(10),
            Scalar::Int64(20),
            Scalar::Int64(30),
            Scalar::Int64(40),
            Scalar::Int64(20),
        ],
    )
    .expect("s");

    let isin_res = s
        .isin(&[Scalar::Int64(20), Scalar::Int64(40), Scalar::Int64(99)])
        .expect("isin");
    let actual_vals: Vec<bool> = isin_res
        .column()
        .values()
        .iter()
        .map(|v| v.to_bool().unwrap_or(false))
        .collect();
    let oracle_vals: Vec<bool> = oracle["isin_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(actual_vals, oracle_vals);

    let actual_idx: Vec<String> = isin_res
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_idx: Vec<String> = oracle["isin_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_idx, oracle_idx);
}

#[test]
fn conformance_series_add_prefix_suffix_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([1, 2, 3], index=["x", "y", "z"])
p = s.add_prefix("pre_")
suf = s.add_suffix("_post")
res = {
    "pre_idx": [str(x) for x in p.index],
    "suf_idx": [str(x) for x in suf.index],
    "pre_vals": p.tolist(),
    "suf_vals": suf.tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping Series add_prefix/suffix differential test"
            );
            return;
        }
    };

    let s = Series::from_values(
        "s",
        vec![
            IndexLabel::Utf8("x".into()),
            IndexLabel::Utf8("y".into()),
            IndexLabel::Utf8("z".into()),
        ],
        vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
    )
    .expect("s");

    let p = s.add_prefix("pre_").expect("prefix");
    let suf = s.add_suffix("_post").expect("suffix");

    let actual_pre_idx: Vec<String> = p.index().labels().iter().map(|l| l.to_string()).collect();
    let oracle_pre_idx: Vec<String> = oracle["pre_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_pre_idx, oracle_pre_idx);

    let actual_suf_idx: Vec<String> = suf.index().labels().iter().map(|l| l.to_string()).collect();
    let oracle_suf_idx: Vec<String> = oracle["suf_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_suf_idx, oracle_suf_idx);

    let actual_pre_vals: Vec<i64> = p
        .column()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap_or(0))
        .collect();
    let oracle_pre_vals: Vec<i64> = oracle["pre_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_pre_vals, oracle_pre_vals);
}

#[test]
fn conformance_series_pop_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([10.0, 20.0, 30.0], index=["a", "b", "c"], name="target")
s.index.name = "idx_name"
val = s.pop("b")
res = {
    "popped_val": float(val),
    "remainder_idx": [str(x) for x in s.index],
    "remainder_vals": s.tolist(),
    "remainder_name": s.name,
    "remainder_idx_name": s.index.name,
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series pop differential test");
            return;
        }
    };

    let s = Series::from_values(
        "target",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
        ],
        vec![
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
            Scalar::Float64(30.0),
        ],
    )
    .expect("s")
    .rename_axis("idx_name")
    .expect("rename_axis");

    let (popped_val, remainder) = s.pop(&IndexLabel::Utf8("b".into())).expect("pop");
    assert_eq!(
        popped_val.to_f64().unwrap(),
        oracle["popped_val"].as_f64().unwrap()
    );

    let actual_idx: Vec<String> = remainder
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_idx: Vec<String> = oracle["remainder_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_idx, oracle_idx);

    let actual_vals: Vec<f64> = remainder
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let oracle_vals: Vec<f64> = oracle["remainder_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_vals, oracle_vals);

    assert_eq!(remainder.name(), oracle["remainder_name"].as_str().unwrap());
    assert_eq!(
        remainder.index().name(),
        oracle["remainder_idx_name"].as_str()
    );
}

#[test]
fn conformance_series_clip_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([1.0, 5.0, 10.0, 15.0, 20.0], index=["a", "b", "c", "d", "e"])
clipped = s.clip(lower=5.0, upper=15.0)
res = {
    "clipped_vals": clipped.tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series clip differential test");
            return;
        }
    };

    let s = Series::from_values(
        "nums",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
            IndexLabel::Utf8("e".into()),
        ],
        vec![
            Scalar::Float64(1.0),
            Scalar::Float64(5.0),
            Scalar::Float64(10.0),
            Scalar::Float64(15.0),
            Scalar::Float64(20.0),
        ],
    )
    .expect("s");

    let clipped = s.clip(Some(5.0), Some(15.0)).expect("clip");
    let actual_vals: Vec<f64> = clipped
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let oracle_vals: Vec<f64> = oracle["clipped_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_vals, oracle_vals);
}

#[test]
fn conformance_series_between_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([1.0, 5.0, 10.0, 15.0, 20.0], index=["a", "b", "c", "d", "e"])
b_both = s.between(5.0, 15.0, inclusive="both")
b_neither = s.between(5.0, 15.0, inclusive="neither")
res = {
    "both_vals": b_both.tolist(),
    "neither_vals": b_neither.tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series between differential test");
            return;
        }
    };

    let s = Series::from_values(
        "nums",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
            IndexLabel::Utf8("e".into()),
        ],
        vec![
            Scalar::Float64(1.0),
            Scalar::Float64(5.0),
            Scalar::Float64(10.0),
            Scalar::Float64(15.0),
            Scalar::Float64(20.0),
        ],
    )
    .expect("s");

    let b_both = s
        .between(&Scalar::Float64(5.0), &Scalar::Float64(15.0), "both")
        .expect("between both");
    let actual_both_vals: Vec<bool> = b_both
        .column()
        .values()
        .iter()
        .map(|v| v.to_bool().unwrap_or(false))
        .collect();
    let oracle_both_vals: Vec<bool> = oracle["both_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(actual_both_vals, oracle_both_vals);

    let b_neither = s
        .between(&Scalar::Float64(5.0), &Scalar::Float64(15.0), "neither")
        .expect("between neither");
    let actual_neither_vals: Vec<bool> = b_neither
        .column()
        .values()
        .iter()
        .map(|v| v.to_bool().unwrap_or(false))
        .collect();
    let oracle_neither_vals: Vec<bool> = oracle["neither_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(actual_neither_vals, oracle_neither_vals);
}

#[test]
fn conformance_series_diff_and_pct_change_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([10.0, 20.0, 50.0, 100.0], index=["p0", "p1", "p2", "p3"])
diff_res = s.diff(1)
pct_res = s.pct_change(1)
res = {
    "diff_vals": [None if pd.isna(x) else float(x) for x in diff_res],
    "pct_vals": [None if pd.isna(x) else float(x) for x in pct_res],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping Series diff/pct_change differential test"
            );
            return;
        }
    };

    let s = Series::from_values(
        "series",
        vec![
            IndexLabel::Utf8("p0".into()),
            IndexLabel::Utf8("p1".into()),
            IndexLabel::Utf8("p2".into()),
            IndexLabel::Utf8("p3".into()),
        ],
        vec![
            Scalar::Float64(10.0),
            Scalar::Float64(20.0),
            Scalar::Float64(50.0),
            Scalar::Float64(100.0),
        ],
    )
    .expect("s");

    let diff_res = s.diff(1).expect("diff");
    let actual_diff_vals: Vec<Option<f64>> = diff_res
        .column()
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_f64().ok()
            }
        })
        .collect();
    let oracle_diff_vals: Vec<Option<f64>> = oracle["diff_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_diff_vals, oracle_diff_vals);

    let pct_res = s.pct_change(1).expect("pct_change");
    let actual_pct_vals: Vec<Option<f64>> = pct_res
        .column()
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_f64().ok()
            }
        })
        .collect();
    let oracle_pct_vals: Vec<Option<f64>> = oracle["pct_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_pct_vals, oracle_pct_vals);
}

#[test]
fn conformance_series_value_counts_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series(["cat", "dog", "cat", "cat"], index=[0, 1, 2, 3])
vc_default = s.value_counts()
vc_norm = s.value_counts(normalize=True)
vc_asc = s.value_counts(ascending=True)
res = {
    "default_idx": [str(x) for x in vc_default.index],
    "default_vals": vc_default.tolist(),
    "norm_vals": vc_norm.tolist(),
    "asc_vals": vc_asc.tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series value_counts differential test");
            return;
        }
    };

    let s = Series::from_values(
        "animals",
        vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ],
        vec![
            Scalar::Utf8("cat".into()),
            Scalar::Utf8("dog".into()),
            Scalar::Utf8("cat".into()),
            Scalar::Utf8("cat".into()),
        ],
    )
    .expect("s");

    let vc_default = s
        .value_counts_with_options(false, true, false, true)
        .expect("vc default");
    let actual_default_idx: Vec<String> = vc_default
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_default_idx: Vec<String> = oracle["default_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_default_idx, oracle_default_idx);

    let actual_default_vals: Vec<i64> = vc_default
        .column()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap_or(0))
        .collect();
    let oracle_default_vals: Vec<i64> = oracle["default_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_default_vals, oracle_default_vals);

    let vc_norm = s
        .value_counts_with_options(true, true, false, true)
        .expect("vc norm");
    let actual_norm_vals: Vec<f64> = vc_norm
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(0.0))
        .collect();
    let oracle_norm_vals: Vec<f64> = oracle["norm_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_norm_vals, oracle_norm_vals);

    let vc_asc = s
        .value_counts_with_options(false, true, true, true)
        .expect("vc asc");
    let actual_asc_vals: Vec<i64> = vc_asc
        .column()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap_or(0))
        .collect();
    let oracle_asc_vals: Vec<i64> = oracle["asc_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_asc_vals, oracle_asc_vals);
}

#[test]
fn conformance_series_cumulative_differential() {
    let python_code = r#"
import json, pandas as pd, numpy as np
s = pd.Series([2.0, np.nan, 4.0, 1.0])
res = {
    "cs_skip": [None if pd.isna(x) else float(x) for x in s.cumsum(skipna=True)],
    "cp_skip": [None if pd.isna(x) else float(x) for x in s.cumprod(skipna=True)],
    "cmin_skip": [None if pd.isna(x) else float(x) for x in s.cummin(skipna=True)],
    "cmax_skip": [None if pd.isna(x) else float(x) for x in s.cummax(skipna=True)],
    "cs_noskip": [None if pd.isna(x) else float(x) for x in s.cumsum(skipna=False)],
    "cp_noskip": [None if pd.isna(x) else float(x) for x in s.cumprod(skipna=False)],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series cumulative differential test");
            return;
        }
    };

    let s = Series::from_values(
        "s",
        vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(3),
        ],
        vec![
            Scalar::Float64(2.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(4.0),
            Scalar::Float64(1.0),
        ],
    )
    .expect("s");

    let cs_skip = s.cumsum_with_skipna(true).expect("cs skip");
    let actual_cs_skip: Vec<Option<f64>> = cs_skip
        .column()
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_f64().ok()
            }
        })
        .collect();
    let oracle_cs_skip: Vec<Option<f64>> = oracle["cs_skip"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_cs_skip, oracle_cs_skip);

    let cp_skip = s.cumprod_with_skipna(true).expect("cp skip");
    let actual_cp_skip: Vec<Option<f64>> = cp_skip
        .column()
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_f64().ok()
            }
        })
        .collect();
    let oracle_cp_skip: Vec<Option<f64>> = oracle["cp_skip"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_cp_skip, oracle_cp_skip);

    let cmin_skip = s.cummin_with_skipna(true).expect("cmin skip");
    let actual_cmin_skip: Vec<Option<f64>> = cmin_skip
        .column()
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_f64().ok()
            }
        })
        .collect();
    let oracle_cmin_skip: Vec<Option<f64>> = oracle["cmin_skip"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_cmin_skip, oracle_cmin_skip);

    let cmax_skip = s.cummax_with_skipna(true).expect("cmax skip");
    let actual_cmax_skip: Vec<Option<f64>> = cmax_skip
        .column()
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_f64().ok()
            }
        })
        .collect();
    let oracle_cmax_skip: Vec<Option<f64>> = oracle["cmax_skip"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_cmax_skip, oracle_cmax_skip);

    let cs_noskip = s.cumsum_with_skipna(false).expect("cs noskip");
    let actual_cs_noskip: Vec<Option<f64>> = cs_noskip
        .column()
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_f64().ok()
            }
        })
        .collect();
    let oracle_cs_noskip: Vec<Option<f64>> = oracle["cs_noskip"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_cs_noskip, oracle_cs_noskip);

    let cp_noskip = s.cumprod_with_skipna(false).expect("cp noskip");
    let actual_cp_noskip: Vec<Option<f64>> = cp_noskip
        .column()
        .values()
        .iter()
        .map(|v| {
            if v.is_missing() {
                None
            } else {
                v.to_f64().ok()
            }
        })
        .collect();
    let oracle_cp_noskip: Vec<Option<f64>> = oracle["cp_noskip"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_cp_noskip, oracle_cp_noskip);
}

#[test]
fn conformance_series_round_rank_clip_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([1.234, 5.678, 3.456, 5.678], index=['s0', 's1', 's2', 's3'])
s_lo = pd.Series([2.0, 2.0, 2.0, 2.0], index=['s0', 's1', 's2', 's3'])
s_hi = pd.Series([4.0, 4.0, 4.0, 4.0], index=['s0', 's1', 's2', 's3'])
res = {
    'round_1': [float(x) for x in s.round(1)],
    'rank_avg': [float(x) for x in s.rank(method='average')],
    'rank_dense_desc': [float(x) for x in s.rank(method='dense', ascending=False)],
    'rank_pct': [float(x) for x in s.rank(pct=True)],
    'clip_scalar': [float(x) for x in s.clip(lower=2.0, upper=4.0)],
    'clip_series': [float(x) for x in s.clip(lower=s_lo, upper=s_hi)],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping Series round/rank/clip differential test"
            );
            return;
        }
    };

    let s = Series::from_values(
        "s",
        vec![
            IndexLabel::Utf8("s0".into()),
            IndexLabel::Utf8("s1".into()),
            IndexLabel::Utf8("s2".into()),
            IndexLabel::Utf8("s3".into()),
        ],
        vec![
            Scalar::Float64(1.234),
            Scalar::Float64(5.678),
            Scalar::Float64(3.456),
            Scalar::Float64(5.678),
        ],
    )
    .expect("s");

    // 1. round
    let rd = s.round(1).expect("round");
    let actual_rd: Vec<f64> = rd
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(0.0))
        .collect();
    let oracle_rd: Vec<f64> = oracle["round_1"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_rd, oracle_rd);

    // 2. rank average
    let rk_avg = s.rank("average", true, "keep").expect("rank avg");
    let actual_rk_avg: Vec<f64> = rk_avg
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(0.0))
        .collect();
    let oracle_rk_avg: Vec<f64> = oracle["rank_avg"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_rk_avg, oracle_rk_avg);

    // 3. rank dense descending
    let rk_dense = s.rank("dense", false, "keep").expect("rank dense desc");
    let actual_rk_dense: Vec<f64> = rk_dense
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(0.0))
        .collect();
    let oracle_rk_dense: Vec<f64> = oracle["rank_dense_desc"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_rk_dense, oracle_rk_dense);

    // 4. rank pct
    let rk_pct = s
        .rank_with_pct("average", true, "keep", true)
        .expect("rank pct");
    let actual_rk_pct: Vec<f64> = rk_pct
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(0.0))
        .collect();
    let oracle_rk_pct: Vec<f64> = oracle["rank_pct"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_rk_pct, oracle_rk_pct);

    // 5. clip scalar
    let cl_sc = s.clip(Some(2.0), Some(4.0)).expect("clip scalar");
    let actual_cl_sc: Vec<f64> = cl_sc
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(0.0))
        .collect();
    let oracle_cl_sc: Vec<f64> = oracle["clip_scalar"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_cl_sc, oracle_cl_sc);

    // 6. clip with series
    let s_lo = Series::from_values(
        "s_lo",
        vec![
            IndexLabel::Utf8("s0".into()),
            IndexLabel::Utf8("s1".into()),
            IndexLabel::Utf8("s2".into()),
            IndexLabel::Utf8("s3".into()),
        ],
        vec![
            Scalar::Float64(2.0),
            Scalar::Float64(2.0),
            Scalar::Float64(2.0),
            Scalar::Float64(2.0),
        ],
    )
    .expect("s_lo");
    let s_hi = Series::from_values(
        "s_hi",
        vec![
            IndexLabel::Utf8("s0".into()),
            IndexLabel::Utf8("s1".into()),
            IndexLabel::Utf8("s2".into()),
            IndexLabel::Utf8("s3".into()),
        ],
        vec![
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
            Scalar::Float64(4.0),
        ],
    )
    .expect("s_hi");
    let cl_ser = s
        .clip_with_series(Some(&s_lo), Some(&s_hi))
        .expect("clip series");
    let actual_cl_ser: Vec<f64> = cl_ser
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(0.0))
        .collect();
    let oracle_cl_ser: Vec<f64> = oracle["clip_series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_cl_ser, oracle_cl_ser);
}

#[test]
fn conformance_series_dropna_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([1.0, float('nan'), 3.0, float('nan'), 5.0], index=['a', 'b', 'c', 'd', 'e'])
res = {
    'dropna_vals': [float(x) for x in s.dropna()],
    'dropna_idx': list(s.dropna().index),
    'dropna_ign_idx': list(s.dropna(ignore_index=True).index),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series dropna differential test");
            return;
        }
    };

    let s = Series::from_values(
        "s",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
            IndexLabel::Utf8("e".into()),
        ],
        vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(3.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(5.0),
        ],
    )
    .expect("s");

    let dropped = s.dropna().expect("dropna");
    let actual_vals: Vec<f64> = dropped
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(0.0))
        .collect();
    let oracle_vals: Vec<f64> = oracle["dropna_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_vals, oracle_vals);

    let actual_idx: Vec<String> = dropped
        .index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Utf8(s) => s.clone(),
            other => other.to_string(),
        })
        .collect();
    let oracle_idx: Vec<String> = oracle["dropna_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_idx, oracle_idx);

    let dropped_ign = match dropped.reset_index(true).expect("reset index") {
        fp_frame::SeriesResetIndexResult::Series(ser) => ser,
        fp_frame::SeriesResetIndexResult::DataFrame(_) => unreachable!(),
    };
    let actual_ign_idx: Vec<i64> = dropped_ign
        .index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(i) => *i,
            _ => -1,
        })
        .collect();
    let oracle_ign_idx: Vec<i64> = oracle["dropna_ign_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_ign_idx, oracle_ign_idx);
}

#[test]
fn conformance_series_fillna_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([1.0, float('nan'), float('nan'), 4.0], index=['a', 'b', 'c', 'd'])
other_s = pd.Series([99.0, 88.0, 77.0, 66.0], index=['d', 'c', 'b', 'a'])
res = {
    'fill_sc': [float(x) for x in s.fillna(0.0)],
    'fill_lim': [None if pd.isna(x) else float(x) for x in s.fillna(0.0, limit=1)],
    'fill_ffill': [float(x) for x in s.ffill()],
    'fill_bfill': [float(x) for x in s.bfill()],
    'fill_other': [float(x) for x in s.fillna(other_s)],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series fillna differential test");
            return;
        }
    };

    let s = Series::from_values(
        "s",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
        ],
        vec![
            Scalar::Float64(1.0),
            Scalar::Null(NullKind::NaN),
            Scalar::Null(NullKind::NaN),
            Scalar::Float64(4.0),
        ],
    )
    .expect("s");

    // 1. scalar fill
    let f_sc = s.fillna(&Scalar::Float64(0.0)).expect("fill sc");
    let actual_f_sc: Vec<f64> = f_sc
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(-1.0))
        .collect();
    let oracle_f_sc: Vec<f64> = oracle["fill_sc"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_f_sc, oracle_f_sc);

    // 2. limit fill
    let f_lim = s.fillna_limit(&Scalar::Float64(0.0), 1).expect("fill lim");
    let actual_f_lim: Vec<Option<f64>> = f_lim
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let oracle_f_lim: Vec<Option<f64>> = oracle["fill_lim"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_f_lim, oracle_f_lim);

    // 3. ffill
    let f_ffill = s.ffill(None).expect("ffill");
    let actual_f_ffill: Vec<f64> = f_ffill
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(-1.0))
        .collect();
    let oracle_f_ffill: Vec<f64> = oracle["fill_ffill"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_f_ffill, oracle_f_ffill);

    // 4. bfill
    let f_bfill = s.bfill(None).expect("bfill");
    let actual_f_bfill: Vec<f64> = f_bfill
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(-1.0))
        .collect();
    let oracle_f_bfill: Vec<f64> = oracle["fill_bfill"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_f_bfill, oracle_f_bfill);

    // 5. other series fill (aligned)
    let other_s = Series::from_values(
        "other",
        vec![
            IndexLabel::Utf8("d".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("a".into()),
        ],
        vec![
            Scalar::Float64(99.0),
            Scalar::Float64(88.0),
            Scalar::Float64(77.0),
            Scalar::Float64(66.0),
        ],
    )
    .expect("other_s");
    let aligned_other = other_s
        .reindex(s.index().labels().to_vec())
        .expect("reindex");
    let f_other = s.fillna_with_series(&aligned_other).expect("fill other");
    let actual_f_other: Vec<f64> = f_other
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap_or(-1.0))
        .collect();
    let oracle_f_other: Vec<f64> = oracle["fill_other"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_f_other, oracle_f_other);
}

#[test]
fn conformance_series_replace_differential() {
    let python_code = r#"
import json, pandas as pd
s = pd.Series([1, 2, 3, 2, 1], index=['a', 'b', 'c', 'd', 'e'])
s_str = pd.Series(['apple1', 'banana2', 'apricot3', 'cherry4'], index=['w', 'x', 'y', 'z'])
res = {
    'scalar_repl': [int(x) for x in s.replace(1, 10)],
    'list_to_list': [int(x) for x in s.replace([1, 2], [10, 20])],
    'list_to_scalar': [int(x) for x in s.replace([1, 2], 99)],
    'dict_repl': [int(x) for x in s.replace({1: 10, 2: 20})],
    'regex_repl': [str(x) for x in s_str.replace(r'^([a-z]+)(\d)$', r'fruit_\1', regex=True)],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping Series replace differential test");
            return;
        }
    };

    let s = Series::from_values(
        "s",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
            IndexLabel::Utf8("e".into()),
        ],
        vec![
            Scalar::Int64(1),
            Scalar::Int64(2),
            Scalar::Int64(3),
            Scalar::Int64(2),
            Scalar::Int64(1),
        ],
    )
    .expect("s");

    let s_str = Series::from_values(
        "s_str",
        vec![
            IndexLabel::Utf8("w".into()),
            IndexLabel::Utf8("x".into()),
            IndexLabel::Utf8("y".into()),
            IndexLabel::Utf8("z".into()),
        ],
        vec![
            Scalar::Utf8("apple1".into()),
            Scalar::Utf8("banana2".into()),
            Scalar::Utf8("apricot3".into()),
            Scalar::Utf8("cherry4".into()),
        ],
    )
    .expect("s_str");

    // 1. scalar replace
    let r_sc = s
        .replace(&[(Scalar::Int64(1), Scalar::Int64(10))])
        .expect("sc replace");
    let actual_sc: Vec<i64> = r_sc
        .column()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap_or(-1))
        .collect();
    let oracle_sc: Vec<i64> = oracle["scalar_repl"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_sc, oracle_sc);

    // 2. list to list replace
    let r_l2l = s
        .replace(&[
            (Scalar::Int64(1), Scalar::Int64(10)),
            (Scalar::Int64(2), Scalar::Int64(20)),
        ])
        .expect("l2l replace");
    let actual_l2l: Vec<i64> = r_l2l
        .column()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap_or(-1))
        .collect();
    let oracle_l2l: Vec<i64> = oracle["list_to_list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_l2l, oracle_l2l);

    // 3. list to scalar replace
    let r_l2s = s
        .replace(&[
            (Scalar::Int64(1), Scalar::Int64(99)),
            (Scalar::Int64(2), Scalar::Int64(99)),
        ])
        .expect("l2s replace");
    let actual_l2s: Vec<i64> = r_l2s
        .column()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap_or(-1))
        .collect();
    let oracle_l2s: Vec<i64> = oracle["list_to_scalar"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_l2s, oracle_l2s);

    // 4. dict replace
    let r_dict = s
        .replace(&[
            (Scalar::Int64(1), Scalar::Int64(10)),
            (Scalar::Int64(2), Scalar::Int64(20)),
        ])
        .expect("dict replace");
    let actual_dict: Vec<i64> = r_dict
        .column()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap_or(-1))
        .collect();
    let oracle_dict: Vec<i64> = oracle["dict_repl"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_dict, oracle_dict);

    // 5. regex replace
    let r_reg = s_str
        .replace_regex(r"^([a-z]+)(\d)$", "fruit_$1")
        .expect("regex replace");
    let actual_reg: Vec<String> = r_reg
        .column()
        .values()
        .iter()
        .map(|v| v.to_string())
        .collect();
    let oracle_reg: Vec<String> = oracle["regex_repl"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_reg, oracle_reg);
}

#[test]
fn conformance_series_sort_values_and_duplicates_differential() {
    let python_code = r#"
import json, pandas as pd, numpy as np
s = pd.Series([2.0, 1.0, 2.0, np.nan], index=['a', 'b', 'c', 'd'], name='vals')

s_sort_asc = s.sort_values(ascending=True, na_position='last')
s_sort_desc_na_first = s.sort_values(ascending=False, na_position='first')
s_sort_ign_idx = s.sort_values(ascending=True, ignore_index=True)

s_dedup_first = s.drop_duplicates(keep='first')
s_dedup_last = s.drop_duplicates(keep='last')
s_dedup_none = s.drop_duplicates(keep=False, ignore_index=True)

s_dup_first = s.duplicated(keep='first')
s_dup_none = s.duplicated(keep=False)

res = {
    'sort_asc_idx': list(s_sort_asc.index),
    'sort_asc_vals': [None if pd.isna(x) else float(x) for x in s_sort_asc],
    'sort_desc_na_first_idx': list(s_sort_desc_na_first.index),
    'sort_desc_na_first_vals': [None if pd.isna(x) else float(x) for x in s_sort_desc_na_first],
    'sort_ign_idx': [int(x) for x in s_sort_ign_idx.index],
    'sort_ign_vals': [None if pd.isna(x) else float(x) for x in s_sort_ign_idx],
    'dedup_first_idx': list(s_dedup_first.index),
    'dedup_first_vals': [None if pd.isna(x) else float(x) for x in s_dedup_first],
    'dedup_last_idx': list(s_dedup_last.index),
    'dedup_last_vals': [None if pd.isna(x) else float(x) for x in s_dedup_last],
    'dedup_none_idx': [int(x) for x in s_dedup_none.index],
    'dedup_none_vals': [None if pd.isna(x) else float(x) for x in s_dedup_none],
    'dup_first': [bool(x) for x in s_dup_first],
    'dup_none': [bool(x) for x in s_dup_none],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping Series sort_values/duplicates differential test"
            );
            return;
        }
    };

    let s = Series::from_values(
        "vals",
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
        ],
        vec![
            Scalar::Float64(2.0),
            Scalar::Float64(1.0),
            Scalar::Float64(2.0),
            Scalar::Null(NullKind::NaN),
        ],
    )
    .expect("s");

    // 1. sort_values ascending na_position='last'
    let s_asc = s.sort_values_na(true, "last").expect("sort asc");
    let actual_asc_idx: Vec<String> = s_asc
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_asc_idx: Vec<String> = oracle["sort_asc_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_asc_idx, oracle_asc_idx);
    let actual_asc_vals: Vec<Option<f64>> = s_asc
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let oracle_asc_vals: Vec<Option<f64>> = oracle["sort_asc_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_asc_vals, oracle_asc_vals);

    // 2. sort_values descending na_position='first'
    let s_desc = s.sort_values_na(false, "first").expect("sort desc");
    let actual_desc_idx: Vec<String> = s_desc
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_desc_idx: Vec<String> = oracle["sort_desc_na_first_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_desc_idx, oracle_desc_idx);
    let actual_desc_vals: Vec<Option<f64>> = s_desc
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let oracle_desc_vals: Vec<Option<f64>> = oracle["sort_desc_na_first_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_desc_vals, oracle_desc_vals);

    // 3. sort_values ignore_index
    let s_ign = s
        .sort_values_na(true, "last")
        .expect("sort ign")
        .reset_index(true)
        .expect("reset")
        .into_series()
        .expect("into_series");
    let actual_ign_idx: Vec<i64> = s_ign
        .index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(i) => *i,
            _ => -1,
        })
        .collect();
    let oracle_ign_idx: Vec<i64> = oracle["sort_ign_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_ign_idx, oracle_ign_idx);
    let actual_ign_vals: Vec<Option<f64>> = s_ign
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let oracle_ign_vals: Vec<Option<f64>> = oracle["sort_ign_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_ign_vals, oracle_ign_vals);

    // 4. drop_duplicates keep='first'
    let s_dedup_first = s
        .drop_duplicates_keep(fp_index::DuplicateKeep::First)
        .expect("dedup first");
    let actual_df_idx: Vec<String> = s_dedup_first
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_df_idx: Vec<String> = oracle["dedup_first_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_df_idx, oracle_df_idx);
    let actual_df_vals: Vec<Option<f64>> = s_dedup_first
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let oracle_df_vals: Vec<Option<f64>> = oracle["dedup_first_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_df_vals, oracle_df_vals);

    // 5. drop_duplicates keep='last'
    let s_dedup_last = s
        .drop_duplicates_keep(fp_index::DuplicateKeep::Last)
        .expect("dedup last");
    let actual_dl_idx: Vec<String> = s_dedup_last
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_dl_idx: Vec<String> = oracle["dedup_last_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_dl_idx, oracle_dl_idx);
    let actual_dl_vals: Vec<Option<f64>> = s_dedup_last
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let oracle_dl_vals: Vec<Option<f64>> = oracle["dedup_last_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_dl_vals, oracle_dl_vals);

    // 6. drop_duplicates keep=False, ignore_index=True
    let s_dedup_none = s
        .drop_duplicates_keep(fp_index::DuplicateKeep::None)
        .expect("dedup none")
        .reset_index(true)
        .expect("reset")
        .into_series()
        .expect("into_series");
    let actual_dn_idx: Vec<i64> = s_dedup_none
        .index()
        .labels()
        .iter()
        .map(|l| match l {
            IndexLabel::Int64(i) => *i,
            _ => -1,
        })
        .collect();
    let oracle_dn_idx: Vec<i64> = oracle["dedup_none_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_dn_idx, oracle_dn_idx);
    let actual_dn_vals: Vec<Option<f64>> = s_dedup_none
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let oracle_dn_vals: Vec<Option<f64>> = oracle["dedup_none_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(actual_dn_vals, oracle_dn_vals);

    // 7. duplicated keep='first'
    let s_dup_first = s
        .duplicated_keep(fp_index::DuplicateKeep::First)
        .expect("dup first");
    let actual_dup_first: Vec<bool> = s_dup_first
        .column()
        .values()
        .iter()
        .map(|v| v.to_bool().unwrap())
        .collect();
    let oracle_dup_first: Vec<bool> = oracle["dup_first"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(actual_dup_first, oracle_dup_first);

    // 8. duplicated keep=False
    let s_dup_none = s
        .duplicated_keep(fp_index::DuplicateKeep::None)
        .expect("dup none");
    let actual_dup_none: Vec<bool> = s_dup_none
        .column()
        .values()
        .iter()
        .map(|v| v.to_bool().unwrap())
        .collect();
    let oracle_dup_none: Vec<bool> = oracle["dup_none"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(actual_dup_none, oracle_dup_none);
}
