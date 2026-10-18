//! Index parity-matrix conformance suite (br-frankenpandas-jl63).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares the
//! existing Rust Index behavior with live upstream pandas for an edge-case
//! input: empty indexes, single labels, duplicate labels, mixed labels,
//! NA-like string labels, and extreme integer labels.

use fp_index::{IndexLabel, IntervalIndex, RangeIndex};
use fp_types::IntervalClosed;

use super::{
    CaseStatus, HarnessConfig, HarnessError, OracleMode, PacketFixture, ResolvedExpected,
    SuiteOptions, capture_live_oracle_expected,
};

fn strict_config() -> HarnessConfig {
    let mut cfg = HarnessConfig::default_paths();
    // br-frankenpandas-l7r1p: without this every test in this file SKIPS. The
    // legacy oracle root (`legacy_pandas_code/pandas`) does not exist on this
    // host, so `capture_live_oracle_expected` returns OracleUnavailable and
    // `check_index_fixture` returns before comparing anything -- the tests
    // report green while executing no differential at all. The live_oracle_*
    // suites already opt into the SYSTEM pandas (2.2.3) for exactly this
    // reason; the conformance_* suites never did.
    cfg.allow_system_pandas_fallback = true;
    cfg
}

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(
            ResolvedExpected::Alignment(_)
            | ResolvedExpected::Bool(_)
            | ResolvedExpected::Positions(_),
        ) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping Index conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_index_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("index oracle") {
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
        "pandas Index parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

#[test]
fn conformance_index_align_union_empty_pair() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-ALIGN-001",
        "case_id": "index_align_union_empty_pair",
        "mode": "strict",
        "operation": "index_align_union",
        "oracle_source": "live_legacy_pandas",
        "left": { "name": "left", "index": [], "values": [] },
        "right": { "name": "right", "index": [], "values": [] }
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_align_union_single_label() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-ALIGN-002",
        "case_id": "index_align_union_single_label",
        "mode": "strict",
        "operation": "index_align_union",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "left",
            "index": [{ "kind": "int64", "value": 7 }],
            "values": []
        },
        "right": {
            "name": "right",
            "index": [{ "kind": "int64", "value": 7 }],
            "values": []
        }
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_align_union_mixed_labels_preserves_order() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-ALIGN-003",
        "case_id": "index_align_union_mixed_labels_preserves_order",
        "mode": "strict",
        "operation": "index_align_union",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "left",
            "index": [
                { "kind": "utf8", "value": "b" },
                { "kind": "int64", "value": 1 }
            ],
            "values": []
        },
        "right": {
            "name": "right",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" }
            ],
            "values": []
        }
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_align_union_duplicate_right_positions() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-ALIGN-004",
        "case_id": "index_align_union_duplicate_right_positions",
        "mode": "strict",
        "operation": "index_align_union",
        "oracle_source": "live_legacy_pandas",
        "left": {
            "name": "left",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" }
            ],
            "values": []
        },
        "right": {
            "name": "right",
            "index": [
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "b" },
                { "kind": "utf8", "value": "c" }
            ],
            "values": []
        }
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_has_duplicates_empty() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-DUPS-001",
        "case_id": "index_has_duplicates_empty",
        "mode": "strict",
        "operation": "index_has_duplicates",
        "oracle_source": "live_legacy_pandas",
        "index": []
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_has_duplicates_na_like_strings() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-DUPS-002",
        "case_id": "index_has_duplicates_na_like_strings",
        "mode": "strict",
        "operation": "index_has_duplicates",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "utf8", "value": "NaN" },
            { "kind": "utf8", "value": "x" },
            { "kind": "utf8", "value": "NaN" }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_first_positions_duplicate_ints() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-POS-001",
        "case_id": "index_first_positions_duplicate_ints",
        "mode": "strict",
        "operation": "index_first_positions",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "int64", "value": 2 },
            { "kind": "int64", "value": 1 },
            { "kind": "int64", "value": 2 },
            { "kind": "int64", "value": 3 }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_first_positions_mixed_labels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-POS-002",
        "case_id": "index_first_positions_mixed_labels",
        "mode": "strict",
        "operation": "index_first_positions",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "utf8", "value": "alpha" },
            { "kind": "int64", "value": 1 },
            { "kind": "utf8", "value": "alpha" },
            { "kind": "utf8", "value": "missing" },
            { "kind": "int64", "value": 1 }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_first_positions_repeated_null_labels() {
    // br-frankenpandas-l4xuh: the case the old oracle got WRONG. It resolved
    // label identity with a Python dict, and because `label_from_json` mints a
    // fresh `float("nan")` per element and `nan != nan`, each NaN became its own
    // key -- [0, 1, 2] where pandas says [0, 0, 2]. Live pandas 2.2.3:
    //
    //     >>> pd.Index([nan, nan, 1.0]).get_indexer_for([nan])
    //     array([0, 1])
    //     >>> pd.Index([nan, 'a', nan]).has_duplicates
    //     True
    //
    // NaN is a matchable, duplicable index label. This pins that, and it is
    // only expressible at all because label_to_json/label_from_json learned the
    // null kind (br-frankenpandas-l7r1p).
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-POS-003",
        "case_id": "index_first_positions_repeated_null_labels",
        "mode": "strict",
        "operation": "index_first_positions",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "null", "value": "na_n" },
            { "kind": "null", "value": "na_n" },
            { "kind": "float64", "value": 1.0 },
            { "kind": "null", "value": "na_n" }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_monotonic_increasing_duplicate_plateau() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-MONO-001",
        "case_id": "index_monotonic_increasing_duplicate_plateau",
        "mode": "strict",
        "operation": "index_is_monotonic_increasing",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "int64", "value": -1 },
            { "kind": "int64", "value": -1 },
            { "kind": "int64", "value": 0 },
            { "kind": "int64", "value": 5 }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

#[test]
fn conformance_index_monotonic_decreasing_extreme_ints() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-INDEX-MONO-002",
        "case_id": "index_monotonic_decreasing_extreme_ints",
        "mode": "strict",
        "operation": "index_is_monotonic_decreasing",
        "oracle_source": "live_legacy_pandas",
        "index": [
            { "kind": "int64", "value": i64::MAX },
            { "kind": "int64", "value": 0 },
            { "kind": "int64", "value": i64::MIN }
        ]
    }))
    .expect("fixture");
    check_index_fixture(fixture);
}

fn run_pandas_oracle_eval(code: &str) -> Option<serde_json::Value> {
    use std::io::Write;
    if std::env::var("FP_SKIP_LIVE_ORACLE").as_deref() == Ok("1") {
        return None;
    }
    let cfg = strict_config();
    let python = &cfg.python_bin;
    let mut child = std::process::Command::new(python)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(code.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    assert!(
        output.status.success(),
        "pandas oracle script failed (status={}):\nstderr={}\nstdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    Some(serde_json::from_slice(&output.stdout).expect("valid index oracle json"))
}

fn index_to_f64_vec(idx: &fp_index::Index) -> Vec<f64> {
    idx.labels()
        .iter()
        .filter_map(|l| match l {
            IndexLabel::Float64(f) => Some(f.0),
            _ => None,
        })
        .collect()
}

#[test]
fn conformance_interval_index_breaks_differential() {
    let python_code = r#"
import pandas as pd, json
idx = pd.IntervalIndex.from_breaks([0.0, 1.5, 3.0, 5.0], closed='right')
res = {
    'left': idx.left.tolist(),
    'right': idx.right.tolist(),
    'mid': idx.mid.tolist(),
    'length': idx.length.tolist(),
    'closed_left': bool(idx.closed_left),
    'closed_right': bool(idx.closed_right),
    'open_left': bool(idx.open_left),
    'open_right': bool(idx.open_right),
    'is_overlapping': bool(idx.is_overlapping),
    'is_unique': bool(idx.is_unique),
    'is_monotonic_increasing': bool(idx.is_monotonic_increasing),
    'is_monotonic_decreasing': bool(idx.is_monotonic_decreasing),
    'is_non_overlapping_monotonic': bool(idx.is_non_overlapping_monotonic),
    'contains_1_5': idx.contains(1.5).tolist(),
    'contains_2_0': idx.contains(2.0).tolist(),
    'get_loc_2_0': int(idx.get_loc(2.0)),
    'get_indexer': [int(x) for x in idx.get_indexer([0.5, 2.0, 4.0, 6.0])],
    'to_tuples': [list(t) for t in idx.to_tuples()]
}
print(json.dumps(res))
"#;
    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping IntervalIndex differential test");
            return;
        }
    };

    let ii = IntervalIndex::from_breaks(&[0.0, 1.5, 3.0, 5.0], IntervalClosed::Right)
        .expect("from_breaks");

    let left_vals = index_to_f64_vec(&ii.left());
    let oracle_left: Vec<f64> = oracle["left"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(left_vals, oracle_left);

    let right_vals = index_to_f64_vec(&ii.right());
    let oracle_right: Vec<f64> = oracle["right"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(right_vals, oracle_right);

    let mid_vals = index_to_f64_vec(&ii.mid());
    let oracle_mid: Vec<f64> = oracle["mid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(mid_vals, oracle_mid);

    let len_vals = index_to_f64_vec(&ii.length());
    let oracle_len: Vec<f64> = oracle["length"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(len_vals, oracle_len);

    assert_eq!(ii.closed_left(), oracle["closed_left"].as_bool().unwrap());
    assert_eq!(ii.closed_right(), oracle["closed_right"].as_bool().unwrap());
    assert_eq!(ii.open_left(), oracle["open_left"].as_bool().unwrap());
    assert_eq!(ii.open_right(), oracle["open_right"].as_bool().unwrap());
    assert_eq!(
        ii.is_overlapping(),
        oracle["is_overlapping"].as_bool().unwrap()
    );
    assert_eq!(ii.is_unique(), oracle["is_unique"].as_bool().unwrap());
    assert_eq!(
        ii.is_monotonic_increasing(),
        oracle["is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        ii.is_monotonic_decreasing(),
        oracle["is_monotonic_decreasing"].as_bool().unwrap()
    );
    assert_eq!(
        ii.is_non_overlapping_monotonic(),
        oracle["is_non_overlapping_monotonic"].as_bool().unwrap()
    );

    let contains_1_5 = ii.contains(1.5);
    let oracle_contains_1_5: Vec<bool> = oracle["contains_1_5"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(contains_1_5, oracle_contains_1_5);

    let contains_2_0 = ii.contains(2.0);
    let oracle_contains_2_0: Vec<bool> = oracle["contains_2_0"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(contains_2_0, oracle_contains_2_0);

    assert_eq!(
        ii.get_loc(2.0).unwrap(),
        oracle["get_loc_2_0"].as_u64().unwrap() as usize
    );

    let indexer_vals: Vec<i64> = ii
        .get_indexer(&[0.5, 2.0, 4.0, 6.0])
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as i64))
        .collect();
    let oracle_indexer: Vec<i64> = oracle["get_indexer"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(indexer_vals, oracle_indexer);

    let tuples = ii.to_tuples();
    let oracle_tuples: Vec<(f64, f64)> = oracle["to_tuples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            let arr = pair.as_array().unwrap();
            (arr[0].as_f64().unwrap(), arr[1].as_f64().unwrap())
        })
        .collect();
    assert_eq!(tuples, oracle_tuples);
}

#[test]
fn conformance_interval_index_tuples_overlapping_both_differential() {
    let python_code = r#"
import pandas as pd, json
idx = pd.IntervalIndex.from_tuples([(0.0, 2.0), (1.0, 3.0)], closed='both')
res = {
    'is_overlapping': bool(idx.is_overlapping),
    'is_unique': bool(idx.is_unique),
    'is_monotonic_increasing': bool(idx.is_monotonic_increasing),
    'is_monotonic_decreasing': bool(idx.is_monotonic_decreasing),
    'is_non_overlapping_monotonic': bool(idx.is_non_overlapping_monotonic),
    'to_tuples': [list(t) for t in idx.to_tuples()]
}
print(json.dumps(res))
"#;
    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping IntervalIndex differential test");
            return;
        }
    };

    let ii = IntervalIndex::from_tuples(&[(0.0, 2.0), (1.0, 3.0)], IntervalClosed::Both);
    assert_eq!(
        ii.is_overlapping(),
        oracle["is_overlapping"].as_bool().unwrap()
    );
    assert_eq!(ii.is_unique(), oracle["is_unique"].as_bool().unwrap());
    assert_eq!(
        ii.is_monotonic_increasing(),
        oracle["is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        ii.is_monotonic_decreasing(),
        oracle["is_monotonic_decreasing"].as_bool().unwrap()
    );
    assert_eq!(
        ii.is_non_overlapping_monotonic(),
        oracle["is_non_overlapping_monotonic"].as_bool().unwrap()
    );

    let tuples = ii.to_tuples();
    let oracle_tuples: Vec<(f64, f64)> = oracle["to_tuples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            let arr = pair.as_array().unwrap();
            (arr[0].as_f64().unwrap(), arr[1].as_f64().unwrap())
        })
        .collect();
    assert_eq!(tuples, oracle_tuples);
}

#[test]
fn conformance_interval_index_arrays_neither_differential() {
    let python_code = r#"
import pandas as pd, json
idx = pd.IntervalIndex.from_arrays([0.0, 2.0], [2.0, 4.0], closed='neither')
res = {
    'closed_left': bool(idx.closed_left),
    'closed_right': bool(idx.closed_right),
    'open_left': bool(idx.open_left),
    'open_right': bool(idx.open_right),
    'dtype': str(idx.dtype),
    'to_tuples': [list(t) for t in idx.to_tuples()]
}
print(json.dumps(res))
"#;
    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping IntervalIndex differential test");
            return;
        }
    };

    let ii = IntervalIndex::from_arrays(&[0.0, 2.0], &[2.0, 4.0], IntervalClosed::Neither)
        .expect("from_arrays");
    assert_eq!(ii.closed_left(), oracle["closed_left"].as_bool().unwrap());
    assert_eq!(ii.closed_right(), oracle["closed_right"].as_bool().unwrap());
    assert_eq!(ii.open_left(), oracle["open_left"].as_bool().unwrap());
    assert_eq!(ii.open_right(), oracle["open_right"].as_bool().unwrap());
    assert_eq!(ii.dtype(), oracle["dtype"].as_str().unwrap());

    let tuples = ii.to_tuples();
    let oracle_tuples: Vec<(f64, f64)> = oracle["to_tuples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            let arr = pair.as_array().unwrap();
            (arr[0].as_f64().unwrap(), arr[1].as_f64().unwrap())
        })
        .collect();
    assert_eq!(tuples, oracle_tuples);
}

#[test]
fn conformance_interval_index_decreasing_differential() {
    let python_code = r#"
import pandas as pd, json
idx = pd.IntervalIndex.from_tuples([(3.0, 5.0), (1.0, 3.0), (0.0, 1.0)], closed='left')
res = {
    'is_overlapping': bool(idx.is_overlapping),
    'is_unique': bool(idx.is_unique),
    'is_monotonic_increasing': bool(idx.is_monotonic_increasing),
    'is_monotonic_decreasing': bool(idx.is_monotonic_decreasing),
    'is_non_overlapping_monotonic': bool(idx.is_non_overlapping_monotonic),
    'closed_left': bool(idx.closed_left),
    'closed_right': bool(idx.closed_right)
}
print(json.dumps(res))
"#;
    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping IntervalIndex differential test");
            return;
        }
    };

    let ii =
        IntervalIndex::from_tuples(&[(3.0, 5.0), (1.0, 3.0), (0.0, 1.0)], IntervalClosed::Left);
    assert_eq!(
        ii.is_overlapping(),
        oracle["is_overlapping"].as_bool().unwrap()
    );
    assert_eq!(ii.is_unique(), oracle["is_unique"].as_bool().unwrap());
    assert_eq!(
        ii.is_monotonic_increasing(),
        oracle["is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        ii.is_monotonic_decreasing(),
        oracle["is_monotonic_decreasing"].as_bool().unwrap()
    );
    assert_eq!(
        ii.is_non_overlapping_monotonic(),
        oracle["is_non_overlapping_monotonic"].as_bool().unwrap()
    );
    assert_eq!(ii.closed_left(), oracle["closed_left"].as_bool().unwrap());
    assert_eq!(ii.closed_right(), oracle["closed_right"].as_bool().unwrap());
}

#[test]
fn conformance_range_index_ascending_differential() {
    let python_code = r#"
import pandas as pd, json
idx = pd.RangeIndex(2, 10, 2, name="my_range")
res = {
    "start": int(idx.start),
    "stop": int(idx.stop),
    "step": int(idx.step),
    "name": idx.name,
    "len": len(idx),
    "is_monotonic_increasing": bool(idx.is_monotonic_increasing),
    "is_monotonic_decreasing": bool(idx.is_monotonic_decreasing),
    "is_unique": bool(idx.is_unique),
    "has_duplicates": bool(idx.has_duplicates),
    "min": int(idx.min()),
    "max": int(idx.max()),
    "argmax": int(idx.argmax()),
    "argmin": int(idx.argmin()),
    "argsort": [int(x) for x in idx.argsort()],
    "all": bool(idx.all()),
    "any": bool(idx.any()),
    "hasnans": bool(idx.hasnans),
    "nlevels": int(idx.nlevels),
    "to_list": idx.tolist(),
    "contains_2": bool(2 in idx),
    "contains_6": bool(6 in idx),
    "contains_8": bool(8 in idx),
    "contains_0": bool(0 in idx),
    "contains_10": bool(10 in idx),
    "contains_5": bool(5 in idx),
    "get_loc_2": int(idx.get_loc(2)),
    "get_loc_6": int(idx.get_loc(6)),
    "get_loc_8": int(idx.get_loc(8)),
    "slice_bound_left": int(idx.get_slice_bound(4, "left")),
    "slice_bound_right": int(idx.get_slice_bound(4, "right"))
}
print(json.dumps(res))
"#;
    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping RangeIndex differential test");
            return;
        }
    };

    let r = RangeIndex::from_range(2, 10, 2)
        .expect("from_range")
        .set_name("my_range");

    assert_eq!(r.start(), oracle["start"].as_i64().unwrap());
    assert_eq!(r.stop(), oracle["stop"].as_i64().unwrap());
    assert_eq!(r.step(), oracle["step"].as_i64().unwrap());
    assert_eq!(r.name().map(|n| n.as_str()), Some("my_range"));
    assert_eq!(r.len(), oracle["len"].as_u64().unwrap() as usize);
    assert_eq!(
        r.is_monotonic_increasing(),
        oracle["is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        r.is_monotonic_decreasing(),
        oracle["is_monotonic_decreasing"].as_bool().unwrap()
    );
    assert_eq!(r.is_unique(), oracle["is_unique"].as_bool().unwrap());
    assert_eq!(
        r.has_duplicates(),
        oracle["has_duplicates"].as_bool().unwrap()
    );
    assert_eq!(r.min().unwrap(), oracle["min"].as_i64().unwrap());
    assert_eq!(r.max().unwrap(), oracle["max"].as_i64().unwrap());
    assert_eq!(
        r.argmax().unwrap(),
        oracle["argmax"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.argmin().unwrap(),
        oracle["argmin"].as_u64().unwrap() as usize
    );
    let argsort: Vec<usize> = oracle["argsort"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(r.argsort(), argsort);
    assert_eq!(r.all(), oracle["all"].as_bool().unwrap());
    assert_eq!(r.any(), oracle["any"].as_bool().unwrap());
    assert_eq!(r.hasnans(), oracle["hasnans"].as_bool().unwrap());
    assert_eq!(r.nlevels(), oracle["nlevels"].as_u64().unwrap() as usize);
    let to_list: Vec<i64> = oracle["to_list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(r.to_list(), to_list);
    assert_eq!(r.contains(2), oracle["contains_2"].as_bool().unwrap());
    assert_eq!(r.contains(6), oracle["contains_6"].as_bool().unwrap());
    assert_eq!(r.contains(8), oracle["contains_8"].as_bool().unwrap());
    assert_eq!(r.contains(0), oracle["contains_0"].as_bool().unwrap());
    assert_eq!(r.contains(10), oracle["contains_10"].as_bool().unwrap());
    assert_eq!(r.contains(5), oracle["contains_5"].as_bool().unwrap());
    assert_eq!(
        r.get_loc(2).unwrap(),
        oracle["get_loc_2"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.get_loc(6).unwrap(),
        oracle["get_loc_6"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.get_loc(8).unwrap(),
        oracle["get_loc_8"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.get_slice_bound(4, "left").unwrap(),
        oracle["slice_bound_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.get_slice_bound(4, "right").unwrap(),
        oracle["slice_bound_right"].as_u64().unwrap() as usize
    );
}

#[test]
fn conformance_range_index_descending_differential() {
    let python_code = r#"
import pandas as pd, json
idx = pd.RangeIndex(10, 0, -2, name="desc")
res = {
    "start": int(idx.start),
    "stop": int(idx.stop),
    "step": int(idx.step),
    "len": len(idx),
    "is_monotonic_increasing": bool(idx.is_monotonic_increasing),
    "is_monotonic_decreasing": bool(idx.is_monotonic_decreasing),
    "min": int(idx.min()),
    "max": int(idx.max()),
    "argmax": int(idx.argmax()),
    "argmin": int(idx.argmin()),
    "argsort": [int(x) for x in idx.argsort()],
    "to_list": idx.tolist(),
    "contains_10": bool(10 in idx),
    "contains_2": bool(2 in idx),
    "contains_0": bool(0 in idx),
    "contains_12": bool(12 in idx),
    "get_loc_10": int(idx.get_loc(10)),
    "get_loc_6": int(idx.get_loc(6)),
    "get_loc_2": int(idx.get_loc(2))
}
print(json.dumps(res))
"#;
    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping RangeIndex differential test");
            return;
        }
    };

    let r = RangeIndex::new(10, 0, -2).expect("descending range");
    assert_eq!(r.start(), oracle["start"].as_i64().unwrap());
    assert_eq!(r.stop(), oracle["stop"].as_i64().unwrap());
    assert_eq!(r.step(), oracle["step"].as_i64().unwrap());
    assert_eq!(r.len(), oracle["len"].as_u64().unwrap() as usize);
    assert_eq!(
        r.is_monotonic_increasing(),
        oracle["is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        r.is_monotonic_decreasing(),
        oracle["is_monotonic_decreasing"].as_bool().unwrap()
    );
    assert_eq!(r.min().unwrap(), oracle["min"].as_i64().unwrap());
    assert_eq!(r.max().unwrap(), oracle["max"].as_i64().unwrap());
    assert_eq!(
        r.argmax().unwrap(),
        oracle["argmax"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.argmin().unwrap(),
        oracle["argmin"].as_u64().unwrap() as usize
    );
    let argsort: Vec<usize> = oracle["argsort"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(r.argsort(), argsort);
    let to_list: Vec<i64> = oracle["to_list"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(r.to_list(), to_list);
    assert_eq!(r.contains(10), oracle["contains_10"].as_bool().unwrap());
    assert_eq!(r.contains(2), oracle["contains_2"].as_bool().unwrap());
    assert_eq!(r.contains(0), oracle["contains_0"].as_bool().unwrap());
    assert_eq!(r.contains(12), oracle["contains_12"].as_bool().unwrap());
    assert_eq!(
        r.get_loc(10).unwrap(),
        oracle["get_loc_10"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.get_loc(6).unwrap(),
        oracle["get_loc_6"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.get_loc(2).unwrap(),
        oracle["get_loc_2"].as_u64().unwrap() as usize
    );
}

#[test]
fn conformance_range_index_empty_and_zero_differential() {
    let python_code = r#"
import pandas as pd, json
r_empty = pd.RangeIndex(0, 0)
r_zero = pd.RangeIndex(0, 5)
res = {
    "empty_len": len(r_empty),
    "empty_all": bool(r_empty.all()),
    "empty_any": bool(r_empty.any()),
    "empty_contains_0": bool(0 in r_empty),
    "zero_len": len(r_zero),
    "zero_all": bool(r_zero.all()),
    "zero_any": bool(r_zero.any()),
    "zero_contains_0": bool(0 in r_zero),
    "zero_min": int(r_zero.min()),
    "zero_max": int(r_zero.max()),
    "zero_argmax": int(r_zero.argmax()),
    "zero_argmin": int(r_zero.argmin())
}
print(json.dumps(res))
"#;
    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping RangeIndex differential test");
            return;
        }
    };

    let r_empty = RangeIndex::new(0, 0, 1).expect("empty range");
    assert_eq!(
        r_empty.len(),
        oracle["empty_len"].as_u64().unwrap() as usize
    );
    assert_eq!(r_empty.all(), oracle["empty_all"].as_bool().unwrap());
    assert_eq!(r_empty.any(), oracle["empty_any"].as_bool().unwrap());
    assert_eq!(
        r_empty.contains(0),
        oracle["empty_contains_0"].as_bool().unwrap()
    );
    assert_eq!(r_empty.min(), None);
    assert_eq!(r_empty.max(), None);
    assert!(r_empty.argmax().is_err());
    assert!(r_empty.argmin().is_err());

    let r_zero = RangeIndex::new(0, 5, 1).expect("zero range");
    assert_eq!(r_zero.len(), oracle["zero_len"].as_u64().unwrap() as usize);
    assert_eq!(r_zero.all(), oracle["zero_all"].as_bool().unwrap());
    assert_eq!(r_zero.any(), oracle["zero_any"].as_bool().unwrap());
    assert_eq!(
        r_zero.contains(0),
        oracle["zero_contains_0"].as_bool().unwrap()
    );
    assert_eq!(r_zero.min().unwrap(), oracle["zero_min"].as_i64().unwrap());
    assert_eq!(r_zero.max().unwrap(), oracle["zero_max"].as_i64().unwrap());
    assert_eq!(
        r_zero.argmax().unwrap(),
        oracle["zero_argmax"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r_zero.argmin().unwrap(),
        oracle["zero_argmin"].as_u64().unwrap() as usize
    );
}

#[test]
fn conformance_range_index_set_ops_differential() {
    let python_code = r#"
import pandas as pd, json
r1 = pd.RangeIndex(0, 10, 2)
r2 = pd.RangeIndex(4, 12, 2)
inter = r1.intersection(r2)
indexer = r1.get_indexer([0, 4, 8, 12])
res = {
    "intersection": inter.tolist(),
    "get_indexer": [int(x) for x in indexer]
}
print(json.dumps(res))
"#;
    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping RangeIndex differential test");
            return;
        }
    };

    let r1 = RangeIndex::new(0, 10, 2).expect("r1");
    let r2 = RangeIndex::new(4, 12, 2).expect("r2");
    let inter = r1.intersection(&r2);
    let inter_vals: Vec<i64> = inter
        .labels()
        .iter()
        .filter_map(|l| match l {
            IndexLabel::Int64(v) => Some(*v),
            _ => None,
        })
        .collect();
    let oracle_inter: Vec<i64> = oracle["intersection"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(inter_vals, oracle_inter);

    let indexer: Vec<isize> = r1.get_indexer(&[0, 4, 8, 12]);
    let oracle_indexer: Vec<isize> = oracle["get_indexer"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(indexer, oracle_indexer);
}

#[test]
fn live_oracle_base_index_metadata_dtypes_missing_and_duplicates_differential() {
    // @covers: Index.T
    // @covers: Index.all
    // @covers: Index.any
    // @covers: Index.array
    // @covers: Index.astype
    // @covers: Index.copy
    // @covers: Index.drop_duplicates
    // @covers: Index.dropna
    // @covers: Index.dtype
    // @covers: Index.duplicated
    // @covers: Index.empty
    // @covers: Index.equals
    // @covers: Index.factorize
    // @covers: Index.fillna
    // @covers: Index.format
    // @covers: Index.has_duplicates
    // @covers: Index.hasnans
    // @covers: Index.holds_integer
    // @covers: Index.identical
    // @covers: Index.infer_objects
    // @covers: Index.inferred_type
    // @covers: Index.is_
    // @covers: Index.is_boolean
    // @covers: Index.is_categorical
    // @covers: Index.is_floating
    // @covers: Index.is_integer
    // @covers: Index.is_interval
    // @covers: Index.is_monotonic_decreasing
    // @covers: Index.is_monotonic_increasing
    // @covers: Index.is_numeric
    // @covers: Index.is_object
    // @covers: Index.is_unique
    // @covers: Index.isin
    // @covers: Index.isna
    // @covers: Index.isnull
    // @covers: Index.item
    // @covers: Index.memory_usage
    // @covers: Index.name
    // @covers: Index.names
    // @covers: Index.nbytes
    // @covers: Index.ndim
    // @covers: Index.nlevels
    // @covers: Index.notna
    // @covers: Index.notnull
    // @covers: Index.nunique
    // @covers: Index.putmask
    // @covers: Index.ravel
    // @covers: Index.rename
    // @covers: Index.set_names
    // @covers: Index.shape
    // @covers: Index.size
    // @covers: Index.to_flat_index
    // @covers: Index.to_frame
    // @covers: Index.to_list
    // @covers: Index.to_numpy
    // @covers: Index.to_series
    // @covers: Index.tolist
    // @covers: Index.transpose
    // @covers: Index.unique
    // @covers: Index.value_counts
    // @covers: Index.values
    // @covers: Index.view
    // @covers: Index.where
    use fp_index::{DuplicateKeep, Index, OrderedF64};
    use fp_types::NullKind;

    let python_code = r#"
import json, numpy as np, pandas as pd

i_asc = pd.Index([10, 20, 30, 40], name="idx_int")
i_desc = pd.Index([40, 30, 20, 10], name="idx_desc")
i_zero = pd.Index([0, 10, 20])
i_nan = pd.Index([1.5, np.nan, 1.5, np.nan, -2.25], name="idx_nan")
i_mix = pd.Index([1, "alpha", 2.5, np.nan], dtype=object, name="idx_mix")
i_empty = pd.Index([], dtype="int64", name="empty")
i_single = pd.Index([42], name="one")

def opt_f64_list(seq):
    return [None if pd.isna(x) else float(x) for x in seq]

codes, uniques = i_nan.factorize()
vc = i_nan.value_counts(dropna=True)

res = {
    "asc_name": i_asc.name,
    "asc_names": list(i_asc.names),
    "asc_renamed": i_asc.rename("new_int").name,
    "asc_set_names": i_asc.set_names("set_int").name,
    "asc_dtype": str(i_asc.dtype),
    "asc_inferred_type": str(i_asc.inferred_type),
    "asc_holds_integer": True,
    "asc_is_integer": True,
    "asc_is_numeric": True,
    "asc_is_object": False,
    "asc_is_boolean": False,
    "asc_is_categorical": False,
    "asc_is_floating": False,
    "asc_is_interval": False,
    "asc_is_unique": bool(i_asc.is_unique),
    "asc_has_duplicates": bool(i_asc.has_duplicates),
    "asc_mono_inc": bool(i_asc.is_monotonic_increasing),
    "asc_mono_dec": bool(i_asc.is_monotonic_decreasing),
    "desc_mono_inc": bool(i_desc.is_monotonic_increasing),
    "desc_mono_dec": bool(i_desc.is_monotonic_decreasing),
    "asc_ndim": int(i_asc.ndim),
    "asc_nlevels": int(i_asc.nlevels),
    "asc_size": int(i_asc.size),
    "asc_shape": list(i_asc.shape),
    "asc_empty": bool(i_asc.empty),
    "empty_empty": bool(i_empty.empty),
    "single_item": int(i_single.item()),
    "asc_all": bool(i_asc.all()),
    "asc_any": bool(i_asc.any()),
    "zero_all": bool(i_zero.all()),
    "zero_any": bool(i_zero.any()),
    "empty_all": bool(i_empty.all()),
    "empty_any": bool(i_empty.any()),
    "asc_tolist": [int(x) for x in i_asc.tolist()],
    "asc_format": [str(x) for x in i_asc.astype(str).tolist()],
    "asc_astype_float": [float(x) for x in i_asc.astype("float64").tolist()],
    "asc_astype_str": [str(x) for x in i_asc.astype("str").tolist()],
    "asc_isin": [bool(x) for x in i_asc.isin([20, 99, 40])],
    "asc_where": [int(x) for x in i_asc.where([True, False, True, False], -1)],
    "asc_putmask": [int(x) for x in i_asc.putmask([False, True, False, True], -9)],
    "nan_dtype": str(i_nan.dtype),
    "nan_inferred_type": str(i_nan.inferred_type),
    "nan_hasnans": bool(i_nan.hasnans),
    "nan_isna": [bool(x) for x in i_nan.isna()],
    "nan_notna": [bool(x) for x in i_nan.notna()],
    "nan_dropna": [float(x) for x in i_nan.dropna()],
    "nan_fillna": [float(x) for x in i_nan.fillna(0.0)],
    "nan_unique": opt_f64_list(i_nan.unique()),
    "nan_nunique_dropna": int(i_nan.nunique(dropna=True)),
    "nan_nunique_keepna": int(i_nan.nunique(dropna=False)),
    "nan_dup_first": [bool(x) for x in i_nan.duplicated(keep="first")],
    "nan_dup_last": [bool(x) for x in i_nan.duplicated(keep="last")],
    "nan_dup_none": [bool(x) for x in i_nan.duplicated(keep=False)],
    "nan_drop_dup_first": opt_f64_list(i_nan.drop_duplicates(keep="first")),
    "nan_drop_dup_last": opt_f64_list(i_nan.drop_duplicates(keep="last")),
    "nan_drop_dup_none": opt_f64_list(i_nan.drop_duplicates(keep=False)),
    "nan_factorize_codes": [int(x) for x in codes],
    "nan_factorize_uniques": [float(x) for x in uniques],
    "nan_vc": [[float(k), int(v)] for k, v in vc.items()],
    "nan_equals_self": bool(i_nan.equals(i_nan.copy())),
    "nan_identical_self": bool(i_nan.identical(i_nan.copy())),
    "nan_identical_renamed": bool(i_nan.identical(i_nan.rename("other"))),
    "mix_dtype": str(i_mix.dtype),
    "mix_inferred_type": str(i_mix.inferred_type),
    "mix_is_object": bool(i_mix.dtype == object),
    "mix_hasnans": bool(i_mix.hasnans),
    "mix_isna": [bool(x) for x in i_mix.isna()],
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        eprintln!("skipping base Index metadata live-oracle test");
        return;
    };

    let i_asc = Index::from_i64_values(vec![10, 20, 30, 40]).set_name("idx_int");
    let i_desc = Index::from_i64_values(vec![40, 30, 20, 10]).set_name("idx_desc");
    let i_zero = Index::from_i64_values(vec![0, 10, 20]);
    let i_nan = Index::new(vec![
        IndexLabel::Float64(OrderedF64(1.5)),
        IndexLabel::Null(NullKind::NaN),
        IndexLabel::Float64(OrderedF64(1.5)),
        IndexLabel::Null(NullKind::NaN),
        IndexLabel::Float64(OrderedF64(-2.25)),
    ])
    .set_name("idx_nan");
    let i_mix = Index::new(vec![
        IndexLabel::Int64(1),
        IndexLabel::Utf8("alpha".to_owned()),
        IndexLabel::Float64(OrderedF64(2.5)),
        IndexLabel::Null(NullKind::NaN),
    ])
    .set_name("idx_mix");
    let i_empty = Index::from_i64_values(Vec::new()).set_name("empty");
    let i_single = Index::from_i64_values(vec![42]).set_name("one");

    assert_eq!(
        i_asc.name().map(|n| n.as_str()),
        oracle["asc_name"].as_str()
    );
    assert_eq!(i_asc.names(), vec![Some("idx_int".to_owned())]);
    assert_eq!(
        i_asc
            .rename_index(Some("new_int"))
            .name()
            .map(|n| n.as_str()),
        oracle["asc_renamed"].as_str()
    );
    assert_eq!(
        i_asc.set_names(Some("set_int")).name().map(|n| n.as_str()),
        oracle["asc_set_names"].as_str()
    );
    assert_eq!(
        i_asc
            .rename(|l| match l {
                IndexLabel::Int64(v) => IndexLabel::Int64(v + 1),
                other => other.clone(),
            })
            .labels(),
        &[
            IndexLabel::Int64(11),
            IndexLabel::Int64(21),
            IndexLabel::Int64(31),
            IndexLabel::Int64(41)
        ]
    );
    assert_eq!(i_asc.dtype(), oracle["asc_dtype"].as_str().unwrap());
    assert_eq!(
        i_asc.inferred_type(),
        oracle["asc_inferred_type"].as_str().unwrap()
    );
    assert_eq!(
        i_asc.holds_integer(),
        oracle["asc_holds_integer"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_integer(),
        oracle["asc_is_integer"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_numeric(),
        oracle["asc_is_numeric"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_object(),
        oracle["asc_is_object"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_boolean(),
        oracle["asc_is_boolean"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_categorical(),
        oracle["asc_is_categorical"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_floating(),
        oracle["asc_is_floating"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_interval(),
        oracle["asc_is_interval"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_unique(),
        oracle["asc_is_unique"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.has_duplicates(),
        oracle["asc_has_duplicates"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_monotonic_increasing(),
        oracle["asc_mono_inc"].as_bool().unwrap()
    );
    assert_eq!(
        i_asc.is_monotonic_decreasing(),
        oracle["asc_mono_dec"].as_bool().unwrap()
    );
    assert_eq!(
        i_desc.is_monotonic_increasing(),
        oracle["desc_mono_inc"].as_bool().unwrap()
    );
    assert_eq!(
        i_desc.is_monotonic_decreasing(),
        oracle["desc_mono_dec"].as_bool().unwrap()
    );
    assert_eq!(i_asc.ndim(), oracle["asc_ndim"].as_u64().unwrap() as usize);
    assert_eq!(
        i_asc.nlevels(),
        oracle["asc_nlevels"].as_u64().unwrap() as usize
    );
    assert_eq!(i_asc.size(), oracle["asc_size"].as_u64().unwrap() as usize);
    assert_eq!(
        i_asc.shape(),
        (oracle["asc_shape"][0].as_u64().unwrap() as usize,)
    );
    assert_eq!(i_asc.empty(), oracle["asc_empty"].as_bool().unwrap());
    assert_eq!(i_empty.empty(), oracle["empty_empty"].as_bool().unwrap());
    assert_eq!(
        i_single.item().unwrap(),
        IndexLabel::Int64(oracle["single_item"].as_i64().unwrap())
    );
    // Negative cases: item() on multi-element and empty indexes must fail
    assert!(i_asc.item().is_err());
    assert!(i_empty.item().is_err());

    assert_eq!(i_asc.all(), oracle["asc_all"].as_bool().unwrap());
    assert_eq!(i_asc.any(), oracle["asc_any"].as_bool().unwrap());
    assert_eq!(i_zero.all(), oracle["zero_all"].as_bool().unwrap());
    assert_eq!(i_zero.any(), oracle["zero_any"].as_bool().unwrap());
    assert_eq!(i_empty.all(), oracle["empty_all"].as_bool().unwrap());
    assert_eq!(i_empty.any(), oracle["empty_any"].as_bool().unwrap());
    assert!(i_asc.memory_usage(true) >= i_asc.nbytes());
    assert!(i_asc.nbytes() > 0);
    assert!(i_asc.is_(&i_asc));
    assert!(!i_asc.is_(&i_asc.copy()));
    assert_eq!(i_asc.copy(), i_asc);
    assert_eq!(i_asc.view(), i_asc);
    assert_eq!(i_asc.transpose(), i_asc);
    assert_eq!(i_asc.T(), i_asc);
    assert_eq!(i_asc.to_flat_index(), i_asc);
    assert_eq!(i_asc.infer_objects(), i_asc);

    let expected_asc: Vec<IndexLabel> = oracle["asc_tolist"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| IndexLabel::Int64(v.as_i64().unwrap()))
        .collect();
    assert_eq!(i_asc.to_list(), expected_asc);
    assert_eq!(i_asc.tolist(), expected_asc);
    assert_eq!(i_asc.to_numpy(), expected_asc);
    assert_eq!(i_asc.array(), expected_asc);
    assert_eq!(i_asc.values(), expected_asc);
    assert_eq!(i_asc.ravel(), expected_asc);
    assert_eq!(i_asc.to_frame().len(), 4);
    assert_eq!(i_asc.to_series().len(), 4);

    let expected_fmt: Vec<String> = oracle["asc_format"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(i_asc.format(), expected_fmt);
    assert_eq!(
        i_asc
            .astype("str")
            .unwrap()
            .labels()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        expected_fmt
    );
    assert_eq!(
        index_to_f64_vec(&i_asc.astype("float64").unwrap()),
        oracle["asc_astype_float"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect::<Vec<_>>()
    );
    // Negative case: astype("float64") and astype("int32") on non-numeric string fail (astype("int64") tracked in br-frankenpandas-qzknb)
    assert!(i_mix.astype("float64").is_err());
    assert!(i_mix.astype("int32").is_err());

    let expected_isin: Vec<bool> = oracle["asc_isin"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(
        i_asc.isin(&[
            IndexLabel::Int64(20),
            IndexLabel::Int64(99),
            IndexLabel::Int64(40),
        ]),
        expected_isin
    );
    let expected_where: Vec<IndexLabel> = oracle["asc_where"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| IndexLabel::Int64(v.as_i64().unwrap()))
        .collect();
    assert_eq!(
        i_asc
            .where_(&[true, false, true, false], &IndexLabel::Int64(-1))
            .to_list(),
        expected_where
    );
    let expected_putmask: Vec<IndexLabel> = oracle["asc_putmask"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| IndexLabel::Int64(v.as_i64().unwrap()))
        .collect();
    assert_eq!(
        i_asc
            .putmask(&[false, true, false, true], &IndexLabel::Int64(-9))
            .to_list(),
        expected_putmask
    );

    // NaN + duplicate float index assertions
    assert_eq!(i_nan.dtype(), oracle["nan_dtype"].as_str().unwrap());
    assert_eq!(
        i_nan.inferred_type(),
        oracle["nan_inferred_type"].as_str().unwrap()
    );
    assert_eq!(i_nan.hasnans(), oracle["nan_hasnans"].as_bool().unwrap());
    let expected_nan_isna: Vec<bool> = oracle["nan_isna"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    let expected_nan_notna: Vec<bool> = oracle["nan_notna"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(i_nan.isna(), expected_nan_isna);
    assert_eq!(i_nan.isnull(), expected_nan_isna);
    assert_eq!(i_nan.notna(), expected_nan_notna);
    assert_eq!(i_nan.notnull(), expected_nan_notna);
    assert_eq!(
        index_to_f64_vec(&i_nan.dropna()),
        oracle["nan_dropna"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        index_to_f64_vec(&i_nan.fillna(&IndexLabel::Float64(OrderedF64(0.0)))),
        oracle["nan_fillna"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect::<Vec<_>>()
    );

    let to_opt_f64 = |idx: &Index| -> Vec<Option<f64>> {
        idx.labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Float64(f) => Some(f.0),
                IndexLabel::Null(_) => None,
                other => panic!("unexpected label: {other:?}"),
            })
            .collect()
    };
    let parse_opt_f64 = |val: &serde_json::Value| -> Vec<Option<f64>> {
        val.as_array()
            .unwrap()
            .iter()
            .map(|v| {
                if v.is_null() {
                    None
                } else {
                    Some(v.as_f64().unwrap())
                }
            })
            .collect()
    };
    assert_eq!(
        to_opt_f64(&i_nan.unique()),
        parse_opt_f64(&oracle["nan_unique"])
    );
    assert_eq!(
        i_nan.nunique(),
        oracle["nan_nunique_dropna"].as_u64().unwrap() as usize
    );
    assert_eq!(
        i_nan.nunique_with_dropna(false),
        oracle["nan_nunique_keepna"].as_u64().unwrap() as usize
    );
    for (keep, key_dup, key_drop) in [
        (DuplicateKeep::First, "nan_dup_first", "nan_drop_dup_first"),
        (DuplicateKeep::Last, "nan_dup_last", "nan_drop_dup_last"),
        (DuplicateKeep::None, "nan_dup_none", "nan_drop_dup_none"),
    ] {
        let exp_dup: Vec<bool> = oracle[key_dup]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        assert_eq!(i_nan.duplicated(keep), exp_dup);
        assert_eq!(
            to_opt_f64(&i_nan.drop_duplicates_keep(keep)),
            parse_opt_f64(&oracle[key_drop])
        );
    }
    assert_eq!(
        to_opt_f64(&i_nan.drop_duplicates()),
        parse_opt_f64(&oracle["nan_drop_dup_first"])
    );

    let (codes, uniques) = i_nan.factorize();
    let exp_codes: Vec<isize> = oracle["nan_factorize_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(codes, exp_codes);
    assert_eq!(
        index_to_f64_vec(&uniques),
        oracle["nan_factorize_uniques"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect::<Vec<_>>()
    );
    let vc: Vec<(f64, usize)> = i_nan
        .value_counts()
        .into_iter()
        .map(|(l, c)| match l {
            IndexLabel::Float64(f) => (f.0, c),
            other => panic!("unexpected vc label: {other:?}"),
        })
        .collect();
    let exp_vc: Vec<(f64, usize)> = oracle["nan_vc"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| {
            let arr = pair.as_array().unwrap();
            (arr[0].as_f64().unwrap(), arr[1].as_u64().unwrap() as usize)
        })
        .collect();
    assert_eq!(vc, exp_vc);
    assert_eq!(
        i_nan.equals(&i_nan.copy()),
        oracle["nan_equals_self"].as_bool().unwrap()
    );
    assert_eq!(
        i_nan.identical(&i_nan.copy()),
        oracle["nan_identical_self"].as_bool().unwrap()
    );
    assert_eq!(
        i_nan.identical(&i_nan.rename_index(Some("other"))),
        oracle["nan_identical_renamed"].as_bool().unwrap()
    );

    // Mixed-type object index assertions
    assert_eq!(i_mix.dtype(), oracle["mix_dtype"].as_str().unwrap());
    assert_eq!(
        i_mix.inferred_type(),
        oracle["mix_inferred_type"].as_str().unwrap()
    );
    assert_eq!(
        i_mix.is_object(),
        oracle["mix_is_object"].as_bool().unwrap()
    );
    assert_eq!(i_mix.hasnans(), oracle["mix_hasnans"].as_bool().unwrap());
    let exp_mix_isna: Vec<bool> = oracle["mix_isna"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(i_mix.isna(), exp_mix_isna);
}

#[test]
fn live_oracle_base_index_indexing_set_ops_sorting_and_mutations_differential() {
    // @covers: Index.append
    // @covers: Index.argmax
    // @covers: Index.argmin
    // @covers: Index.argsort
    // @covers: Index.asof
    // @covers: Index.asof_locs
    // @covers: Index.delete
    // @covers: Index.diff
    // @covers: Index.difference
    // @covers: Index.drop
    // @covers: Index.droplevel
    // @covers: Index.get_indexer
    // @covers: Index.get_indexer_for
    // @covers: Index.get_indexer_non_unique
    // @covers: Index.get_level_values
    // @covers: Index.get_loc
    // @covers: Index.get_slice_bound
    // @covers: Index.groupby
    // @covers: Index.insert
    // @covers: Index.intersection
    // @covers: Index.join
    // @covers: Index.map
    // @covers: Index.max
    // @covers: Index.min
    // @covers: Index.reindex
    // @covers: Index.repeat
    // @covers: Index.round
    // @covers: Index.searchsorted
    // @covers: Index.shift
    // @covers: Index.slice_indexer
    // @covers: Index.slice_locs
    // @covers: Index.sort
    // @covers: Index.sort_values
    // @covers: Index.sortlevel
    // @covers: Index.str
    // @covers: Index.symmetric_difference
    // @covers: Index.take
    // @covers: Index.union
    use fp_index::{Index, OrderedF64};

    let python_code = r#"
import json, numpy as np, pandas as pd

i_asc = pd.Index([10, 20, 30, 40], name="k")
i_other = pd.Index([20, 40, 50], name="k")
i_unsorted = pd.Index([30, 10, 20, 10], name="u")
i_desc = pd.Index([40, 30, 20, 10], name="d")
i_flt = pd.Index([1.235, 2.5, -3.75], name="f")
i_str = pd.Index(["Alpha", "beta", "GAMMA"], name="s")

sl_idx, sl_ord = i_unsorted.sortlevel()
nu_indexer, nu_missing = i_unsorted.get_indexer_non_unique([10, 99, 30])
re_idx, re_pos = i_asc.reindex([20, 99, 10])
where_idx = pd.Index([5, 10, 25, 40, 55])
asof_locs = i_asc.asof_locs(where_idx, np.array([True, True, True, True], dtype=bool))
gb = pd.Series(range(len(i_unsorted)), index=i_unsorted).groupby(level=0).apply(list)

res = {
    "min": int(i_unsorted.min()),
    "max": int(i_unsorted.max()),
    "argmin": int(i_unsorted.argmin()),
    "argmax": int(i_unsorted.argmax()),
    "argsort": [int(x) for x in i_unsorted.argsort()],
    "sort_values": [int(x) for x in i_unsorted.sort_values()],
    "sortlevel_idx": [int(x) for x in sl_idx],
    "sortlevel_ord": [int(x) for x in sl_ord],
    "take": [int(x) for x in i_asc.take([3, 0, 2])],
    "append": [int(x) for x in i_asc.append(i_other)],
    "insert": [int(x) for x in i_asc.insert(2, 25)],
    "delete": [int(x) for x in i_asc.delete(1)],
    "drop": [int(x) for x in i_asc.drop([20, 40])],
    "repeat": [int(x) for x in i_asc.repeat(2)],
    "map": [int(x) for x in i_asc.map(lambda v: v * 2)],
    "diff_1": [None if pd.isna(x) else int(x) for x in i_asc.diff(1)],
    "round_1": [float(x) for x in i_flt.round(1)],
    "get_loc_30": int(i_asc.get_loc(30)),
    "get_indexer": [int(x) for x in i_asc.get_indexer([20, 99, 10])],
    "get_indexer_for_dup": [int(x) for x in i_unsorted.get_indexer_for([10, 99, 30])],
    "nu_indexer": [int(x) for x in nu_indexer],
    "nu_missing": [int(x) for x in nu_missing],
    "reindex_idx": [int(x) for x in re_idx],
    "reindex_pos": [int(x) for x in re_pos],
    "get_level_values_0": [int(x) for x in i_asc.get_level_values(0)],
    "searchsorted_left": int(i_asc.searchsorted(20, side="left")),
    "searchsorted_right": int(i_asc.searchsorted(20, side="right")),
    "asof_25": int(i_asc.asof(25)),
    "asof_5_isna": bool(pd.isna(i_asc.asof(5))),
    "asof_locs": [int(x) for x in asof_locs],
    "sb_asc_left": int(i_asc.get_slice_bound(20, "left")),
    "sb_asc_right": int(i_asc.get_slice_bound(20, "right")),
    "sb_desc_left": int(i_desc.get_slice_bound(25, "left")),
    "sb_desc_right": int(i_desc.get_slice_bound(25, "right")),
    "slice_locs": list(i_asc.slice_locs(20, 30)),
    "slice_indexer": [int(i_asc.slice_indexer(20, 30).start), int(i_asc.slice_indexer(20, 30).stop)],
    "union": [int(x) for x in i_asc.union(i_other)],
    "intersection": [int(x) for x in i_asc.intersection(i_other)],
    "difference": [int(x) for x in i_asc.difference(i_other)],
    "symdiff": [int(x) for x in i_asc.symmetric_difference(i_other)],
    "join_inner": [int(x) for x in i_asc.join(i_other, how="inner")],
    "join_outer": [int(x) for x in i_asc.join(i_other, how="outer")],
    "join_left": [int(x) for x in i_asc.join(i_other, how="left")],
    "join_right": [int(x) for x in i_asc.join(i_other, how="right")],
    "gb_10": [int(x) for x in gb[10]],
    "gb_20": [int(x) for x in gb[20]],
    "gb_30": [int(x) for x in gb[30]],
    "str_lower": list(i_str.str.lower()),
    "str_upper": list(i_str.str.upper()),
    "str_contains": [bool(x) for x in i_str.str.contains("a")],
    "str_len": [int(x) for x in i_str.str.len()],
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        eprintln!("skipping base Index indexing/set-ops live-oracle test");
        return;
    };

    let i_asc = Index::from_i64_values(vec![10, 20, 30, 40]).set_name("k");
    let i_other = Index::from_i64_values(vec![20, 40, 50]).set_name("k");
    let i_unsorted = Index::from_i64_values(vec![30, 10, 20, 10]).set_name("u");
    let i_desc = Index::from_i64_values(vec![40, 30, 20, 10]).set_name("d");
    let i_flt = Index::new(vec![
        IndexLabel::Float64(OrderedF64(1.235)),
        IndexLabel::Float64(OrderedF64(2.5)),
        IndexLabel::Float64(OrderedF64(-3.75)),
    ])
    .set_name("f");
    let i_str = Index::new(vec![
        IndexLabel::Utf8("Alpha".to_owned()),
        IndexLabel::Utf8("beta".to_owned()),
        IndexLabel::Utf8("GAMMA".to_owned()),
    ])
    .set_name("s");

    let to_i64_vec = |idx: &Index| -> Vec<i64> {
        idx.labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Int64(v) => *v,
                other => panic!("expected Int64 label, got {other:?}"),
            })
            .collect()
    };
    let parse_i64_vec = |val: &serde_json::Value| -> Vec<i64> {
        val.as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect()
    };
    let parse_usize_vec = |val: &serde_json::Value| -> Vec<usize> {
        val.as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect()
    };
    let parse_isize_vec = |val: &serde_json::Value| -> Vec<isize> {
        val.as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap() as isize)
            .collect()
    };

    assert_eq!(
        i_unsorted.min(),
        Some(IndexLabel::Int64(oracle["min"].as_i64().unwrap()))
    );
    assert_eq!(
        i_unsorted.max(),
        Some(IndexLabel::Int64(oracle["max"].as_i64().unwrap()))
    );
    assert_eq!(
        i_unsorted.argmin(),
        Some(oracle["argmin"].as_u64().unwrap() as usize)
    );
    assert_eq!(
        i_unsorted.argmax(),
        Some(oracle["argmax"].as_u64().unwrap() as usize)
    );
    assert_eq!(i_unsorted.argsort(), parse_usize_vec(&oracle["argsort"]));
    assert_eq!(
        to_i64_vec(&i_unsorted.sort_values()),
        parse_i64_vec(&oracle["sort_values"])
    );
    assert_eq!(
        to_i64_vec(&i_unsorted.sort()),
        parse_i64_vec(&oracle["sort_values"])
    );
    let (sl_idx, sl_ord) = i_unsorted.sortlevel();
    assert_eq!(to_i64_vec(&sl_idx), parse_i64_vec(&oracle["sortlevel_idx"]));
    assert_eq!(sl_ord, parse_usize_vec(&oracle["sortlevel_ord"]));

    assert_eq!(
        to_i64_vec(&i_asc.take(&[3, 0, 2])),
        parse_i64_vec(&oracle["take"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.append(&i_other)),
        parse_i64_vec(&oracle["append"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.insert(2, IndexLabel::Int64(25)).unwrap()),
        parse_i64_vec(&oracle["insert"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.delete(1).unwrap()),
        parse_i64_vec(&oracle["delete"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.drop(&[IndexLabel::Int64(20), IndexLabel::Int64(40)])),
        parse_i64_vec(&oracle["drop"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.repeat(2)),
        parse_i64_vec(&oracle["repeat"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.map(|l| match l {
            IndexLabel::Int64(v) => IndexLabel::Int64(v * 2),
            other => other.clone(),
        })),
        parse_i64_vec(&oracle["map"])
    );
    // Positional shift with fill (tracked against pandas NotImplementedError in br-frankenpandas-g7cmb)
    assert_eq!(
        to_i64_vec(&i_asc.shift(1, IndexLabel::Int64(-1))),
        vec![-1, 10, 20, 30]
    );
    let diff_1: Vec<Option<i64>> = i_asc
        .diff(1)
        .into_iter()
        .map(|opt| match opt {
            Some(IndexLabel::Int64(v)) => Some(v),
            None => None,
            other => panic!("unexpected diff label: {other:?}"),
        })
        .collect();
    let exp_diff_1: Vec<Option<i64>> = oracle["diff_1"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            if v.is_null() {
                None
            } else {
                Some(v.as_i64().unwrap())
            }
        })
        .collect();
    assert_eq!(diff_1, exp_diff_1);
    assert_eq!(
        index_to_f64_vec(&i_flt.round(1)),
        oracle["round_1"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect::<Vec<_>>()
    );

    assert_eq!(
        i_asc.get_loc(&IndexLabel::Int64(30)),
        Some(oracle["get_loc_30"].as_u64().unwrap() as usize)
    );
    assert_eq!(i_asc.get_loc(&IndexLabel::Int64(999)), None);

    let target = Index::from_i64_values(vec![20, 99, 10]);
    let indexer_isize: Vec<isize> = i_asc
        .get_indexer(&target)
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    assert_eq!(indexer_isize, parse_isize_vec(&oracle["get_indexer"]));

    let target_dup = Index::from_i64_values(vec![10, 99, 30]);
    let indexer_for_dup: Vec<isize> = i_unsorted
        .get_indexer_for(&target_dup)
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    assert_eq!(
        indexer_for_dup,
        parse_isize_vec(&oracle["get_indexer_for_dup"])
    );
    let (nu_indexer, nu_missing) = i_unsorted.get_indexer_non_unique(&target_dup);
    assert_eq!(nu_indexer, parse_isize_vec(&oracle["nu_indexer"]));
    assert_eq!(nu_missing, parse_usize_vec(&oracle["nu_missing"]));

    let (re_idx, re_pos) = i_asc.reindex(&target);
    assert_eq!(to_i64_vec(&re_idx), parse_i64_vec(&oracle["reindex_idx"]));
    let re_pos_isize: Vec<isize> = re_pos
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    assert_eq!(re_pos_isize, parse_isize_vec(&oracle["reindex_pos"]));

    assert_eq!(
        to_i64_vec(&i_asc.get_level_values(0).unwrap()),
        parse_i64_vec(&oracle["get_level_values_0"])
    );
    assert_eq!(
        i_asc.searchsorted(&IndexLabel::Int64(20), "left").unwrap(),
        oracle["searchsorted_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        i_asc.searchsorted(&IndexLabel::Int64(20), "right").unwrap(),
        oracle["searchsorted_right"].as_u64().unwrap() as usize
    );
    assert_eq!(
        i_asc.asof(&IndexLabel::Int64(25)),
        Some(IndexLabel::Int64(oracle["asof_25"].as_i64().unwrap()))
    );
    assert_eq!(
        i_asc.asof(&IndexLabel::Int64(5)).is_none(),
        oracle["asof_5_isna"].as_bool().unwrap()
    );
    let where_idx = Index::from_i64_values(vec![5, 10, 25, 40, 55]);
    let asof_locs: Vec<isize> = i_asc
        .asof_locs(&where_idx, Some(&[true, true, true, true]))
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    assert_eq!(asof_locs, parse_isize_vec(&oracle["asof_locs"]));

    assert_eq!(
        i_asc
            .get_slice_bound(&IndexLabel::Int64(20), "left")
            .unwrap(),
        oracle["sb_asc_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        i_asc
            .get_slice_bound(&IndexLabel::Int64(20), "right")
            .unwrap(),
        oracle["sb_asc_right"].as_u64().unwrap() as usize
    );
    assert_eq!(
        i_desc
            .get_slice_bound(&IndexLabel::Int64(25), "left")
            .unwrap(),
        oracle["sb_desc_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        i_desc
            .get_slice_bound(&IndexLabel::Int64(25), "right")
            .unwrap(),
        oracle["sb_desc_right"].as_u64().unwrap() as usize
    );
    assert_eq!(
        i_asc
            .slice_locs(Some(&IndexLabel::Int64(20)), Some(&IndexLabel::Int64(30)))
            .unwrap(),
        (
            oracle["slice_locs"][0].as_u64().unwrap() as usize,
            oracle["slice_locs"][1].as_u64().unwrap() as usize,
        )
    );
    assert_eq!(
        i_asc
            .slice_indexer(Some(&IndexLabel::Int64(20)), Some(&IndexLabel::Int64(30)))
            .unwrap(),
        (
            oracle["slice_indexer"][0].as_u64().unwrap() as usize,
            oracle["slice_indexer"][1].as_u64().unwrap() as usize,
        )
    );

    assert_eq!(
        to_i64_vec(&i_asc.union(&i_other)),
        parse_i64_vec(&oracle["union"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.intersection(&i_other)),
        parse_i64_vec(&oracle["intersection"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.difference(&i_other)),
        parse_i64_vec(&oracle["difference"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.symmetric_difference(&i_other)),
        parse_i64_vec(&oracle["symdiff"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.join(&i_other, "inner").unwrap()),
        parse_i64_vec(&oracle["join_inner"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.join(&i_other, "outer").unwrap()),
        parse_i64_vec(&oracle["join_outer"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.join(&i_other, "left").unwrap()),
        parse_i64_vec(&oracle["join_left"])
    );
    assert_eq!(
        to_i64_vec(&i_asc.join(&i_other, "right").unwrap()),
        parse_i64_vec(&oracle["join_right"])
    );

    let gb = i_unsorted.groupby();
    assert_eq!(
        gb[&IndexLabel::Int64(10)],
        parse_usize_vec(&oracle["gb_10"])
    );
    assert_eq!(
        gb[&IndexLabel::Int64(20)],
        parse_usize_vec(&oracle["gb_20"])
    );
    assert_eq!(
        gb[&IndexLabel::Int64(30)],
        parse_usize_vec(&oracle["gb_30"])
    );

    let s_acc = i_str.r#str();
    let exp_lower: Vec<Option<String>> = oracle["str_lower"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Some(v.as_str().unwrap().to_owned()))
        .collect();
    let exp_upper: Vec<Option<String>> = oracle["str_upper"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Some(v.as_str().unwrap().to_owned()))
        .collect();
    let exp_contains: Vec<Option<bool>> = oracle["str_contains"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Some(v.as_bool().unwrap()))
        .collect();
    let exp_len: Vec<Option<usize>> = oracle["str_len"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| Some(v.as_u64().unwrap() as usize))
        .collect();
    assert_eq!(s_acc.lower(), exp_lower);
    assert_eq!(s_acc.upper(), exp_upper);
    assert_eq!(s_acc.contains("a"), exp_contains);
    assert_eq!(s_acc.len(), exp_len);

    // Negative error paths
    assert!(i_asc.droplevel(0).is_err());
    assert!(i_asc.droplevel(1).is_err());
    assert!(i_asc.get_level_values(1).is_err());
    assert!(i_asc.insert(99, IndexLabel::Int64(1)).is_err());
    assert!(i_asc.delete(99).is_err());
    assert!(
        i_asc
            .searchsorted(&IndexLabel::Int64(20), "middle")
            .is_err()
    );
    assert!(i_asc.join(&i_other, "cross").is_err());
    // Unsorted index with non-contiguous duplicates (10 at 1 and 3) or missing label (99) fails get_slice_bound
    assert!(
        i_unsorted
            .get_slice_bound(&IndexLabel::Int64(10), "left")
            .is_err()
    );
    assert!(
        i_unsorted
            .get_slice_bound(&IndexLabel::Int64(99), "left")
            .is_err()
    );
}
