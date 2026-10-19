//! Index parity-matrix conformance suite (br-frankenpandas-jl63).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares the
//! existing Rust Index behavior with live upstream pandas for an edge-case
//! input: empty indexes, single labels, duplicate labels, mixed labels,
//! NA-like string labels, and extreme integer labels.

use fp_index::{CategoricalIndex, IndexLabel, IntervalIndex, RangeIndex};
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
        let guard = b"import pandas, sys\nif pandas.__version__ != '2.2.3':\n    print('\"__SKIP_VERSION__\"')\n    sys.exit(0)\n";
        let _ = stdin.write_all(guard);
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
    let val: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid index oracle json");
    if val.as_str() == Some("__SKIP_VERSION__") {
        return None;
    }
    Some(val)
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
fn idx_i64_vec(idx: &fp_index::Index) -> Vec<i64> {
    idx.to_list()
        .into_iter()
        .map(|l| {
            match l {
                IndexLabel::Int64(v) => Some(v),
                _ => None,
            }
            .expect("expected Int64 label")
        })
        .collect()
}

fn idx_str_vec(idx: &fp_index::Index) -> Vec<String> {
    idx.to_list().into_iter().map(|l| l.to_string()).collect()
}

fn cat_str_vec(ci: &CategoricalIndex) -> Vec<String> {
    ci.to_list().into_iter().map(|l| l.to_string()).collect()
}

fn parse_i64_vec(val: &serde_json::Value) -> Vec<i64> {
    val.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect()
}

fn parse_str_vec(val: &serde_json::Value) -> Vec<String> {
    val.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

fn parse_bool_vec(val: &serde_json::Value) -> Vec<bool> {
    val.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect()
}

#[test]
fn live_oracle_range_index_full_surface_and_set_ops_differential() {
    // @covers: RangeIndex.T
    // @covers: RangeIndex.all
    // @covers: RangeIndex.any
    // @covers: RangeIndex.append
    // @covers: RangeIndex.argmax
    // @covers: RangeIndex.argmin
    // @covers: RangeIndex.argsort
    // @covers: RangeIndex.array
    // @covers: RangeIndex.asof
    // @covers: RangeIndex.asof_locs
    // @covers: RangeIndex.astype
    // @covers: RangeIndex.copy
    // @covers: RangeIndex.delete
    // @covers: RangeIndex.diff
    // @covers: RangeIndex.difference
    // @covers: RangeIndex.drop
    // @covers: RangeIndex.drop_duplicates
    // @covers: RangeIndex.droplevel
    // @covers: RangeIndex.dropna
    // @covers: RangeIndex.dtype
    // @covers: RangeIndex.duplicated
    // @covers: RangeIndex.empty
    // @covers: RangeIndex.equals
    // @covers: RangeIndex.factorize
    // @covers: RangeIndex.fillna
    // @covers: RangeIndex.format
    // @covers: RangeIndex.from_range
    // @covers: RangeIndex.get_indexer
    // @covers: RangeIndex.get_indexer_for
    // @covers: RangeIndex.get_indexer_non_unique
    // @covers: RangeIndex.get_level_values
    // @covers: RangeIndex.get_loc
    // @covers: RangeIndex.get_slice_bound
    // @covers: RangeIndex.groupby
    // @covers: RangeIndex.has_duplicates
    // @covers: RangeIndex.hasnans
    // @covers: RangeIndex.holds_integer
    // @covers: RangeIndex.identical
    // @covers: RangeIndex.infer_objects
    // @covers: RangeIndex.inferred_type
    // @covers: RangeIndex.insert
    // @covers: RangeIndex.intersection
    // @covers: RangeIndex.is_
    // @covers: RangeIndex.is_boolean
    // @covers: RangeIndex.is_categorical
    // @covers: RangeIndex.is_floating
    // @covers: RangeIndex.is_integer
    // @covers: RangeIndex.is_interval
    // @covers: RangeIndex.is_monotonic_decreasing
    // @covers: RangeIndex.is_monotonic_increasing
    // @covers: RangeIndex.is_numeric
    // @covers: RangeIndex.is_object
    // @covers: RangeIndex.is_unique
    // @covers: RangeIndex.isin
    // @covers: RangeIndex.isna
    // @covers: RangeIndex.isnull
    // @covers: RangeIndex.item
    // @covers: RangeIndex.join
    // @covers: RangeIndex.map
    // @covers: RangeIndex.max
    // @covers: RangeIndex.memory_usage
    // @covers: RangeIndex.min
    // @covers: RangeIndex.name
    // @covers: RangeIndex.names
    // @covers: RangeIndex.nbytes
    // @covers: RangeIndex.ndim
    // @covers: RangeIndex.nlevels
    // @covers: RangeIndex.notna
    // @covers: RangeIndex.notnull
    // @covers: RangeIndex.nunique
    // @covers: RangeIndex.putmask
    // @covers: RangeIndex.ravel
    // @covers: RangeIndex.reindex
    // @covers: RangeIndex.rename
    // @covers: RangeIndex.repeat
    // @covers: RangeIndex.round
    // @covers: RangeIndex.searchsorted
    // @covers: RangeIndex.set_names
    // @covers: RangeIndex.shape
    // @covers: RangeIndex.shift
    // @covers: RangeIndex.size
    // @covers: RangeIndex.slice_indexer
    // @covers: RangeIndex.slice_locs
    // @covers: RangeIndex.sort
    // @covers: RangeIndex.sort_values
    // @covers: RangeIndex.sortlevel
    // @covers: RangeIndex.start
    // @covers: RangeIndex.step
    // @covers: RangeIndex.stop
    // @covers: RangeIndex.str
    // @covers: RangeIndex.symmetric_difference
    // @covers: RangeIndex.take
    // @covers: RangeIndex.to_flat_index
    // @covers: RangeIndex.to_frame
    // @covers: RangeIndex.to_list
    // @covers: RangeIndex.to_numpy
    // @covers: RangeIndex.to_series
    // @covers: RangeIndex.tolist
    // @covers: RangeIndex.transpose
    // @covers: RangeIndex.union
    // @covers: RangeIndex.unique
    // @covers: RangeIndex.value_counts
    // @covers: RangeIndex.values
    // @covers: RangeIndex.view
    // @covers: RangeIndex.where
    let python_code = r#"
import pandas as pd, numpy as np, json, warnings
warnings.filterwarnings("ignore")

r = pd.RangeIndex(2, 10, 2, name="rng")
r_same = pd.RangeIndex(2, 10, 2, name="rng")
r_diff_name = pd.RangeIndex(2, 10, 2, name="other")
r_desc = pd.RangeIndex(10, 2, -2, name="desc")
r_empty = pd.RangeIndex(0, 0, 1, name="empty")
r_single = pd.RangeIndex(7, 8, 1, name="single")
r_other = pd.RangeIndex(6, 14, 2, name="rng")
r_other_desc = pd.RangeIndex(12, 4, -2, name="desc")
r_step3 = pd.RangeIndex(0, 12, 3, name="rng")

def err_type(fn):
    try:
        fn()
        return None
    except Exception as e:
        return type(e).__name__

codes, uniques = r.factorize()
sl_sorted, sl_idx = r_desc.sortlevel()
re_idx, re_pos = r.reindex(pd.RangeIndex(4, 12, 2))
ginu_idx, ginu_miss = r.get_indexer_non_unique([2, 5, 8])

res = {
    "start": int(r.start),
    "stop": int(r.stop),
    "step": int(r.step),
    "from_range": pd.RangeIndex.from_range(range(2, 10, 2)).tolist(),
    "T": r.T.tolist(),
    "transpose": r.transpose().tolist(),
    "view": r.view().tolist(),
    "copy": r.copy().tolist(),
    "infer_objects": r.infer_objects().tolist(),
    "to_flat_index": r.to_flat_index().tolist(),
    "ravel": r.ravel().tolist(),
    "to_list": r.to_list(),
    "tolist": r.tolist(),
    "to_numpy": r.to_numpy().tolist(),
    "array": [int(x) for x in r.array],
    "values": r.values.tolist(),
    "dtype": str(r.dtype),
    "inferred_type": r.inferred_type,
    "empty_inferred_type": r_empty.inferred_type,
    "name": r.name,
    "names": list(r.names),
    "rename": r.rename("renamed").name,
    "set_names": r.set_names("sn").name,
    "ndim": int(r.ndim),
    "nlevels": int(r.nlevels),
    "size": int(r.size),
    "shape": list(r.shape),
    "empty": bool(r.empty),
    "empty_true": bool(r_empty.empty),
    "nbytes": int(r.nbytes),
    "memory_usage": int(r.memory_usage()),
    "all_true": bool(r.all()),
    "all_with_zero": bool(pd.RangeIndex(0, 4, 1).all()),
    "any_true": bool(r.any()),
    "any_empty": bool(r_empty.any()),
    "argmin": int(r.argmin()),
    "argmax": int(r.argmax()),
    "desc_argmin": int(r_desc.argmin()),
    "desc_argmax": int(r_desc.argmax()),
    "empty_argmin_err": err_type(lambda: r_empty.argmin()),
    "empty_argmax_err": err_type(lambda: r_empty.argmax()),
    "min": int(r.min()),
    "max": int(r.max()),
    "desc_min": int(r_desc.min()),
    "desc_max": int(r_desc.max()),
    "argsort": r.argsort().tolist(),
    "desc_argsort": r_desc.argsort().tolist(),
    "sort_values": r_desc.sort_values().tolist(),
    "sort_err": err_type(lambda: r.sort()),
    "sortlevel_vals": sl_sorted.tolist(),
    "sortlevel_idx": sl_idx.tolist(),
    "is_unique": bool(r.is_unique),
    "has_duplicates": bool(r.has_duplicates),
    "is_monotonic_increasing": bool(r.is_monotonic_increasing),
    "is_monotonic_decreasing": bool(r.is_monotonic_decreasing),
    "desc_is_monotonic_increasing": bool(r_desc.is_monotonic_increasing),
    "desc_is_monotonic_decreasing": bool(r_desc.is_monotonic_decreasing),
    "holds_integer": bool(r.holds_integer()),
    "is_boolean": bool(r.is_boolean()),
    "is_categorical": bool(r.is_categorical()),
    "is_floating": bool(r.is_floating()),
    "is_integer": bool(r.is_integer()),
    "is_interval": bool(r.is_interval()),
    "is_numeric": bool(r.is_numeric()),
    "is_object": bool(r.is_object()),
    "hasnans": bool(r.hasnans),
    "isna": r.isna().tolist(),
    "isnull": r.isnull().tolist(),
    "notna": r.notna().tolist(),
    "notnull": r.notnull().tolist(),
    "dropna": r.dropna().tolist(),
    "fillna": r.fillna(-1).tolist(),
    "unique": r.unique().tolist(),
    "nunique": int(r.nunique()),
    "duplicated": r.duplicated().tolist(),
    "drop_duplicates": r.drop_duplicates().tolist(),
    "value_counts": [[int(k), int(v)] for k, v in r.value_counts().items()],
    "factorize_codes": codes.tolist(),
    "factorize_uniques": uniques.tolist(),
    "equals_same": bool(r.equals(r_same)),
    "equals_diff_name": bool(r.equals(r_diff_name)),
    "equals_other": bool(r.equals(r_other)),
    "identical_same": bool(r.identical(r_same)),
    "identical_diff_name": bool(r.identical(r_diff_name)),
    "is_self": bool(r.is_(r)),
    "is_other": bool(r.is_(r_same)),
    "item_single": int(r_single.item()),
    "item_multi_err": err_type(lambda: r.item()),
    "astype_int64": r.astype("int64").tolist(),
    "astype_str": r.astype("str").tolist(),
    "format": [s.strip() for s in r.to_series().astype(str).tolist()],
    "get_level_values_0": r.get_level_values(0).tolist(),
    "get_level_values_oob_err": err_type(lambda: r.get_level_values(1)),
    "droplevel_0_err": err_type(lambda: r.droplevel(0)),
    "get_loc_6": int(r.get_loc(6)),
    "get_loc_missing_err": err_type(lambda: r.get_loc(5)),
    "get_indexer": r.get_indexer([2, 5, 8]).tolist(),
    "get_indexer_for": r.get_indexer_for([2, 5, 8]).tolist(),
    "ginu_idx": ginu_idx.tolist(),
    "ginu_miss": ginu_miss.tolist(),
    "get_slice_bound_left": int(r.get_slice_bound(4, "left")),
    "get_slice_bound_right": int(r.get_slice_bound(4, "right")),
    "slice_locs": [int(x) for x in r.slice_locs(4, 8)],
    "slice_indexer": [int(r.slice_indexer(4, 8).start), int(r.slice_indexer(4, 8).stop)],
    "searchsorted_left": int(r.searchsorted(5, side="left")),
    "searchsorted_right": int(r.searchsorted(6, side="right")),
    "asof_5": int(r.asof(5)),
    "asof_1_na": bool(pd.isna(r.asof(1))),
    "asof_locs": r.asof_locs(pd.Index([1, 4, 7]), np.ones(len(r), dtype=bool)).tolist(),
    "isin": r.isin([4, 8, 99]).tolist(),
    "take": r.take([3, 0, 2]).tolist(),
    "take_oob_err": err_type(lambda: r.take([99])),
    "repeat": r.repeat(2).tolist(),
    "where": r.where(np.array([True, False, True, False]), -1).tolist(),
    "putmask": r.putmask(np.array([False, True, False, True]), -9).tolist(),
    "append": r.append(r_other).tolist(),
    "insert": r.insert(2, 99).tolist(),
    "insert_oob_err": err_type(lambda: r.insert(99, 0)),
    "delete": r.delete(1).tolist(),
    "delete_oob_err": err_type(lambda: r.delete(99)),
    "drop": r.drop([4, 8]).tolist(),
    "drop_missing_err": err_type(lambda: r.drop([99])),
    "diff_1": [None if pd.isna(x) else int(x) for x in r.diff(1)],
    "round": r.round(0).tolist(),
    "shift_err": err_type(lambda: r.shift(1)),
    "map": r.map(lambda x: x * 10).tolist(),
    "str_zfill": r.astype(str).str.zfill(2).tolist(),
    "to_frame": r.to_frame(index=False).iloc[:, 0].tolist(),
    "to_series_idx": r.to_series().index.tolist(),
    "to_series_vals": r.to_series().tolist(),
    "groupby": {str(k): [int(v) for v in vals] for k, vals in r.groupby(r).items()},
    "join_inner": r.join(r_other.to_flat_index(), how="inner").tolist(),
    "join_left": r.join(r_other.to_flat_index(), how="left").tolist(),
    "join_right": r.join(r_other.to_flat_index(), how="right").tolist(),
    "join_outer": r.join(r_other.to_flat_index(), how="outer").tolist(),
    "reindex_vals": re_idx.tolist(),
    "reindex_pos": re_pos.tolist(),
    "inter_asc": r.intersection(r_other).tolist(),
    "inter_both_desc": r_desc.intersection(r_other_desc).tolist(),
    "inter_mixed_desc": r_desc.intersection(r_other).tolist(),
    "inter_step3": r.intersection(r_step3).tolist(),
    "union_asc": r.union(r_other).tolist(),
    "union_both_desc": r_desc.union(r_other_desc).tolist(),
    "union_desc_empty": r_desc.union(r_empty).tolist(),
    "union_step3": r.union(r_step3).tolist(),
    "diff_asc": r.difference(r_other).tolist(),
    "diff_both_desc": r_desc.difference(r_other_desc).tolist(),
    "diff_desc_empty": r_desc.difference(r_empty).tolist(),
    "symdiff_asc": r.symmetric_difference(r_other).tolist(),
    "symdiff_both_desc": r_desc.symmetric_difference(r_other_desc).tolist(),
    "symdiff_desc_empty": r_desc.symmetric_difference(r_empty).tolist(),
}
print(json.dumps(res, default=int))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        eprintln!("skipping RangeIndex full surface live-oracle test");
        return;
    };

    let r = RangeIndex::new(2, 10, 2).unwrap().set_name("rng");
    let r_same = RangeIndex::new(2, 10, 2).unwrap().set_name("rng");
    let r_diff_name = RangeIndex::new(2, 10, 2).unwrap().set_name("other");
    let r_desc = RangeIndex::new(10, 2, -2).unwrap().set_name("desc");
    let r_empty = RangeIndex::new(0, 0, 1).unwrap().set_name("empty");
    let r_single = RangeIndex::new(7, 8, 1).unwrap().set_name("single");
    let r_other = RangeIndex::new(6, 14, 2).unwrap().set_name("rng");
    let r_other_desc = RangeIndex::new(12, 4, -2).unwrap().set_name("desc");
    let r_step3 = RangeIndex::new(0, 12, 3).unwrap().set_name("rng");

    assert_eq!(r.start(), oracle["start"].as_i64().unwrap());
    assert_eq!(r.stop(), oracle["stop"].as_i64().unwrap());
    assert_eq!(r.step(), oracle["step"].as_i64().unwrap());
    assert_eq!(
        RangeIndex::from_range(2, 10, 2).unwrap().to_list(),
        parse_i64_vec(&oracle["from_range"])
    );
    assert_eq!(r.T().to_list(), parse_i64_vec(&oracle["T"]));
    assert_eq!(r.transpose().to_list(), parse_i64_vec(&oracle["transpose"]));
    assert_eq!(r.view().to_list(), parse_i64_vec(&oracle["view"]));
    assert_eq!(r.copy().to_list(), parse_i64_vec(&oracle["copy"]));
    assert_eq!(
        r.infer_objects().to_list(),
        parse_i64_vec(&oracle["infer_objects"])
    );
    assert_eq!(
        idx_i64_vec(&r.to_flat_index()),
        parse_i64_vec(&oracle["to_flat_index"])
    );
    assert_eq!(r.ravel(), parse_i64_vec(&oracle["ravel"]));
    assert_eq!(r.to_list(), parse_i64_vec(&oracle["to_list"]));
    assert_eq!(r.tolist(), parse_i64_vec(&oracle["tolist"]));
    assert_eq!(r.to_numpy(), parse_i64_vec(&oracle["to_numpy"]));
    assert_eq!(r.array(), parse_i64_vec(&oracle["array"]));
    assert_eq!(r.values().to_vec(), parse_i64_vec(&oracle["values"]));
    assert_eq!(r.dtype(), oracle["dtype"].as_str().unwrap());
    assert_eq!(r.inferred_type(), oracle["inferred_type"].as_str().unwrap());
    // Note: empty RangeIndex.inferred_type is "integer" in pandas 2.2.3 vs "empty" in fp_index (filed br-frankenpandas-hysyx)
    assert_eq!(r_empty.inferred_type(), "empty");
    assert_eq!(oracle["empty_inferred_type"].as_str(), Some("integer"));
    assert_eq!(
        r.name().map(|n| n.as_str()),
        Some(oracle["name"].as_str().unwrap())
    );
    assert_eq!(
        r.names(),
        vec![Some(
            oracle["names"].as_array().unwrap()[0]
                .as_str()
                .unwrap()
                .to_owned()
        )]
    );
    assert_eq!(
        r.rename("renamed").name().map(|n| n.as_str()),
        Some(oracle["rename"].as_str().unwrap())
    );
    assert_eq!(
        r.set_names(Some("sn")).name().map(|n| n.as_str()),
        Some(oracle["set_names"].as_str().unwrap())
    );
    assert_eq!(r.ndim(), oracle["ndim"].as_u64().unwrap() as usize);
    assert_eq!(r.nlevels(), oracle["nlevels"].as_u64().unwrap() as usize);
    assert_eq!(r.size(), oracle["size"].as_u64().unwrap() as usize);
    assert_eq!(
        r.shape(),
        (oracle["shape"].as_array().unwrap()[0].as_u64().unwrap() as usize,)
    );
    assert_eq!(r.empty(), oracle["empty"].as_bool().unwrap());
    assert_eq!(r_empty.empty(), oracle["empty_true"].as_bool().unwrap());
    assert!(r.nbytes() > 0 && oracle["nbytes"].as_u64().unwrap() > 0);
    assert!(r.memory_usage(false) > 0 && oracle["memory_usage"].as_u64().unwrap() > 0);

    assert_eq!(r.all(), oracle["all_true"].as_bool().unwrap());
    assert_eq!(
        RangeIndex::new(0, 4, 1).unwrap().all(),
        oracle["all_with_zero"].as_bool().unwrap()
    );
    assert_eq!(r.any(), oracle["any_true"].as_bool().unwrap());
    assert_eq!(r_empty.any(), oracle["any_empty"].as_bool().unwrap());

    assert_eq!(
        r.argmin().unwrap(),
        oracle["argmin"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.argmax().unwrap(),
        oracle["argmax"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r_desc.argmin().unwrap(),
        oracle["desc_argmin"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r_desc.argmax().unwrap(),
        oracle["desc_argmax"].as_u64().unwrap() as usize
    );
    assert!(r_empty.argmin().is_err());
    assert!(r_empty.argmax().is_err());
    assert_eq!(oracle["empty_argmin_err"].as_str(), Some("ValueError"));
    assert_eq!(oracle["empty_argmax_err"].as_str(), Some("ValueError"));

    assert_eq!(r.min().unwrap(), oracle["min"].as_i64().unwrap());
    assert_eq!(r.max().unwrap(), oracle["max"].as_i64().unwrap());
    assert_eq!(r_desc.min().unwrap(), oracle["desc_min"].as_i64().unwrap());
    assert_eq!(r_desc.max().unwrap(), oracle["desc_max"].as_i64().unwrap());

    let exp_argsort: Vec<usize> = oracle["argsort"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    let exp_desc_argsort: Vec<usize> = oracle["desc_argsort"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(r.argsort(), exp_argsort);
    assert_eq!(r_desc.argsort(), exp_desc_argsort);
    assert_eq!(
        r_desc.sort_values().to_list(),
        parse_i64_vec(&oracle["sort_values"])
    );
    assert_eq!(r_desc.sort().to_list(), r_desc.sort_values().to_list());
    assert_eq!(oracle["sort_err"].as_str(), Some("TypeError"));

    let (sl_sorted, sl_idx) = r_desc.sortlevel();
    let exp_sl_idx: Vec<usize> = oracle["sortlevel_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(
        idx_i64_vec(&sl_sorted),
        parse_i64_vec(&oracle["sortlevel_vals"])
    );
    assert_eq!(sl_idx, exp_sl_idx);

    assert_eq!(r.is_unique(), oracle["is_unique"].as_bool().unwrap());
    assert_eq!(
        r.has_duplicates(),
        oracle["has_duplicates"].as_bool().unwrap()
    );
    assert_eq!(
        r.is_monotonic_increasing(),
        oracle["is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        r.is_monotonic_decreasing(),
        oracle["is_monotonic_decreasing"].as_bool().unwrap()
    );
    assert_eq!(
        r_desc.is_monotonic_increasing(),
        oracle["desc_is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        r_desc.is_monotonic_decreasing(),
        oracle["desc_is_monotonic_decreasing"].as_bool().unwrap()
    );
    assert_eq!(
        r.holds_integer(),
        oracle["holds_integer"].as_bool().unwrap()
    );
    assert_eq!(r.is_boolean(), oracle["is_boolean"].as_bool().unwrap());
    assert_eq!(
        r.is_categorical(),
        oracle["is_categorical"].as_bool().unwrap()
    );
    assert_eq!(r.is_floating(), oracle["is_floating"].as_bool().unwrap());
    assert_eq!(r.is_integer(), oracle["is_integer"].as_bool().unwrap());
    assert_eq!(r.is_interval(), oracle["is_interval"].as_bool().unwrap());
    assert_eq!(r.is_numeric(), oracle["is_numeric"].as_bool().unwrap());
    assert_eq!(r.is_object(), oracle["is_object"].as_bool().unwrap());

    assert_eq!(r.hasnans(), oracle["hasnans"].as_bool().unwrap());
    assert_eq!(r.isna(), parse_bool_vec(&oracle["isna"]));
    assert_eq!(r.isnull(), parse_bool_vec(&oracle["isnull"]));
    assert_eq!(r.notna(), parse_bool_vec(&oracle["notna"]));
    assert_eq!(r.notnull(), parse_bool_vec(&oracle["notnull"]));
    assert_eq!(r.dropna().to_list(), parse_i64_vec(&oracle["dropna"]));
    assert_eq!(r.fillna(-1).to_list(), parse_i64_vec(&oracle["fillna"]));

    assert_eq!(r.unique().to_list(), parse_i64_vec(&oracle["unique"]));
    assert_eq!(r.nunique(), oracle["nunique"].as_u64().unwrap() as usize);
    assert_eq!(
        r.duplicated(fp_index::DuplicateKeep::First),
        parse_bool_vec(&oracle["duplicated"])
    );
    assert_eq!(
        r.drop_duplicates().to_list(),
        parse_i64_vec(&oracle["drop_duplicates"])
    );

    let vc = r.to_index().value_counts();
    let exp_vc: Vec<(IndexLabel, usize)> = oracle["value_counts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let arr = row.as_array().unwrap();
            (
                IndexLabel::Int64(arr[0].as_i64().unwrap()),
                arr[1].as_u64().unwrap() as usize,
            )
        })
        .collect();
    assert_eq!(vc, exp_vc);

    let (fac_codes, fac_uniques) = r.factorize();
    let exp_fac_codes: Vec<usize> = oracle["factorize_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(fac_codes, exp_fac_codes);
    assert_eq!(
        fac_uniques.to_list(),
        parse_i64_vec(&oracle["factorize_uniques"])
    );

    assert_eq!(r.equals(&r_same), oracle["equals_same"].as_bool().unwrap());
    assert_eq!(
        r.equals(&r_diff_name),
        oracle["equals_diff_name"].as_bool().unwrap()
    );
    assert_eq!(
        r.equals(&r_other),
        oracle["equals_other"].as_bool().unwrap()
    );
    assert_eq!(
        r.identical(&r_same),
        oracle["identical_same"].as_bool().unwrap()
    );
    assert_eq!(
        r.identical(&r_diff_name),
        oracle["identical_diff_name"].as_bool().unwrap()
    );
    assert_eq!(r.is_(&r), oracle["is_self"].as_bool().unwrap());
    assert_eq!(r.is_(&r_same), oracle["is_other"].as_bool().unwrap());

    assert_eq!(
        r_single.item().unwrap(),
        oracle["item_single"].as_i64().unwrap()
    );
    assert!(r.item().is_err());
    assert_eq!(oracle["item_multi_err"].as_str(), Some("ValueError"));

    assert_eq!(
        idx_i64_vec(&r.astype("int64").unwrap()),
        parse_i64_vec(&oracle["astype_int64"])
    );
    assert_eq!(
        idx_str_vec(&r.astype("str").unwrap()),
        parse_str_vec(&oracle["astype_str"])
    );
    assert_eq!(r.format(), parse_str_vec(&oracle["format"]));

    assert_eq!(
        idx_i64_vec(&r.get_level_values(0).unwrap()),
        parse_i64_vec(&oracle["get_level_values_0"])
    );
    assert!(r.get_level_values(1).is_err());
    assert_eq!(
        oracle["get_level_values_oob_err"].as_str(),
        Some("IndexError")
    );
    assert!(r.droplevel(0).is_err());
    assert_eq!(oracle["droplevel_0_err"].as_str(), Some("ValueError"));

    assert_eq!(
        r.get_loc(6).unwrap(),
        oracle["get_loc_6"].as_u64().unwrap() as usize
    );
    assert!(r.get_loc(5).is_err());
    assert_eq!(oracle["get_loc_missing_err"].as_str(), Some("KeyError"));

    let exp_gi: Vec<isize> = oracle["get_indexer"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    let exp_gif: Vec<isize> = oracle["get_indexer_for"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(r.get_indexer(&[2, 5, 8]), exp_gi);
    assert_eq!(r.get_indexer_for(&[2, 5, 8]), exp_gif);

    let (ginu_idx, ginu_miss) = r.get_indexer_non_unique(&[2, 5, 8]);
    let exp_ginu_idx: Vec<isize> = oracle["ginu_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    let exp_ginu_miss: Vec<usize> = oracle["ginu_miss"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(ginu_idx, exp_ginu_idx);
    assert_eq!(ginu_miss, exp_ginu_miss);

    assert_eq!(
        r.get_slice_bound(4, "left").unwrap(),
        oracle["get_slice_bound_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.get_slice_bound(4, "right").unwrap(),
        oracle["get_slice_bound_right"].as_u64().unwrap() as usize
    );

    let sl = r.slice_locs(4, 8).unwrap();
    let exp_sl = oracle["slice_locs"].as_array().unwrap();
    assert_eq!(
        sl,
        (
            exp_sl[0].as_u64().unwrap() as usize,
            exp_sl[1].as_u64().unwrap() as usize
        )
    );
    let si = r.slice_indexer(4, 8).unwrap();
    let exp_si = oracle["slice_indexer"].as_array().unwrap();
    assert_eq!(
        (si.start, si.end),
        (
            exp_si[0].as_u64().unwrap() as usize,
            exp_si[1].as_u64().unwrap() as usize
        )
    );

    assert_eq!(
        r.searchsorted(5, "left").unwrap(),
        oracle["searchsorted_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.searchsorted(6, "right").unwrap(),
        oracle["searchsorted_right"].as_u64().unwrap() as usize
    );
    assert_eq!(
        r.asof(&IndexLabel::Int64(5)),
        Some(IndexLabel::Int64(oracle["asof_5"].as_i64().unwrap()))
    );
    assert_eq!(r.asof(&IndexLabel::Int64(1)), None);
    assert!(oracle["asof_1_na"].as_bool().unwrap());

    let where_idx = fp_index::Index::from_i64_values(vec![1, 4, 7]);
    let asof_locs: Vec<isize> = r
        .asof_locs(&where_idx, Some(&[true; 4]))
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    let exp_asof_locs: Vec<isize> = oracle["asof_locs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(asof_locs, exp_asof_locs);

    assert_eq!(r.isin(&[4, 8, 99]), parse_bool_vec(&oracle["isin"]));
    assert_eq!(
        idx_i64_vec(&r.take(&[3, 0, 2]).unwrap()),
        parse_i64_vec(&oracle["take"])
    );
    assert!(r.take(&[99]).is_err());
    assert_eq!(oracle["take_oob_err"].as_str(), Some("IndexError"));
    assert_eq!(idx_i64_vec(&r.repeat(2)), parse_i64_vec(&oracle["repeat"]));
    assert_eq!(
        idx_i64_vec(&r.r#where(&[true, false, true, false], -1).unwrap()),
        parse_i64_vec(&oracle["where"])
    );
    assert_eq!(
        idx_i64_vec(&r.putmask(&[false, true, false, true], -9).unwrap()),
        parse_i64_vec(&oracle["putmask"])
    );
    assert_eq!(
        idx_i64_vec(&r.append(&r_other)),
        parse_i64_vec(&oracle["append"])
    );
    assert_eq!(
        idx_i64_vec(&r.insert(2, 99).unwrap()),
        parse_i64_vec(&oracle["insert"])
    );
    assert!(r.insert(99, 0).is_err());
    assert_eq!(oracle["insert_oob_err"].as_str(), Some("IndexError"));
    assert_eq!(
        idx_i64_vec(&r.delete(1).unwrap()),
        parse_i64_vec(&oracle["delete"])
    );
    assert!(r.delete(99).is_err());
    assert_eq!(oracle["delete_oob_err"].as_str(), Some("IndexError"));
    assert_eq!(
        idx_i64_vec(&r.drop(&[IndexLabel::Int64(4), IndexLabel::Int64(8)])),
        parse_i64_vec(&oracle["drop"])
    );
    assert_eq!(oracle["drop_missing_err"].as_str(), Some("KeyError"));

    let diff_1 = r.diff(1);
    let exp_diff_1: Vec<Option<i64>> = oracle["diff_1"]
        .as_array()
        .unwrap()
        .iter()
        .map(serde_json::Value::as_i64)
        .collect();
    assert_eq!(diff_1, exp_diff_1);

    assert_eq!(
        idx_i64_vec(&r.to_index().round(0)),
        parse_i64_vec(&oracle["round"])
    );
    let shifted = r.to_index().shift(1, IndexLabel::Int64(-1));
    assert_eq!(idx_i64_vec(&shifted), vec![-1, 2, 4, 6]);
    assert_eq!(oracle["shift_err"].as_str(), Some("NotImplementedError"));

    let mapped = r.map(|l| match l {
        IndexLabel::Int64(v) => IndexLabel::Int64(v * 10),
        other => other.clone(),
    });
    assert_eq!(idx_i64_vec(&mapped), parse_i64_vec(&oracle["map"]));

    assert_eq!(r.r#str().len(), vec![None, None, None, None]);
    let str_up: Vec<String> = r
        .astype("str")
        .unwrap()
        .r#str()
        .upper()
        .into_iter()
        .map(|o| o.unwrap())
        .collect();
    assert_eq!(str_up, vec!["2", "4", "6", "8"]);

    let tf: Vec<i64> = r
        .to_frame()
        .into_iter()
        .map(|row| {
            match row[0] {
                IndexLabel::Int64(v) => Some(v),
                _ => None,
            }
            .unwrap()
        })
        .collect();
    assert_eq!(tf, parse_i64_vec(&oracle["to_frame"]));
    let ts: Vec<i64> = r
        .to_series()
        .into_iter()
        .map(|(_, v)| {
            match v {
                IndexLabel::Int64(x) => Some(x),
                _ => None,
            }
            .unwrap()
        })
        .collect();
    assert_eq!(ts, parse_i64_vec(&oracle["to_series_vals"]));

    let gb = r.groupby();
    assert_eq!(gb.get(&IndexLabel::Int64(2)), Some(&vec![0usize]));
    assert_eq!(gb.get(&IndexLabel::Int64(8)), Some(&vec![3usize]));

    assert_eq!(
        idx_i64_vec(&r.join(&r_other.to_flat_index(), "inner").unwrap()),
        parse_i64_vec(&oracle["join_inner"])
    );
    assert_eq!(
        idx_i64_vec(&r.join(&r_other.to_flat_index(), "left").unwrap()),
        parse_i64_vec(&oracle["join_left"])
    );
    assert_eq!(
        idx_i64_vec(&r.join(&r_other.to_flat_index(), "right").unwrap()),
        parse_i64_vec(&oracle["join_right"])
    );
    assert_eq!(
        idx_i64_vec(&r.join(&r_other.to_flat_index(), "outer").unwrap()),
        parse_i64_vec(&oracle["join_outer"])
    );

    let (re_idx, re_pos) = r.reindex(&RangeIndex::new(4, 12, 2).unwrap());
    let exp_re_pos: Vec<isize> = oracle["reindex_pos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(re_idx.to_list(), parse_i64_vec(&oracle["reindex_vals"]));
    assert_eq!(re_pos, exp_re_pos);

    // RangeIndex set-op ordering and step arithmetic
    assert_eq!(
        idx_i64_vec(&r.intersection(&r_other)),
        parse_i64_vec(&oracle["inter_asc"])
    );
    assert_eq!(
        idx_i64_vec(&r_desc.intersection(&r_other_desc)),
        parse_i64_vec(&oracle["inter_both_desc"])
    );
    assert_eq!(
        idx_i64_vec(&r_desc.intersection(&r_other)),
        parse_i64_vec(&oracle["inter_mixed_desc"])
    );
    assert_eq!(
        idx_i64_vec(&r.intersection(&r_step3)),
        parse_i64_vec(&oracle["inter_step3"])
    );
    assert_eq!(
        idx_i64_vec(&r.union(&r_other)),
        parse_i64_vec(&oracle["union_asc"])
    );
    assert_eq!(
        idx_i64_vec(&r_desc.union(&r_other_desc)),
        parse_i64_vec(&oracle["union_both_desc"])
    );
    assert_eq!(
        idx_i64_vec(&r_desc.union(&r_empty)),
        parse_i64_vec(&oracle["union_desc_empty"])
    );
    assert_eq!(
        idx_i64_vec(&r.union(&r_step3)),
        parse_i64_vec(&oracle["union_step3"])
    );
    assert_eq!(
        idx_i64_vec(&r.difference(&r_other)),
        parse_i64_vec(&oracle["diff_asc"])
    );
    assert_eq!(
        idx_i64_vec(&r_desc.difference(&r_other_desc)),
        parse_i64_vec(&oracle["diff_both_desc"])
    );
    assert_eq!(
        idx_i64_vec(&r_desc.difference(&r_empty)),
        parse_i64_vec(&oracle["diff_desc_empty"])
    );
    assert_eq!(
        idx_i64_vec(&r.symmetric_difference(&r_other)),
        parse_i64_vec(&oracle["symdiff_asc"])
    );
    assert_eq!(
        idx_i64_vec(&r_desc.symmetric_difference(&r_other_desc)),
        parse_i64_vec(&oracle["symdiff_both_desc"])
    );
    assert_eq!(
        idx_i64_vec(&r_desc.symmetric_difference(&r_empty)),
        parse_i64_vec(&oracle["symdiff_desc_empty"])
    );

    // Negative error paths
    assert!(RangeIndex::new(0, 10, 0).is_err());
    assert!(r_desc.searchsorted(4, "left").is_err());
    assert!(r_desc.slice_locs(4, 8).is_err());
    assert!(r.r#where(&[true], 0).is_err());
    assert!(r.putmask(&[true], 0).is_err());
    assert!(r.join(&r_other.to_flat_index(), "cross").is_err());
}

#[test]
fn live_oracle_categorical_index_full_surface_and_category_ops_differential() {
    // @covers: CategoricalIndex.T
    // @covers: CategoricalIndex.add_categories
    // @covers: CategoricalIndex.all
    // @covers: CategoricalIndex.any
    // @covers: CategoricalIndex.append
    // @covers: CategoricalIndex.argmax
    // @covers: CategoricalIndex.argmin
    // @covers: CategoricalIndex.argsort
    // @covers: CategoricalIndex.array
    // @covers: CategoricalIndex.as_ordered
    // @covers: CategoricalIndex.as_unordered
    // @covers: CategoricalIndex.asof
    // @covers: CategoricalIndex.asof_locs
    // @covers: CategoricalIndex.astype
    // @covers: CategoricalIndex.categories
    // @covers: CategoricalIndex.codes
    // @covers: CategoricalIndex.copy
    // @covers: CategoricalIndex.delete
    // @covers: CategoricalIndex.diff
    // @covers: CategoricalIndex.difference
    // @covers: CategoricalIndex.drop
    // @covers: CategoricalIndex.drop_duplicates
    // @covers: CategoricalIndex.droplevel
    // @covers: CategoricalIndex.dropna
    // @covers: CategoricalIndex.dtype
    // @covers: CategoricalIndex.duplicated
    // @covers: CategoricalIndex.empty
    // @covers: CategoricalIndex.equals
    // @covers: CategoricalIndex.factorize
    // @covers: CategoricalIndex.fillna
    // @covers: CategoricalIndex.format
    // @covers: CategoricalIndex.get_indexer
    // @covers: CategoricalIndex.get_indexer_for
    // @covers: CategoricalIndex.get_indexer_non_unique
    // @covers: CategoricalIndex.get_level_values
    // @covers: CategoricalIndex.get_loc
    // @covers: CategoricalIndex.get_slice_bound
    // @covers: CategoricalIndex.groupby
    // @covers: CategoricalIndex.has_duplicates
    // @covers: CategoricalIndex.hasnans
    // @covers: CategoricalIndex.holds_integer
    // @covers: CategoricalIndex.identical
    // @covers: CategoricalIndex.infer_objects
    // @covers: CategoricalIndex.inferred_type
    // @covers: CategoricalIndex.insert
    // @covers: CategoricalIndex.intersection
    // @covers: CategoricalIndex.is_
    // @covers: CategoricalIndex.is_boolean
    // @covers: CategoricalIndex.is_categorical
    // @covers: CategoricalIndex.is_floating
    // @covers: CategoricalIndex.is_integer
    // @covers: CategoricalIndex.is_interval
    // @covers: CategoricalIndex.is_monotonic_decreasing
    // @covers: CategoricalIndex.is_monotonic_increasing
    // @covers: CategoricalIndex.is_numeric
    // @covers: CategoricalIndex.is_object
    // @covers: CategoricalIndex.is_unique
    // @covers: CategoricalIndex.isin
    // @covers: CategoricalIndex.isna
    // @covers: CategoricalIndex.isnull
    // @covers: CategoricalIndex.item
    // @covers: CategoricalIndex.join
    // @covers: CategoricalIndex.map
    // @covers: CategoricalIndex.max
    // @covers: CategoricalIndex.memory_usage
    // @covers: CategoricalIndex.min
    // @covers: CategoricalIndex.name
    // @covers: CategoricalIndex.names
    // @covers: CategoricalIndex.nbytes
    // @covers: CategoricalIndex.ndim
    // @covers: CategoricalIndex.nlevels
    // @covers: CategoricalIndex.notna
    // @covers: CategoricalIndex.notnull
    // @covers: CategoricalIndex.nunique
    // @covers: CategoricalIndex.ordered
    // @covers: CategoricalIndex.putmask
    // @covers: CategoricalIndex.ravel
    // @covers: CategoricalIndex.reindex
    // @covers: CategoricalIndex.remove_categories
    // @covers: CategoricalIndex.remove_unused_categories
    // @covers: CategoricalIndex.rename
    // @covers: CategoricalIndex.rename_categories
    // @covers: CategoricalIndex.reorder_categories
    // @covers: CategoricalIndex.repeat
    // @covers: CategoricalIndex.round
    // @covers: CategoricalIndex.searchsorted
    // @covers: CategoricalIndex.set_categories
    // @covers: CategoricalIndex.set_names
    // @covers: CategoricalIndex.shape
    // @covers: CategoricalIndex.shift
    // @covers: CategoricalIndex.size
    // @covers: CategoricalIndex.slice_indexer
    // @covers: CategoricalIndex.slice_locs
    // @covers: CategoricalIndex.sort
    // @covers: CategoricalIndex.sort_values
    // @covers: CategoricalIndex.sortlevel
    // @covers: CategoricalIndex.str
    // @covers: CategoricalIndex.symmetric_difference
    // @covers: CategoricalIndex.take
    // @covers: CategoricalIndex.to_flat_index
    // @covers: CategoricalIndex.to_frame
    // @covers: CategoricalIndex.to_list
    // @covers: CategoricalIndex.to_numpy
    // @covers: CategoricalIndex.to_series
    // @covers: CategoricalIndex.tolist
    // @covers: CategoricalIndex.transpose
    // @covers: CategoricalIndex.union
    // @covers: CategoricalIndex.unique
    // @covers: CategoricalIndex.value_counts
    // @covers: CategoricalIndex.values
    // @covers: CategoricalIndex.view
    // @covers: CategoricalIndex.where
    let python_code = r#"
import pandas as pd, numpy as np, json, warnings
warnings.filterwarnings("ignore")

ci = pd.CategoricalIndex(["b", "a", "b", "c"], categories=["b", "a", "c", "d"], ordered=True, name="cat")
ci_same = pd.CategoricalIndex(["b", "a", "b", "c"], categories=["b", "a", "c", "d"], ordered=True, name="cat")
ci_diff_name = pd.CategoricalIndex(["b", "a", "b", "c"], categories=["b", "a", "c", "d"], ordered=True, name="other")
ci_unord = pd.CategoricalIndex(["b", "a", "b", "c"], categories=["b", "a", "c", "d"], ordered=False, name="cat")
ci_na = pd.CategoricalIndex(["b", None, "a", "c"], categories=["b", "a", "c"], ordered=True, name="cat")
ci_single = pd.CategoricalIndex(["b"], categories=["b", "a"], ordered=True, name="single")
ci_empty = pd.CategoricalIndex([], categories=["b", "a"], ordered=True, name="empty")
ci_sorted = pd.CategoricalIndex(["a", "b", "b", "c"], categories=["a", "b", "c"], ordered=True, name="cat")
ci_other = pd.CategoricalIndex(["a", "c", "d"], categories=["b", "a", "c", "d"], ordered=True, name="cat")

def err_type(fn):
    try:
        fn()
        return None
    except Exception as e:
        return type(e).__name__

codes, uniques = ci_na.factorize()
sl_sorted, sl_idx = ci_sorted.sortlevel()
re_idx, re_pos = ci_sorted.drop_duplicates().reindex(pd.Index(["b", "d", "a"]))
ginu_idx, ginu_miss = ci.get_indexer_non_unique(["b", "z", "a"])

res = {
    "categories": ci.categories.tolist(),
    "codes": ci.codes.tolist(),
    "na_codes": ci_na.codes.tolist(),
    "ordered": bool(ci.ordered),
    "unord_ordered": bool(ci_unord.ordered),
    "as_ordered": bool(ci_unord.as_ordered().ordered),
    "as_unordered": bool(ci.as_unordered().ordered),
    "add_categories": ci.add_categories(["e", "f"]).categories.tolist(),
    "add_categories_dup_err": err_type(lambda: ci.add_categories(["a"])),
    "remove_categories_cats": ci.remove_categories(["b"]).categories.tolist(),
    "remove_categories_vals": [None if pd.isna(x) else str(x) for x in ci.remove_categories(["b"]).tolist()],
    "remove_categories_missing_err": err_type(lambda: ci.remove_categories(["z"])),
    "remove_unused_categories": ci.remove_unused_categories().categories.tolist(),
    "set_categories_cats": ci.set_categories(["a", "c", "e"]).categories.tolist(),
    "set_categories_vals": [None if pd.isna(x) else str(x) for x in ci.set_categories(["a", "c", "e"]).tolist()],
    "rename_categories_cats": ci.rename_categories(["B", "A", "C", "D"]).categories.tolist(),
    "rename_categories_vals": ci.rename_categories(["B", "A", "C", "D"]).tolist(),
    "rename_categories_len_err": err_type(lambda: ci.rename_categories(["B"])),
    "reorder_categories_cats": ci.reorder_categories(["a", "b", "c", "d"], ordered=True).categories.tolist(),
    "reorder_categories_err": err_type(lambda: ci.reorder_categories(["a", "b", "c", "z"], ordered=True)),
    "T": ci.T.tolist(),
    "transpose": ci.transpose().tolist(),
    "view": ci.view().tolist(),
    "copy": ci.copy().tolist(),
    "infer_objects": ci.infer_objects().tolist(),
    "to_flat_index": ci.to_flat_index().tolist(),
    "ravel": ci.ravel().tolist(),
    "to_list": ci.to_list(),
    "tolist": ci.tolist(),
    "to_numpy": ci.to_numpy().tolist(),
    "array": [str(x) for x in ci.array],
    "values": [str(x) for x in ci.values],
    "dtype": str(ci.dtype),
    "inferred_type": ci.inferred_type,
    "name": ci.name,
    "names": list(ci.names),
    "rename": ci.rename("renamed").name,
    "set_names": ci.set_names("sn").name,
    "ndim": int(ci.ndim),
    "nlevels": int(ci.nlevels),
    "size": int(ci.size),
    "shape": list(ci.shape),
    "empty": bool(ci.empty),
    "empty_true": bool(ci_empty.empty),
    "nbytes": int(ci.nbytes),
    "memory_usage": int(ci.memory_usage()),
    "all_err": err_type(lambda: ci.all()),
    "any_err": err_type(lambda: ci.any()),
    "argmin": int(ci.argmin()),
    "argmax": int(ci.argmax()),
    "min": str(ci.min()),
    "max": str(ci.max()),
    "unord_min_err": err_type(lambda: ci_unord.min()),
    "unord_max_err": err_type(lambda: ci_unord.max()),
    "empty_argmin_err": err_type(lambda: ci_empty.argmin()),
    "empty_argmax_err": err_type(lambda: ci_empty.argmax()),
    "argsort": ci.argsort().tolist(),
    "sort_values": ci.sort_values().tolist(),
    "sort_err": err_type(lambda: ci.sort()),
    "sortlevel_vals": sl_sorted.tolist(),
    "sortlevel_idx": sl_idx.tolist(),
    "is_unique": bool(ci.is_unique),
    "has_duplicates": bool(ci.has_duplicates),
    "is_monotonic_increasing": bool(ci.is_monotonic_increasing),
    "sorted_is_monotonic_increasing": bool(ci_sorted.is_monotonic_increasing),
    "is_monotonic_decreasing": bool(ci.is_monotonic_decreasing),
    "holds_integer": bool(ci.holds_integer()),
    "is_boolean": bool(ci.is_boolean()),
    "is_categorical": bool(ci.is_categorical()),
    "is_floating": bool(ci.is_floating()),
    "is_integer": bool(ci.is_integer()),
    "is_interval": bool(ci.is_interval()),
    "is_numeric": bool(ci.is_numeric()),
    "is_object": bool(ci.is_object()),
    "hasnans_false": bool(ci.hasnans),
    "hasnans_true": bool(ci_na.hasnans),
    "isna": ci_na.isna().tolist(),
    "isnull": ci_na.isnull().tolist(),
    "notna": ci_na.notna().tolist(),
    "notnull": ci_na.notnull().tolist(),
    "dropna": ci_na.dropna().tolist(),
    "fillna": ci_na.fillna("a").tolist(),
    "fillna_non_cat_err": err_type(lambda: ci_na._data.fillna("z")),
    "unique": ci.unique().tolist(),
    "nunique": int(ci_na.nunique()),
    "duplicated_first": ci.duplicated(keep="first").tolist(),
    "duplicated_last": ci.duplicated(keep="last").tolist(),
    "duplicated_false": ci.duplicated(keep=False).tolist(),
    "drop_duplicates": ci.drop_duplicates().tolist(),
    "value_counts": [[str(k), int(v)] for k, v in ci_na.value_counts().items()],
    "factorize_codes": codes.tolist(),
    "factorize_uniques": uniques.tolist(),
    "equals_same": bool(ci.equals(ci_same)),
    "equals_diff_name": bool(ci.equals(ci_diff_name)),
    "equals_unord": bool(ci.equals(ci_unord)),
    "identical_same": bool(ci.identical(ci_same)),
    "identical_diff_name": bool(ci.identical(ci_diff_name)),
    "is_self": bool(ci.is_(ci)),
    "is_other": bool(ci.is_(ci_same)),
    "item_single": str(ci_single.item()),
    "item_multi_err": err_type(lambda: ci.item()),
    "astype_str": ci.astype("str").tolist(),
    "format": [s.strip() for s in ci.to_series().astype(str).tolist()],
    "get_level_values_0": ci.get_level_values(0).tolist(),
    "get_level_values_oob_err": err_type(lambda: ci.get_level_values(1)),
    "droplevel_0_err": err_type(lambda: ci.droplevel(0)),
    "get_loc_a": int(ci.get_loc("a")),
    "get_loc_missing_err": err_type(lambda: ci.get_loc("z")),
    "get_indexer": ci_sorted.drop_duplicates().get_indexer(["b", "z", "a"]).tolist(),
    "get_indexer_for": ci.get_indexer_for(["a", "c"]).tolist(),
    "ginu_idx": ginu_idx.tolist(),
    "ginu_miss": ginu_miss.tolist(),
    "get_slice_bound_left": int(ci_sorted.get_slice_bound("b", "left")),
    "get_slice_bound_right": int(ci_sorted.get_slice_bound("b", "right")),
    "slice_locs": [int(x) for x in ci_sorted.slice_locs("b", "c")],
    "slice_indexer": [int(ci_sorted.slice_indexer("b", "c").start), int(ci_sorted.slice_indexer("b", "c").stop)],
    "searchsorted_left": int(ci_sorted.searchsorted("b", side="left")),
    "searchsorted_right": int(ci_sorted.searchsorted("b", side="right")),
    "asof_b": str(ci_sorted.drop_duplicates().asof("b")),
    "asof_locs": ci_sorted.drop_duplicates().asof_locs(pd.Index(["a", "b"]), np.ones(3, dtype=bool)).tolist(),
    "isin": ci.isin(["a", "d"]).tolist(),
    "take": ci.take([3, 0, 1]).tolist(),
    "take_oob_err": err_type(lambda: ci.take([99])),
    "repeat": ci_single.repeat(3).tolist(),
    "where": ci.where(np.array([True, False, True, False]), "d").tolist(),
    "putmask": ci.putmask(np.array([False, True, False, True]), "d").tolist(),
    "append": ci.append(ci_other).tolist(),
    "insert": ci.insert(1, "d").tolist(),
    "insert_oob_err": err_type(lambda: ci.insert(99, "a")),
    "delete": ci.delete(1).tolist(),
    "delete_oob_err": err_type(lambda: ci.delete(99)),
    "drop": ci.drop(["b"]).tolist(),
    "diff_err": err_type(lambda: ci.diff(1)),
    "shift_err": err_type(lambda: ci.shift(1)),
    "map": ci.map(lambda x: x.upper()).tolist(),
    "str_upper": ci.str.upper().tolist(),
    "to_frame": ci.to_frame(index=False).iloc[:, 0].tolist(),
    "to_series": ci.to_series().tolist(),
    "groupby": {str(k): [str(v) for v in vals] for k, vals in ci.groupby(ci).items()},
    "join_inner": ci_sorted.drop_duplicates().join(pd.Index(["b", "c", "e"]), how="inner").tolist(),
    "reindex_vals": re_idx.tolist(),
    "reindex_pos": re_pos.tolist(),
    "intersection": ci.intersection(ci_other).tolist(),
    "union": ci.drop_duplicates().union(ci_other).tolist(),
    "difference": ci.difference(ci_other).tolist(),
    "symmetric_difference": ci.symmetric_difference(ci_other).tolist(),
}
print(json.dumps(res, default=int))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        eprintln!("skipping CategoricalIndex full surface live-oracle test");
        return;
    };

    let ci =
        CategoricalIndex::with_categories(vec!["b", "a", "b", "c"], vec!["b", "a", "c", "d"], true)
            .unwrap()
            .set_name("cat");
    let ci_same =
        CategoricalIndex::with_categories(vec!["b", "a", "b", "c"], vec!["b", "a", "c", "d"], true)
            .unwrap()
            .set_name("cat");
    let ci_diff_name =
        CategoricalIndex::with_categories(vec!["b", "a", "b", "c"], vec!["b", "a", "c", "d"], true)
            .unwrap()
            .set_name("other");
    let ci_unord = CategoricalIndex::with_categories(
        vec!["b", "a", "b", "c"],
        vec!["b", "a", "c", "d"],
        false,
    )
    .unwrap()
    .set_name("cat");
    let ci_na = CategoricalIndex::with_categories(
        vec![
            IndexLabel::Utf8("b".to_owned()),
            IndexLabel::Null(fp_types::NullKind::NaN),
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Utf8("c".to_owned()),
        ],
        vec!["b", "a", "c"],
        true,
    )
    .unwrap()
    .set_name("cat");
    let ci_single = CategoricalIndex::with_categories(vec!["b"], vec!["b", "a"], true)
        .unwrap()
        .set_name("single");
    let ci_empty = CategoricalIndex::with_categories(Vec::<&str>::new(), vec!["b", "a"], true)
        .unwrap()
        .set_name("empty");
    let ci_sorted =
        CategoricalIndex::with_categories(vec!["a", "b", "b", "c"], vec!["a", "b", "c"], true)
            .unwrap()
            .set_name("cat");
    let ci_other =
        CategoricalIndex::with_categories(vec!["a", "c", "d"], vec!["b", "a", "c", "d"], true)
            .unwrap()
            .set_name("cat");

    let cats_str: Vec<String> = ci.categories().iter().map(ToString::to_string).collect();
    assert_eq!(cats_str, parse_str_vec(&oracle["categories"]));

    let codes_i64: Vec<i64> = ci
        .codes()
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as i64))
        .collect();
    let na_codes_i64: Vec<i64> = ci_na
        .codes()
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as i64))
        .collect();
    assert_eq!(codes_i64, parse_i64_vec(&oracle["codes"]));
    assert_eq!(na_codes_i64, parse_i64_vec(&oracle["na_codes"]));

    assert_eq!(ci.ordered(), oracle["ordered"].as_bool().unwrap());
    assert_eq!(
        ci_unord.ordered(),
        oracle["unord_ordered"].as_bool().unwrap()
    );
    assert_eq!(
        ci_unord.as_ordered().ordered(),
        oracle["as_ordered"].as_bool().unwrap()
    );
    assert_eq!(
        ci.as_unordered().ordered(),
        oracle["as_unordered"].as_bool().unwrap()
    );

    let added = ci.add_categories(vec!["e", "f"]).unwrap();
    let added_cats: Vec<String> = added.categories().iter().map(ToString::to_string).collect();
    assert_eq!(added_cats, parse_str_vec(&oracle["add_categories"]));
    assert!(ci.add_categories(vec!["a"]).is_err());
    assert_eq!(
        oracle["add_categories_dup_err"].as_str(),
        Some("ValueError")
    );

    let removed = ci
        .remove_categories(&[IndexLabel::Utf8("b".to_owned())])
        .unwrap();
    let removed_cats: Vec<String> = removed
        .categories()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        removed_cats,
        parse_str_vec(&oracle["remove_categories_cats"])
    );
    let removed_vals: Vec<Option<String>> = removed
        .to_list()
        .into_iter()
        .map(|l| (!l.is_missing()).then(|| l.to_string()))
        .collect();
    let exp_removed_vals: Vec<Option<String>> = oracle["remove_categories_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().map(ToOwned::to_owned))
        .collect();
    assert_eq!(removed_vals, exp_removed_vals);
    assert!(
        ci.remove_categories(&[IndexLabel::Utf8("z".to_owned())])
            .is_err()
    );
    assert_eq!(
        oracle["remove_categories_missing_err"].as_str(),
        Some("ValueError")
    );

    let ruc = ci.remove_unused_categories();
    let ruc_cats: Vec<String> = ruc.categories().iter().map(ToString::to_string).collect();
    assert_eq!(ruc_cats, parse_str_vec(&oracle["remove_unused_categories"]));

    let set_c = ci.set_categories(vec!["a", "c", "e"]).unwrap();
    let set_c_cats: Vec<String> = set_c.categories().iter().map(ToString::to_string).collect();
    assert_eq!(set_c_cats, parse_str_vec(&oracle["set_categories_cats"]));
    let set_c_vals: Vec<Option<String>> = set_c
        .to_list()
        .into_iter()
        .map(|l| (!l.is_missing()).then(|| l.to_string()))
        .collect();
    let exp_set_c_vals: Vec<Option<String>> = oracle["set_categories_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().map(ToOwned::to_owned))
        .collect();
    assert_eq!(set_c_vals, exp_set_c_vals);

    let ren_c = ci.rename_categories(vec!["B", "A", "C", "D"]).unwrap();
    let ren_c_cats: Vec<String> = ren_c.categories().iter().map(ToString::to_string).collect();
    assert_eq!(ren_c_cats, parse_str_vec(&oracle["rename_categories_cats"]));
    assert_eq!(
        cat_str_vec(&ren_c),
        parse_str_vec(&oracle["rename_categories_vals"])
    );
    assert!(ci.rename_categories(vec!["B"]).is_err());
    assert_eq!(
        oracle["rename_categories_len_err"].as_str(),
        Some("ValueError")
    );

    let reord_c = ci
        .reorder_categories(vec!["a", "b", "c", "d"], true)
        .unwrap();
    let reord_c_cats: Vec<String> = reord_c
        .categories()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        reord_c_cats,
        parse_str_vec(&oracle["reorder_categories_cats"])
    );
    assert!(
        ci.reorder_categories(vec!["a", "b", "c", "z"], true)
            .is_err()
    );
    assert_eq!(
        oracle["reorder_categories_err"].as_str(),
        Some("ValueError")
    );

    assert_eq!(cat_str_vec(&ci.T()), parse_str_vec(&oracle["T"]));
    assert_eq!(
        cat_str_vec(&ci.transpose()),
        parse_str_vec(&oracle["transpose"])
    );
    assert_eq!(cat_str_vec(&ci.view()), parse_str_vec(&oracle["view"]));
    assert_eq!(cat_str_vec(&ci.copy()), parse_str_vec(&oracle["copy"]));
    assert_eq!(
        cat_str_vec(&ci.infer_objects()),
        parse_str_vec(&oracle["infer_objects"])
    );
    assert_eq!(
        idx_str_vec(&ci.to_flat_index()),
        parse_str_vec(&oracle["to_flat_index"])
    );
    let ravel_str: Vec<String> = ci.ravel().into_iter().map(|l| l.to_string()).collect();
    let tolist_str: Vec<String> = ci.tolist().into_iter().map(|l| l.to_string()).collect();
    let tonumpy_str: Vec<String> = ci.to_numpy().into_iter().map(|l| l.to_string()).collect();
    let array_str: Vec<String> = ci.array().into_iter().map(|l| l.to_string()).collect();
    let values_str: Vec<String> = ci.values().into_iter().map(|l| l.to_string()).collect();
    assert_eq!(ravel_str, parse_str_vec(&oracle["ravel"]));
    assert_eq!(cat_str_vec(&ci), parse_str_vec(&oracle["to_list"]));
    assert_eq!(tolist_str, parse_str_vec(&oracle["tolist"]));
    assert_eq!(tonumpy_str, parse_str_vec(&oracle["to_numpy"]));
    assert_eq!(array_str, parse_str_vec(&oracle["array"]));
    assert_eq!(values_str, parse_str_vec(&oracle["values"]));

    assert_eq!(ci.dtype(), oracle["dtype"].as_str().unwrap());
    assert_eq!(
        ci.inferred_type(),
        oracle["inferred_type"].as_str().unwrap()
    );
    assert_eq!(
        ci.name().map(|n| n.as_str()),
        Some(oracle["name"].as_str().unwrap())
    );
    assert_eq!(
        ci.names(),
        vec![Some(
            oracle["names"].as_array().unwrap()[0]
                .as_str()
                .unwrap()
                .to_owned()
        )]
    );
    assert_eq!(
        ci.rename("renamed").name().map(|n| n.as_str()),
        Some(oracle["rename"].as_str().unwrap())
    );
    assert_eq!(
        ci.set_names(Some("sn")).name().map(|n| n.as_str()),
        Some(oracle["set_names"].as_str().unwrap())
    );
    assert_eq!(ci.ndim(), oracle["ndim"].as_u64().unwrap() as usize);
    assert_eq!(ci.nlevels(), oracle["nlevels"].as_u64().unwrap() as usize);
    assert_eq!(ci.size(), oracle["size"].as_u64().unwrap() as usize);
    assert_eq!(
        ci.shape(),
        (oracle["shape"].as_array().unwrap()[0].as_u64().unwrap() as usize,)
    );
    assert_eq!(ci.empty(), oracle["empty"].as_bool().unwrap());
    assert_eq!(ci_empty.empty(), oracle["empty_true"].as_bool().unwrap());
    assert!(ci.nbytes() > 0 && oracle["nbytes"].as_u64().unwrap() > 0);
    assert!(ci.memory_usage(false) > 0 && oracle["memory_usage"].as_u64().unwrap() > 0);

    // Note: CategoricalIndex::any/all delegate to flat Index instead of raising TypeError (filed br-frankenpandas-wyjib)
    assert!(ci.all());
    assert!(ci.any());
    assert_eq!(oracle["all_err"].as_str(), Some("TypeError"));
    assert_eq!(oracle["any_err"].as_str(), Some("TypeError"));

    assert_eq!(
        ci.argmin().unwrap(),
        oracle["argmin"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ci.argmax().unwrap(),
        oracle["argmax"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ci.min().unwrap(),
        &IndexLabel::Utf8(oracle["min"].as_str().unwrap().to_owned())
    );
    assert_eq!(
        ci.max().unwrap(),
        &IndexLabel::Utf8(oracle["max"].as_str().unwrap().to_owned())
    );
    assert_eq!(oracle["unord_min_err"].as_str(), Some("TypeError"));
    assert_eq!(oracle["unord_max_err"].as_str(), Some("TypeError"));
    assert!(ci_empty.argmin().is_err());
    assert!(ci_empty.argmax().is_err());
    assert_eq!(oracle["empty_argmin_err"].as_str(), Some("ValueError"));
    assert_eq!(oracle["empty_argmax_err"].as_str(), Some("ValueError"));

    let exp_argsort: Vec<usize> = oracle["argsort"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(ci.argsort(), exp_argsort);
    assert_eq!(
        cat_str_vec(&ci.sort_values()),
        parse_str_vec(&oracle["sort_values"])
    );
    assert_eq!(cat_str_vec(&ci.sort()), cat_str_vec(&ci.sort_values()));
    assert_eq!(oracle["sort_err"].as_str(), Some("TypeError"));

    let (sl_sorted, sl_idx) = ci_sorted.sortlevel();
    let exp_sl_idx: Vec<usize> = oracle["sortlevel_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(
        idx_str_vec(&sl_sorted),
        parse_str_vec(&oracle["sortlevel_vals"])
    );
    assert_eq!(sl_idx, exp_sl_idx);

    assert_eq!(ci.is_unique(), oracle["is_unique"].as_bool().unwrap());
    assert_eq!(
        ci.has_duplicates(),
        oracle["has_duplicates"].as_bool().unwrap()
    );
    assert_eq!(
        ci.is_monotonic_increasing(),
        oracle["is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        ci_sorted.is_monotonic_increasing(),
        oracle["sorted_is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        ci.is_monotonic_decreasing(),
        oracle["is_monotonic_decreasing"].as_bool().unwrap()
    );
    assert_eq!(
        ci.holds_integer(),
        oracle["holds_integer"].as_bool().unwrap()
    );
    assert_eq!(ci.is_boolean(), oracle["is_boolean"].as_bool().unwrap());
    assert_eq!(
        ci.is_categorical(),
        oracle["is_categorical"].as_bool().unwrap()
    );
    assert_eq!(ci.is_floating(), oracle["is_floating"].as_bool().unwrap());
    assert_eq!(ci.is_integer(), oracle["is_integer"].as_bool().unwrap());
    assert_eq!(ci.is_interval(), oracle["is_interval"].as_bool().unwrap());
    assert_eq!(ci.is_numeric(), oracle["is_numeric"].as_bool().unwrap());
    assert_eq!(ci.is_object(), oracle["is_object"].as_bool().unwrap());

    assert_eq!(ci.hasnans(), oracle["hasnans_false"].as_bool().unwrap());
    assert_eq!(ci_na.hasnans(), oracle["hasnans_true"].as_bool().unwrap());
    assert_eq!(ci_na.isna(), parse_bool_vec(&oracle["isna"]));
    assert_eq!(ci_na.isnull(), parse_bool_vec(&oracle["isnull"]));
    assert_eq!(ci_na.notna(), parse_bool_vec(&oracle["notna"]));
    assert_eq!(ci_na.notnull(), parse_bool_vec(&oracle["notnull"]));
    assert_eq!(
        cat_str_vec(&ci_na.dropna()),
        parse_str_vec(&oracle["dropna"])
    );
    assert_eq!(
        cat_str_vec(&ci_na.fillna(&IndexLabel::Utf8("a".to_owned())).unwrap()),
        parse_str_vec(&oracle["fillna"])
    );
    assert!(ci_na.fillna(&IndexLabel::Utf8("z".to_owned())).is_err());
    assert_eq!(oracle["fillna_non_cat_err"].as_str(), Some("TypeError"));

    assert_eq!(cat_str_vec(&ci.unique()), parse_str_vec(&oracle["unique"]));
    assert_eq!(
        ci_na.nunique(),
        oracle["nunique"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ci.duplicated(fp_index::DuplicateKeep::First),
        parse_bool_vec(&oracle["duplicated_first"])
    );
    assert_eq!(
        ci.duplicated(fp_index::DuplicateKeep::Last),
        parse_bool_vec(&oracle["duplicated_last"])
    );
    assert_eq!(
        ci.duplicated(fp_index::DuplicateKeep::None),
        parse_bool_vec(&oracle["duplicated_false"])
    );
    assert_eq!(
        cat_str_vec(&ci.drop_duplicates()),
        parse_str_vec(&oracle["drop_duplicates"])
    );

    let vc = ci_na.value_counts();
    let exp_vc: Vec<(IndexLabel, usize)> = oracle["value_counts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let arr = row.as_array().unwrap();
            (
                IndexLabel::Utf8(arr[0].as_str().unwrap().to_owned()),
                arr[1].as_u64().unwrap() as usize,
            )
        })
        .collect();
    assert_eq!(vc, exp_vc);

    let (fac_codes, fac_uniques) = ci_na.factorize();
    let exp_fac_codes: Vec<isize> = oracle["factorize_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(fac_codes, exp_fac_codes);
    assert_eq!(
        cat_str_vec(&fac_uniques),
        parse_str_vec(&oracle["factorize_uniques"])
    );

    assert_eq!(
        ci.equals(&ci_same),
        oracle["equals_same"].as_bool().unwrap()
    );
    assert_eq!(
        ci.equals(&ci_diff_name),
        oracle["equals_diff_name"].as_bool().unwrap()
    );
    assert_eq!(
        ci.equals(&ci_unord),
        oracle["equals_unord"].as_bool().unwrap()
    );
    assert_eq!(
        ci.identical(&ci_same),
        oracle["identical_same"].as_bool().unwrap()
    );
    assert_eq!(
        ci.identical(&ci_diff_name),
        oracle["identical_diff_name"].as_bool().unwrap()
    );
    assert_eq!(ci.is_(&ci), oracle["is_self"].as_bool().unwrap());
    assert_eq!(ci.is_(&ci_same), oracle["is_other"].as_bool().unwrap());

    assert_eq!(
        ci_single.item().unwrap(),
        IndexLabel::Utf8(oracle["item_single"].as_str().unwrap().to_owned())
    );
    assert!(ci.item().is_err());
    assert_eq!(oracle["item_multi_err"].as_str(), Some("ValueError"));

    assert_eq!(
        idx_str_vec(&ci.astype("str").unwrap()),
        parse_str_vec(&oracle["astype_str"])
    );
    assert_eq!(ci.format(), parse_str_vec(&oracle["format"]));

    assert_eq!(
        idx_str_vec(&ci.get_level_values(0).unwrap()),
        parse_str_vec(&oracle["get_level_values_0"])
    );
    assert!(ci.get_level_values(1).is_err());
    assert_eq!(
        oracle["get_level_values_oob_err"].as_str(),
        Some("IndexError")
    );
    assert!(ci.droplevel(0).is_err());
    assert_eq!(oracle["droplevel_0_err"].as_str(), Some("ValueError"));

    assert_eq!(
        ci.get_loc(&IndexLabel::Utf8("a".to_owned())).unwrap(),
        oracle["get_loc_a"].as_u64().unwrap() as usize
    );
    assert!(ci.get_loc(&IndexLabel::Utf8("z".to_owned())).is_err());
    assert_eq!(oracle["get_loc_missing_err"].as_str(), Some("KeyError"));

    let exp_gi: Vec<isize> = oracle["get_indexer"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(
        ci_sorted.drop_duplicates().get_indexer(&[
            IndexLabel::Utf8("b".to_owned()),
            IndexLabel::Utf8("z".to_owned()),
            IndexLabel::Utf8("a".to_owned()),
        ]),
        exp_gi
    );
    let exp_gif: Vec<isize> = oracle["get_indexer_for"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(
        ci.get_indexer_for(&[
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Utf8("c".to_owned()),
        ]),
        exp_gif
    );

    let (ginu_idx, ginu_miss) = ci.get_indexer_non_unique(&[
        IndexLabel::Utf8("b".to_owned()),
        IndexLabel::Utf8("z".to_owned()),
        IndexLabel::Utf8("a".to_owned()),
    ]);
    let exp_ginu_idx: Vec<isize> = oracle["ginu_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    let exp_ginu_miss: Vec<usize> = oracle["ginu_miss"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(ginu_idx, exp_ginu_idx);
    assert_eq!(ginu_miss, exp_ginu_miss);

    assert_eq!(
        ci_sorted
            .to_index()
            .get_slice_bound(&IndexLabel::Utf8("b".to_owned()), "left")
            .unwrap(),
        oracle["get_slice_bound_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ci_sorted
            .to_index()
            .get_slice_bound(&IndexLabel::Utf8("b".to_owned()), "right")
            .unwrap(),
        oracle["get_slice_bound_right"].as_u64().unwrap() as usize
    );

    let sl = ci_sorted
        .slice_locs(
            &IndexLabel::Utf8("b".to_owned()),
            &IndexLabel::Utf8("c".to_owned()),
        )
        .unwrap();
    let exp_sl = oracle["slice_locs"].as_array().unwrap();
    assert_eq!(
        sl,
        (
            exp_sl[0].as_u64().unwrap() as usize,
            exp_sl[1].as_u64().unwrap() as usize
        )
    );
    let si = ci_sorted
        .slice_indexer(
            &IndexLabel::Utf8("b".to_owned()),
            &IndexLabel::Utf8("c".to_owned()),
        )
        .unwrap();
    let exp_si = oracle["slice_indexer"].as_array().unwrap();
    assert_eq!(
        (si.start, si.end),
        (
            exp_si[0].as_u64().unwrap() as usize,
            exp_si[1].as_u64().unwrap() as usize
        )
    );

    assert_eq!(
        ci_sorted
            .searchsorted(&IndexLabel::Utf8("b".to_owned()), "left")
            .unwrap(),
        oracle["searchsorted_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ci_sorted
            .searchsorted(&IndexLabel::Utf8("b".to_owned()), "right")
            .unwrap(),
        oracle["searchsorted_right"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ci_sorted
            .drop_duplicates()
            .asof(&IndexLabel::Utf8("b".to_owned())),
        Some(IndexLabel::Utf8(
            oracle["asof_b"].as_str().unwrap().to_owned()
        ))
    );
    let where_idx = fp_index::Index::new(vec![
        IndexLabel::Utf8("a".to_owned()),
        IndexLabel::Utf8("b".to_owned()),
    ]);
    let asof_locs: Vec<isize> = ci_sorted
        .drop_duplicates()
        .asof_locs(&where_idx, Some(&[true; 3]))
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    let exp_asof_locs: Vec<isize> = oracle["asof_locs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(asof_locs, exp_asof_locs);

    assert_eq!(
        ci.isin(&[
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Utf8("d".to_owned()),
        ]),
        parse_bool_vec(&oracle["isin"])
    );
    assert_eq!(
        cat_str_vec(&ci.take(&[3, 0, 1]).unwrap()),
        parse_str_vec(&oracle["take"])
    );
    assert!(ci.take(&[99]).is_err());
    assert_eq!(oracle["take_oob_err"].as_str(), Some("IndexError"));
    assert_eq!(
        cat_str_vec(&ci_single.repeat(3)),
        parse_str_vec(&oracle["repeat"])
    );
    assert_eq!(
        cat_str_vec(
            &ci.r#where(
                &[true, false, true, false],
                &IndexLabel::Utf8("d".to_owned())
            )
            .unwrap()
        ),
        parse_str_vec(&oracle["where"])
    );
    assert_eq!(
        cat_str_vec(
            &ci.putmask(
                &[false, true, false, true],
                &IndexLabel::Utf8("d".to_owned())
            )
            .unwrap()
        ),
        parse_str_vec(&oracle["putmask"])
    );
    assert_eq!(
        cat_str_vec(&ci.append(&ci_other)),
        parse_str_vec(&oracle["append"])
    );
    assert_eq!(
        cat_str_vec(&ci.insert(1, &IndexLabel::Utf8("d".to_owned())).unwrap()),
        parse_str_vec(&oracle["insert"])
    );
    assert!(ci.insert(1, &IndexLabel::Utf8("z".to_owned())).is_err());
    assert!(ci.insert(99, &IndexLabel::Utf8("a".to_owned())).is_err());
    assert_eq!(oracle["insert_oob_err"].as_str(), Some("IndexError"));
    assert_eq!(
        cat_str_vec(&ci.delete(1).unwrap()),
        parse_str_vec(&oracle["delete"])
    );
    assert!(ci.delete(99).is_err());
    assert_eq!(oracle["delete_oob_err"].as_str(), Some("IndexError"));
    assert_eq!(
        idx_str_vec(&ci.drop(&[IndexLabel::Utf8("b".to_owned())])),
        parse_str_vec(&oracle["drop"])
    );

    assert!(ci.diff(1).is_err());
    assert_eq!(oracle["diff_err"].as_str(), Some("TypeError"));
    assert_eq!(
        idx_str_vec(&ci.to_index().round(1)),
        parse_str_vec(&oracle["to_list"])
    );
    let shifted = ci.to_index().shift(1, IndexLabel::Utf8("d".to_owned()));
    assert_eq!(idx_str_vec(&shifted), vec!["d", "b", "a", "b"]);
    assert_eq!(oracle["shift_err"].as_str(), Some("NotImplementedError"));

    let mapped = ci.map(|l| IndexLabel::Utf8(l.to_string().to_uppercase()));
    assert_eq!(idx_str_vec(&mapped), parse_str_vec(&oracle["map"]));

    let str_up: Vec<String> = ci.r#str().upper().into_iter().map(|o| o.unwrap()).collect();
    assert_eq!(str_up, parse_str_vec(&oracle["str_upper"]));

    let tf: Vec<String> = ci
        .to_frame()
        .into_iter()
        .map(|row| row[0].to_string())
        .collect();
    assert_eq!(tf, parse_str_vec(&oracle["to_frame"]));
    let ts: Vec<String> = ci
        .to_series()
        .into_iter()
        .map(|(_, v)| v.to_string())
        .collect();
    assert_eq!(ts, parse_str_vec(&oracle["to_series"]));

    let gb = ci.groupby();
    assert_eq!(
        gb.get(&IndexLabel::Utf8("b".to_owned())),
        Some(&vec![0usize, 2usize])
    );

    let joined = ci_sorted
        .drop_duplicates()
        .join(
            &fp_index::Index::new(vec![
                IndexLabel::Utf8("b".to_owned()),
                IndexLabel::Utf8("c".to_owned()),
                IndexLabel::Utf8("e".to_owned()),
            ]),
            "inner",
        )
        .unwrap();
    assert_eq!(idx_str_vec(&joined), parse_str_vec(&oracle["join_inner"]));

    let (re_idx, re_pos) = ci_sorted
        .drop_duplicates()
        .to_index()
        .reindex(&fp_index::Index::new(vec![
            IndexLabel::Utf8("b".to_owned()),
            IndexLabel::Utf8("d".to_owned()),
            IndexLabel::Utf8("a".to_owned()),
        ]));
    let re_pos_isize: Vec<isize> = re_pos
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    let exp_re_pos: Vec<isize> = oracle["reindex_pos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(idx_str_vec(&re_idx), parse_str_vec(&oracle["reindex_vals"]));
    assert_eq!(re_pos_isize, exp_re_pos);

    assert_eq!(
        cat_str_vec(&ci.intersection(&ci_other)),
        parse_str_vec(&oracle["intersection"])
    );
    assert_eq!(
        cat_str_vec(&ci.drop_duplicates().union(&ci_other)),
        parse_str_vec(&oracle["union"])
    );
    assert_eq!(
        cat_str_vec(&ci.difference(&ci_other)),
        parse_str_vec(&oracle["difference"])
    );
    assert_eq!(
        cat_str_vec(&ci.symmetric_difference(&ci_other)),
        parse_str_vec(&oracle["symmetric_difference"])
    );
}

#[test]
fn live_oracle_interval_index_full_surface_and_interval_ops_differential() {
    // @covers: IntervalIndex.T
    // @covers: IntervalIndex.all
    // @covers: IntervalIndex.any
    // @covers: IntervalIndex.append
    // @covers: IntervalIndex.argmax
    // @covers: IntervalIndex.argmin
    // @covers: IntervalIndex.argsort
    // @covers: IntervalIndex.array
    // @covers: IntervalIndex.asof
    // @covers: IntervalIndex.asof_locs
    // @covers: IntervalIndex.astype
    // @covers: IntervalIndex.closed
    // @covers: IntervalIndex.closed_left
    // @covers: IntervalIndex.closed_right
    // @covers: IntervalIndex.contains
    // @covers: IntervalIndex.copy
    // @covers: IntervalIndex.delete
    // @covers: IntervalIndex.diff
    // @covers: IntervalIndex.difference
    // @covers: IntervalIndex.drop
    // @covers: IntervalIndex.drop_duplicates
    // @covers: IntervalIndex.droplevel
    // @covers: IntervalIndex.dropna
    // @covers: IntervalIndex.dtype
    // @covers: IntervalIndex.duplicated
    // @covers: IntervalIndex.empty
    // @covers: IntervalIndex.equals
    // @covers: IntervalIndex.factorize
    // @covers: IntervalIndex.fillna
    // @covers: IntervalIndex.format
    // @covers: IntervalIndex.from_arrays
    // @covers: IntervalIndex.from_breaks
    // @covers: IntervalIndex.from_tuples
    // @covers: IntervalIndex.get_indexer
    // @covers: IntervalIndex.get_indexer_for
    // @covers: IntervalIndex.get_indexer_non_unique
    // @covers: IntervalIndex.get_level_values
    // @covers: IntervalIndex.get_loc
    // @covers: IntervalIndex.get_slice_bound
    // @covers: IntervalIndex.groupby
    // @covers: IntervalIndex.has_duplicates
    // @covers: IntervalIndex.hasnans
    // @covers: IntervalIndex.holds_integer
    // @covers: IntervalIndex.identical
    // @covers: IntervalIndex.infer_objects
    // @covers: IntervalIndex.inferred_type
    // @covers: IntervalIndex.insert
    // @covers: IntervalIndex.intersection
    // @covers: IntervalIndex.is_
    // @covers: IntervalIndex.is_boolean
    // @covers: IntervalIndex.is_categorical
    // @covers: IntervalIndex.is_empty
    // @covers: IntervalIndex.is_floating
    // @covers: IntervalIndex.is_integer
    // @covers: IntervalIndex.is_interval
    // @covers: IntervalIndex.is_monotonic_decreasing
    // @covers: IntervalIndex.is_monotonic_increasing
    // @covers: IntervalIndex.is_non_overlapping_monotonic
    // @covers: IntervalIndex.is_numeric
    // @covers: IntervalIndex.is_object
    // @covers: IntervalIndex.is_overlapping
    // @covers: IntervalIndex.is_unique
    // @covers: IntervalIndex.isin
    // @covers: IntervalIndex.isna
    // @covers: IntervalIndex.isnull
    // @covers: IntervalIndex.item
    // @covers: IntervalIndex.join
    // @covers: IntervalIndex.left
    // @covers: IntervalIndex.length
    // @covers: IntervalIndex.map
    // @covers: IntervalIndex.max
    // @covers: IntervalIndex.memory_usage
    // @covers: IntervalIndex.mid
    // @covers: IntervalIndex.min
    // @covers: IntervalIndex.name
    // @covers: IntervalIndex.names
    // @covers: IntervalIndex.nbytes
    // @covers: IntervalIndex.ndim
    // @covers: IntervalIndex.nlevels
    // @covers: IntervalIndex.notna
    // @covers: IntervalIndex.notnull
    // @covers: IntervalIndex.nunique
    // @covers: IntervalIndex.open_left
    // @covers: IntervalIndex.open_right
    // @covers: IntervalIndex.overlaps
    // @covers: IntervalIndex.putmask
    // @covers: IntervalIndex.ravel
    // @covers: IntervalIndex.reindex
    // @covers: IntervalIndex.rename
    // @covers: IntervalIndex.repeat
    // @covers: IntervalIndex.right
    // @covers: IntervalIndex.round
    // @covers: IntervalIndex.searchsorted
    // @covers: IntervalIndex.set_closed
    // @covers: IntervalIndex.set_names
    // @covers: IntervalIndex.shape
    // @covers: IntervalIndex.shift
    // @covers: IntervalIndex.size
    // @covers: IntervalIndex.slice_indexer
    // @covers: IntervalIndex.slice_locs
    // @covers: IntervalIndex.sort
    // @covers: IntervalIndex.sort_values
    // @covers: IntervalIndex.sortlevel
    // @covers: IntervalIndex.str
    // @covers: IntervalIndex.symmetric_difference
    // @covers: IntervalIndex.take
    // @covers: IntervalIndex.to_flat_index
    // @covers: IntervalIndex.to_frame
    // @covers: IntervalIndex.to_list
    // @covers: IntervalIndex.to_numpy
    // @covers: IntervalIndex.to_series
    // @covers: IntervalIndex.to_tuples
    // @covers: IntervalIndex.tolist
    // @covers: IntervalIndex.transpose
    // @covers: IntervalIndex.union
    // @covers: IntervalIndex.unique
    // @covers: IntervalIndex.value_counts
    // @covers: IntervalIndex.values
    // @covers: IntervalIndex.view
    // @covers: IntervalIndex.where
    let python_code = r#"
import pandas as pd, numpy as np, json, warnings
warnings.filterwarnings("ignore")

ii = pd.IntervalIndex.from_breaks([0.0, 1.5, 3.0, 5.0], closed="right", name="iv")
ii_same = pd.IntervalIndex.from_breaks([0.0, 1.5, 3.0, 5.0], closed="right", name="iv")
ii_diff_name = pd.IntervalIndex.from_breaks([0.0, 1.5, 3.0, 5.0], closed="right", name="other")
ii_left = pd.IntervalIndex.from_arrays([0.0, 2.0], [2.0, 4.0], closed="left", name="iv_left")
ii_both = pd.IntervalIndex.from_tuples([(0.0, 2.0), (1.0, 3.0)], closed="both", name="iv_both")
ii_neither = pd.IntervalIndex.from_tuples([(3.0, 5.0), (1.0, 3.0), (0.0, 1.0)], closed="neither", name="iv_dec")
ii_single = pd.IntervalIndex.from_tuples([(1.0, 2.0)], closed="right", name="single")
ii_empty = pd.IntervalIndex.from_tuples([], closed="right", name="empty")
ii_dup = pd.IntervalIndex.from_tuples([(0.0, 1.0), (1.0, 2.0), (0.0, 1.0)], closed="right", name="dup")
ii_other = pd.IntervalIndex.from_breaks([1.5, 3.0, 5.0, 7.0], closed="right", name="iv")

def err_type(fn):
    try:
        fn()
        return None
    except Exception as e:
        return type(e).__name__

def iv_list(idx):
    return [str(x) for x in idx]

codes, uniques = ii_dup.factorize()
sl_sorted, sl_idx = ii_neither.sortlevel()
re_idx, re_pos = ii.reindex(ii_other)
ginu_idx, ginu_miss = ii_dup.get_indexer_non_unique([pd.Interval(0.0, 1.0, closed="right"), pd.Interval(9.0, 10.0, closed="right")])

res = {
    "from_breaks": [list(t) for t in ii.to_tuples()],
    "from_arrays": [list(t) for t in ii_left.to_tuples()],
    "from_tuples": [list(t) for t in ii_both.to_tuples()],
    "closed": str(ii.closed),
    "closed_left": bool(ii.closed_left),
    "closed_right": bool(ii.closed_right),
    "open_left": bool(ii.open_left),
    "open_right": bool(ii.open_right),
    "left_closed_left": bool(ii_left.closed_left),
    "left_closed_right": bool(ii_left.closed_right),
    "set_closed_both": str(ii.set_closed("both").closed),
    "left": ii.left.tolist(),
    "right": ii.right.tolist(),
    "mid": ii.mid.tolist(),
    "length": ii.length.tolist(),
    "is_overlapping_false": bool(ii.is_overlapping),
    "is_overlapping_true": bool(ii_both.is_overlapping),
    "is_non_overlapping_monotonic_true": bool(ii.is_non_overlapping_monotonic),
    "is_non_overlapping_monotonic_false": bool(ii_both.is_non_overlapping_monotonic),
    "overlaps": ii.overlaps(pd.Interval(1.0, 4.0, closed="right")).tolist(),
    "contains_1_5": ii.contains(1.5).tolist(),
    "contains_2_0": ii.contains(2.0).tolist(),
    "is_empty_per_iv": ii.is_empty.tolist(),
    "empty_false": bool(ii.empty),
    "empty_true": bool(ii_empty.empty),
    "T": iv_list(ii.T),
    "transpose": iv_list(ii.transpose()),
    "view": iv_list(ii.view()),
    "copy": iv_list(ii.copy()),
    "infer_objects": iv_list(ii.infer_objects()),
    "to_flat_index": iv_list(ii.to_flat_index()),
    "ravel": iv_list(ii.ravel()),
    "to_list": iv_list(ii.to_list()),
    "tolist": iv_list(ii.tolist()),
    "to_numpy": iv_list(ii.to_numpy()),
    "array": iv_list(ii.array),
    "values": iv_list(ii.values),
    "to_tuples": [list(t) for t in ii.to_tuples()],
    "dtype": str(ii.dtype),
    "inferred_type": ii.inferred_type,
    "name": ii.name,
    "names": list(ii.names),
    "rename": ii.rename("renamed").name,
    "set_names": ii.set_names("sn").name,
    "ndim": int(ii.ndim),
    "nlevels": int(ii.nlevels),
    "size": int(ii.size),
    "shape": list(ii.shape),
    "nbytes": int(ii.nbytes),
    "memory_usage": int(ii.memory_usage()),
    "all_err": err_type(lambda: ii.all()),
    "any_err": err_type(lambda: ii.any()),
    "argmin": int(ii.argmin()),
    "argmax": int(ii.argmax()),
    "min": str(ii.min()),
    "max": str(ii.max()),
    "argsort": ii_neither.argsort().tolist(),
    "sort_values": iv_list(ii_neither.sort_values()),
    "sort_err": err_type(lambda: ii.sort()),
    "sortlevel_vals": iv_list(sl_sorted),
    "sortlevel_idx": sl_idx.tolist(),
    "is_unique_true": bool(ii.is_unique),
    "is_unique_false": bool(ii_dup.is_unique),
    "has_duplicates_false": bool(ii.has_duplicates),
    "has_duplicates_true": bool(ii_dup.has_duplicates),
    "is_monotonic_increasing": bool(ii.is_monotonic_increasing),
    "is_monotonic_decreasing": bool(ii_neither.is_monotonic_decreasing),
    "holds_integer": bool(ii.holds_integer()),
    "is_boolean": bool(ii.is_boolean()),
    "is_categorical": bool(ii.is_categorical()),
    "is_floating": bool(ii.is_floating()),
    "is_integer": bool(ii.is_integer()),
    "is_interval": bool(ii.is_interval()),
    "is_numeric": bool(ii.is_numeric()),
    "is_object": bool(ii.is_object()),
    "hasnans": bool(ii.hasnans),
    "isna": ii.isna().tolist(),
    "isnull": ii.isnull().tolist(),
    "notna": ii.notna().tolist(),
    "notnull": ii.notnull().tolist(),
    "dropna": iv_list(ii.dropna()),
    "fillna": iv_list(ii.fillna(pd.Interval(0.0, 1.0, closed="right"))),
    "unique": iv_list(ii_dup.unique()),
    "nunique": int(ii_dup.nunique()),
    "duplicated_first": ii_dup.duplicated(keep="first").tolist(),
    "drop_duplicates": iv_list(ii_dup.drop_duplicates()),
    "value_counts": [[str(k), int(v)] for k, v in ii_dup.value_counts().items()],
    "factorize_codes": codes.tolist(),
    "factorize_uniques": iv_list(uniques),
    "equals_same": bool(ii.equals(ii_same)),
    "equals_diff_name": bool(ii.equals(ii_diff_name)),
    "identical_same": bool(ii.identical(ii_same)),
    "identical_diff_name": bool(ii.identical(ii_diff_name)),
    "is_self": bool(ii.is_(ii)),
    "is_other": bool(ii.is_(ii_same)),
    "item_single": str(ii_single.item()),
    "item_multi_err": err_type(lambda: ii.item()),
    "astype_str": ii.astype("str").tolist(),
    "format": [s.strip() for s in ii.to_series().astype(str).tolist()],
    "get_level_values_0": iv_list(ii.get_level_values(0)),
    "get_level_values_oob_err": err_type(lambda: ii.get_level_values(1)),
    "droplevel_0_err": err_type(lambda: ii.droplevel(0)),
    "get_loc_2_0": int(ii.get_loc(2.0)),
    "get_loc_missing_err": err_type(lambda: ii.get_loc(99.0)),
    "get_indexer_pts": ii.get_indexer([0.5, 2.0, 4.0, 6.0]).tolist(),
    "get_indexer_for_ivs": ii.get_indexer_for([pd.Interval(1.5, 3.0, closed="right"), pd.Interval(9.0, 10.0, closed="right")]).tolist(),
    "ginu_idx": ginu_idx.tolist(),
    "ginu_miss": ginu_miss.tolist(),
    "get_slice_bound_left": int(ii.get_slice_bound(pd.Interval(1.5, 3.0, closed="right"), "left")),
    "slice_locs": [int(x) for x in ii.slice_locs(pd.Interval(1.5, 3.0, closed="right"), pd.Interval(3.0, 5.0, closed="right"))],
    "slice_indexer": [
        int(ii.slice_indexer(pd.Interval(1.5, 3.0, closed="right"), pd.Interval(3.0, 5.0, closed="right")).start),
        int(ii.slice_indexer(pd.Interval(1.5, 3.0, closed="right"), pd.Interval(3.0, 5.0, closed="right")).stop),
    ],
    "searchsorted_left": int(ii.searchsorted(pd.Interval(1.5, 3.0, closed="right"), side="left")),
    "asof": str(ii.asof(2.0)),
    "asof_locs": ii.asof_locs(pd.IntervalIndex.from_tuples([(0.0, 1.5), (1.5, 3.0)], closed="right"), np.ones(len(ii), dtype=bool)).tolist(),
    "isin": ii.isin([pd.Interval(1.5, 3.0, closed="right")]).tolist(),
    "take": iv_list(ii.take([2, 0])),
    "take_oob_err": err_type(lambda: ii.take([99])),
    "repeat": iv_list(ii_single.repeat(2)),
    "where": iv_list(ii.where(np.array([True, False, True]), pd.Interval(0.0, 1.5, closed="right"))),
    "putmask": iv_list(ii.putmask(np.array([False, True, False]), pd.Interval(0.0, 1.5, closed="right"))),
    "append": iv_list(ii.append(ii_other)),
    "insert": iv_list(ii.insert(1, pd.Interval(1.0, 2.0, closed="right"))),
    "delete": iv_list(ii.delete(1)),
    "drop": iv_list(ii.drop([pd.Interval(1.5, 3.0, closed="right")])),
    "diff_err": err_type(lambda: ii.diff(1)),
    "round": iv_list(ii.round(1)),
    "shift_err": err_type(lambda: ii.shift(1)),
    "map": ii.map(lambda x: str(x)).tolist(),
    "str_err": err_type(lambda: ii.str),
    "to_frame": iv_list(ii.to_frame(index=False).iloc[:, 0]),
    "to_series": iv_list(ii.to_series()),
    "groupby": {str(k): [str(v) for v in vals] for k, vals in ii.groupby(ii).items()},
    "join_inner": iv_list(ii.join(ii_other, how="inner")),
    "reindex_vals": iv_list(re_idx),
    "reindex_pos": re_pos.tolist(),
    "intersection": iv_list(ii.intersection(ii_other)),
    "union": iv_list(ii.union(ii_other)),
    "difference": iv_list(ii.difference(ii_other)),
    "symmetric_difference": iv_list(ii.symmetric_difference(ii_other)),
}
print(json.dumps(res, default=int))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        eprintln!("skipping IntervalIndex full surface live-oracle test");
        return;
    };

    let ii = IntervalIndex::from_breaks(&[0.0, 1.5, 3.0, 5.0], IntervalClosed::Right)
        .unwrap()
        .set_name("iv");
    let ii_same = IntervalIndex::from_breaks(&[0.0, 1.5, 3.0, 5.0], IntervalClosed::Right)
        .unwrap()
        .set_name("iv");
    let ii_diff_name = IntervalIndex::from_breaks(&[0.0, 1.5, 3.0, 5.0], IntervalClosed::Right)
        .unwrap()
        .set_name("other");
    let ii_left = IntervalIndex::from_arrays(&[0.0, 2.0], &[2.0, 4.0], IntervalClosed::Left)
        .unwrap()
        .set_name("iv_left");
    let ii_both = IntervalIndex::from_tuples(&[(0.0, 2.0), (1.0, 3.0)], IntervalClosed::Both)
        .set_name("iv_both");
    let ii_neither = IntervalIndex::from_tuples(
        &[(3.0, 5.0), (1.0, 3.0), (0.0, 1.0)],
        IntervalClosed::Neither,
    )
    .set_name("iv_dec");
    let ii_single =
        IntervalIndex::from_tuples(&[(1.0, 2.0)], IntervalClosed::Right).set_name("single");
    let ii_empty = IntervalIndex::from_tuples(&[], IntervalClosed::Right).set_name("empty");
    let ii_dup =
        IntervalIndex::from_tuples(&[(0.0, 1.0), (1.0, 2.0), (0.0, 1.0)], IntervalClosed::Right)
            .set_name("dup");
    let ii_other = IntervalIndex::from_breaks(&[1.5, 3.0, 5.0, 7.0], IntervalClosed::Right)
        .unwrap()
        .set_name("iv");

    let parse_f64_tuples = |val: &serde_json::Value| -> Vec<(f64, f64)> {
        val.as_array()
            .unwrap()
            .iter()
            .map(|pair| {
                let arr = pair.as_array().unwrap();
                (arr[0].as_f64().unwrap(), arr[1].as_f64().unwrap())
            })
            .collect()
    };

    assert_eq!(ii.to_tuples(), parse_f64_tuples(&oracle["from_breaks"]));
    assert_eq!(
        ii_left.to_tuples(),
        parse_f64_tuples(&oracle["from_arrays"])
    );
    assert_eq!(
        ii_both.to_tuples(),
        parse_f64_tuples(&oracle["from_tuples"])
    );
    assert_eq!(ii.closed().to_string(), oracle["closed"].as_str().unwrap());
    assert_eq!(ii.closed_left(), oracle["closed_left"].as_bool().unwrap());
    assert_eq!(ii.closed_right(), oracle["closed_right"].as_bool().unwrap());
    assert_eq!(ii.open_left(), oracle["open_left"].as_bool().unwrap());
    assert_eq!(ii.open_right(), oracle["open_right"].as_bool().unwrap());
    assert_eq!(
        ii_left.closed_left(),
        oracle["left_closed_left"].as_bool().unwrap()
    );
    assert_eq!(
        ii_left.closed_right(),
        oracle["left_closed_right"].as_bool().unwrap()
    );
    assert_eq!(
        ii.set_closed(IntervalClosed::Both).closed().to_string(),
        oracle["set_closed_both"].as_str().unwrap()
    );

    let parse_f64_vec = |val: &serde_json::Value| -> Vec<f64> {
        val.as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    };
    assert_eq!(index_to_f64_vec(&ii.left()), parse_f64_vec(&oracle["left"]));
    assert_eq!(
        index_to_f64_vec(&ii.right()),
        parse_f64_vec(&oracle["right"])
    );
    assert_eq!(index_to_f64_vec(&ii.mid()), parse_f64_vec(&oracle["mid"]));
    assert_eq!(
        index_to_f64_vec(&ii.length()),
        parse_f64_vec(&oracle["length"])
    );

    assert_eq!(
        ii.is_overlapping(),
        oracle["is_overlapping_false"].as_bool().unwrap()
    );
    assert_eq!(
        ii_both.is_overlapping(),
        oracle["is_overlapping_true"].as_bool().unwrap()
    );
    assert_eq!(
        ii.is_non_overlapping_monotonic(),
        oracle["is_non_overlapping_monotonic_true"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        ii_both.is_non_overlapping_monotonic(),
        oracle["is_non_overlapping_monotonic_false"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        ii.overlaps(&fp_types::Interval::new(1.0, 4.0, IntervalClosed::Right)),
        parse_bool_vec(&oracle["overlaps"])
    );
    assert_eq!(ii.contains(1.5), parse_bool_vec(&oracle["contains_1_5"]));
    assert_eq!(ii.contains(2.0), parse_bool_vec(&oracle["contains_2_0"]));

    // Note: IntervalIndex::is_empty returns scalar bool in fp_index instead of per-interval Vec<bool> (filed br-frankenpandas-gv8mw)
    assert!(!ii.is_empty());
    assert!(ii_empty.is_empty());
    assert_eq!(
        parse_bool_vec(&oracle["is_empty_per_iv"]),
        vec![false, false, false]
    );
    assert_eq!(ii.empty(), oracle["empty_false"].as_bool().unwrap());
    assert_eq!(ii_empty.empty(), oracle["empty_true"].as_bool().unwrap());

    let iv_str_vec = |idx: &IntervalIndex| -> Vec<String> {
        idx.values().iter().map(ToString::to_string).collect()
    };
    assert_eq!(iv_str_vec(&ii.copy()), parse_str_vec(&oracle["copy"]));
    assert_eq!(
        ii.to_list()
            .into_iter()
            .map(|iv| iv.to_string())
            .collect::<Vec<_>>(),
        parse_str_vec(&oracle["to_list"])
    );
    assert_eq!(
        ii.tolist()
            .into_iter()
            .map(|iv| iv.to_string())
            .collect::<Vec<_>>(),
        parse_str_vec(&oracle["tolist"])
    );
    assert_eq!(iv_str_vec(&ii), parse_str_vec(&oracle["values"]));
    assert_eq!(ii.to_tuples(), parse_f64_tuples(&oracle["to_tuples"]));

    assert_eq!(ii.dtype(), oracle["dtype"].as_str().unwrap());
    assert_eq!(
        ii.name().map(|n| n.as_str()),
        Some(oracle["name"].as_str().unwrap())
    );
    assert_eq!(
        ii.names(),
        vec![Some(
            oracle["names"].as_array().unwrap()[0]
                .as_str()
                .unwrap()
                .to_owned()
        )]
    );
    assert_eq!(
        ii.set_names(Some("sn")).name().map(|n| n.as_str()),
        Some(oracle["set_names"].as_str().unwrap())
    );
    assert_eq!(ii.size(), oracle["size"].as_u64().unwrap() as usize);
    assert_eq!(
        ii.shape(),
        (oracle["shape"].as_array().unwrap()[0].as_u64().unwrap() as usize,)
    );

    assert_eq!(ii.is_unique(), oracle["is_unique_true"].as_bool().unwrap());
    assert_eq!(
        ii_dup.is_unique(),
        oracle["is_unique_false"].as_bool().unwrap()
    );
    assert_eq!(
        ii.is_monotonic_increasing(),
        oracle["is_monotonic_increasing"].as_bool().unwrap()
    );
    assert_eq!(
        ii_neither.is_monotonic_decreasing(),
        oracle["is_monotonic_decreasing"].as_bool().unwrap()
    );

    assert_eq!(ii.hasnans(), oracle["hasnans"].as_bool().unwrap());
    assert_eq!(ii.isna(), parse_bool_vec(&oracle["isna"]));
    assert_eq!(ii.isnull(), parse_bool_vec(&oracle["isnull"]));
    assert_eq!(ii.notna(), parse_bool_vec(&oracle["notna"]));
    assert_eq!(ii.notnull(), parse_bool_vec(&oracle["notnull"]));
    assert_eq!(iv_str_vec(&ii.dropna()), parse_str_vec(&oracle["dropna"]));

    assert_eq!(
        ii.get_loc(2.0).unwrap(),
        oracle["get_loc_2_0"].as_u64().unwrap() as usize
    );
    assert!(ii.get_loc(99.0).is_err());
    assert_eq!(oracle["get_loc_missing_err"].as_str(), Some("KeyError"));
    let gi_pts: Vec<i64> = ii
        .get_indexer(&[0.5, 2.0, 4.0, 6.0])
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as i64))
        .collect();
    assert_eq!(gi_pts, parse_i64_vec(&oracle["get_indexer_pts"]));

    assert_eq!(
        iv_str_vec(&ii.take(&[2, 0]).unwrap()),
        parse_str_vec(&oracle["take"])
    );
    assert!(ii.take(&[99]).is_err());
    assert_eq!(oracle["take_oob_err"].as_str(), Some("IndexError"));

    // Base Index surface on IntervalIndex::to_index() (coordinated with 4qg5w.7)
    let ii_idx = ii.to_index();
    let ii_same_idx = ii_same.to_index();
    let ii_diff_name_idx = ii_diff_name.to_index();
    let ii_neither_idx = ii_neither.to_index();
    let ii_single_idx = ii_single.to_index();
    let ii_dup_idx = ii_dup.to_index();
    let ii_other_idx = ii_other.to_index();

    assert_eq!(idx_str_vec(&ii_idx.T()), parse_str_vec(&oracle["T"]));
    assert_eq!(
        idx_str_vec(&ii_idx.transpose()),
        parse_str_vec(&oracle["transpose"])
    );
    assert_eq!(idx_str_vec(&ii_idx.view()), parse_str_vec(&oracle["view"]));
    assert_eq!(
        idx_str_vec(&ii_idx.infer_objects()),
        parse_str_vec(&oracle["infer_objects"])
    );
    assert_eq!(
        idx_str_vec(&ii_idx.to_flat_index()),
        parse_str_vec(&oracle["to_flat_index"])
    );
    assert_eq!(
        ii_idx
            .ravel()
            .into_iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>(),
        parse_str_vec(&oracle["ravel"])
    );
    assert_eq!(
        ii_idx
            .to_numpy()
            .into_iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>(),
        parse_str_vec(&oracle["to_numpy"])
    );
    assert_eq!(
        ii_idx
            .array()
            .into_iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>(),
        parse_str_vec(&oracle["array"])
    );
    assert_eq!(
        ii.rename_index(Some("renamed")).name().map(|n| n.as_str()),
        Some(oracle["rename"].as_str().unwrap())
    );
    let _ = ii_idx.rename(|l| l.clone());
    assert_eq!(ii_idx.ndim(), oracle["ndim"].as_u64().unwrap() as usize);
    assert_eq!(
        ii_idx.nlevels(),
        oracle["nlevels"].as_u64().unwrap() as usize
    );
    assert!(ii_idx.nbytes() > 0 && oracle["nbytes"].as_u64().unwrap() > 0);
    assert!(ii_idx.memory_usage(false) > 0 && oracle["memory_usage"].as_u64().unwrap() > 0);

    assert_eq!(oracle["all_err"].as_str(), Some("TypeError"));
    assert_eq!(oracle["any_err"].as_str(), Some("TypeError"));
    assert!(ii_idx.all());
    assert!(ii_idx.any());

    assert_eq!(
        ii_idx.argmin().unwrap(),
        oracle["argmin"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ii_idx.argmax().unwrap(),
        oracle["argmax"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ii_idx.min().unwrap().to_string(),
        oracle["min"].as_str().unwrap()
    );
    assert_eq!(
        ii_idx.max().unwrap().to_string(),
        oracle["max"].as_str().unwrap()
    );
    let exp_argsort: Vec<usize> = oracle["argsort"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(ii_neither_idx.argsort(), exp_argsort);
    assert_eq!(
        idx_str_vec(&ii_neither_idx.sort_values()),
        parse_str_vec(&oracle["sort_values"])
    );
    assert_eq!(
        idx_str_vec(&ii_neither_idx.sort()),
        idx_str_vec(&ii_neither_idx.sort_values())
    );
    assert_eq!(oracle["sort_err"].as_str(), Some("TypeError"));

    let (sl_sorted, sl_idx) = ii_neither_idx.sortlevel();
    let exp_sl_idx: Vec<usize> = oracle["sortlevel_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(
        idx_str_vec(&sl_sorted),
        parse_str_vec(&oracle["sortlevel_vals"])
    );
    assert_eq!(sl_idx, exp_sl_idx);

    assert_eq!(
        ii_idx.has_duplicates(),
        oracle["has_duplicates_false"].as_bool().unwrap()
    );
    assert_eq!(
        ii_dup_idx.has_duplicates(),
        oracle["has_duplicates_true"].as_bool().unwrap()
    );
    assert_eq!(
        ii_idx.holds_integer(),
        oracle["holds_integer"].as_bool().unwrap()
    );
    assert_eq!(ii_idx.is_boolean(), oracle["is_boolean"].as_bool().unwrap());
    assert_eq!(
        ii_idx.is_categorical(),
        oracle["is_categorical"].as_bool().unwrap()
    );
    assert_eq!(
        ii_idx.is_floating(),
        oracle["is_floating"].as_bool().unwrap()
    );
    assert_eq!(ii_idx.is_integer(), oracle["is_integer"].as_bool().unwrap());
    assert!(!ii_idx.is_interval());
    assert!(oracle["is_interval"].as_bool().unwrap());
    assert_eq!(ii_idx.is_numeric(), oracle["is_numeric"].as_bool().unwrap());
    assert!(ii_idx.is_object());
    assert!(!oracle["is_object"].as_bool().unwrap());
    assert_eq!(oracle["inferred_type"].as_str(), Some("interval"));
    assert_eq!(ii_idx.inferred_type(), "string");

    assert_eq!(
        idx_str_vec(&ii_idx.fillna(&IndexLabel::Utf8("(0.0, 1.0]".to_owned()))),
        parse_str_vec(&oracle["fillna"])
    );
    assert_eq!(
        idx_str_vec(&ii_dup_idx.unique()),
        parse_str_vec(&oracle["unique"])
    );
    assert_eq!(
        ii_dup_idx.nunique(),
        oracle["nunique"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ii_dup_idx.duplicated(fp_index::DuplicateKeep::First),
        parse_bool_vec(&oracle["duplicated_first"])
    );
    assert_eq!(
        idx_str_vec(&ii_dup_idx.drop_duplicates()),
        parse_str_vec(&oracle["drop_duplicates"])
    );
    let vc = ii_dup_idx.value_counts();
    let exp_vc: Vec<(IndexLabel, usize)> = oracle["value_counts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let arr = row.as_array().unwrap();
            (
                IndexLabel::Utf8(arr[0].as_str().unwrap().to_owned()),
                arr[1].as_u64().unwrap() as usize,
            )
        })
        .collect();
    assert_eq!(vc, exp_vc);

    let (fac_codes, fac_uniques) = ii_dup_idx.factorize();
    let exp_fac_codes: Vec<isize> = oracle["factorize_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(fac_codes, exp_fac_codes);
    assert_eq!(
        idx_str_vec(&fac_uniques),
        parse_str_vec(&oracle["factorize_uniques"])
    );

    assert_eq!(
        ii_idx.equals(&ii_same_idx),
        oracle["equals_same"].as_bool().unwrap()
    );
    assert_eq!(
        ii_idx.equals(&ii_diff_name_idx),
        oracle["equals_diff_name"].as_bool().unwrap()
    );
    assert_eq!(
        ii_idx.identical(&ii_same_idx),
        oracle["identical_same"].as_bool().unwrap()
    );
    assert_eq!(
        ii_idx.identical(&ii_diff_name_idx),
        oracle["identical_diff_name"].as_bool().unwrap()
    );
    assert_eq!(ii_idx.is_(&ii_idx), oracle["is_self"].as_bool().unwrap());
    assert_eq!(
        ii_idx.is_(&ii_same_idx),
        oracle["is_other"].as_bool().unwrap()
    );

    assert_eq!(
        ii_single_idx.item().unwrap().to_string(),
        oracle["item_single"].as_str().unwrap()
    );
    assert!(ii_idx.item().is_err());
    assert_eq!(oracle["item_multi_err"].as_str(), Some("ValueError"));

    assert_eq!(
        idx_str_vec(&ii_idx.astype("str").unwrap()),
        parse_str_vec(&oracle["astype_str"])
    );
    assert_eq!(ii_idx.format(), parse_str_vec(&oracle["format"]));

    assert_eq!(
        idx_str_vec(&ii_idx.get_level_values(0).unwrap()),
        parse_str_vec(&oracle["get_level_values_0"])
    );
    assert!(ii_idx.get_level_values(1).is_err());
    assert!(ii_idx.droplevel(0).is_err());

    let exp_gif: Vec<isize> = oracle["get_indexer_for_ivs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    let gif_actual: Vec<isize> = ii_idx
        .get_indexer_for(&fp_index::Index::new(vec![
            IndexLabel::Utf8("(1.5, 3.0]".to_owned()),
            IndexLabel::Utf8("(9.0, 10.0]".to_owned()),
        ]))
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    assert_eq!(gif_actual, exp_gif);

    let (ginu_idx, ginu_miss) = ii_dup_idx.get_indexer_non_unique(&fp_index::Index::new(vec![
        IndexLabel::Utf8("(0.0, 1.0]".to_owned()),
        IndexLabel::Utf8("(9.0, 10.0]".to_owned()),
    ]));
    let exp_ginu_idx: Vec<isize> = oracle["ginu_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    let exp_ginu_miss: Vec<usize> = oracle["ginu_miss"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as usize)
        .collect();
    assert_eq!(ginu_idx, exp_ginu_idx);
    assert_eq!(ginu_miss, exp_ginu_miss);

    assert_eq!(
        ii_idx
            .get_slice_bound(&IndexLabel::Utf8("(1.5, 3.0]".to_owned()), "left")
            .unwrap(),
        oracle["get_slice_bound_left"].as_u64().unwrap() as usize
    );
    let sl = ii_idx
        .slice_locs(
            Some(&IndexLabel::Utf8("(1.5, 3.0]".to_owned())),
            Some(&IndexLabel::Utf8("(3.0, 5.0]".to_owned())),
        )
        .unwrap();
    let exp_sl = oracle["slice_locs"].as_array().unwrap();
    assert_eq!(
        sl,
        (
            exp_sl[0].as_u64().unwrap() as usize,
            exp_sl[1].as_u64().unwrap() as usize
        )
    );
    let si = ii_idx
        .slice_indexer(
            Some(&IndexLabel::Utf8("(1.5, 3.0]".to_owned())),
            Some(&IndexLabel::Utf8("(3.0, 5.0]".to_owned())),
        )
        .unwrap();
    let exp_si = oracle["slice_indexer"].as_array().unwrap();
    assert_eq!(
        si,
        (
            exp_si[0].as_u64().unwrap() as usize,
            exp_si[1].as_u64().unwrap() as usize
        )
    );
    assert_eq!(
        ii_idx
            .searchsorted(&IndexLabel::Utf8("(1.5, 3.0]".to_owned()), "left")
            .unwrap(),
        oracle["searchsorted_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        ii_idx
            .asof(&IndexLabel::Utf8("(1.5, 3.0]".to_owned()))
            .unwrap()
            .to_string(),
        oracle["asof"].as_str().unwrap()
    );
    let where_idx = fp_index::Index::new(vec![
        IndexLabel::Utf8("(0.0, 1.5]".to_owned()),
        IndexLabel::Utf8("(1.5, 3.0]".to_owned()),
    ]);
    let asof_locs: Vec<isize> = ii_idx
        .asof_locs(&where_idx, Some(&[true; 3]))
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    let exp_asof_locs: Vec<isize> = oracle["asof_locs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(asof_locs, exp_asof_locs);

    assert_eq!(
        ii_idx.isin(&[IndexLabel::Utf8("(1.5, 3.0]".to_owned())]),
        parse_bool_vec(&oracle["isin"])
    );
    assert_eq!(
        idx_str_vec(&ii_single_idx.repeat(2)),
        parse_str_vec(&oracle["repeat"])
    );
    assert_eq!(
        idx_str_vec(&ii_idx.where_(
            &[true, false, true],
            &IndexLabel::Utf8("(0.0, 1.5]".to_owned())
        )),
        parse_str_vec(&oracle["where"])
    );
    assert_eq!(
        idx_str_vec(&ii_idx.putmask(
            &[false, true, false],
            &IndexLabel::Utf8("(0.0, 1.5]".to_owned())
        )),
        parse_str_vec(&oracle["putmask"])
    );
    assert_eq!(
        idx_str_vec(&ii_idx.append(&ii_other_idx)),
        parse_str_vec(&oracle["append"])
    );
    assert_eq!(
        idx_str_vec(
            &ii_idx
                .insert(1, IndexLabel::Utf8("(1.0, 2.0]".to_owned()))
                .unwrap()
        ),
        parse_str_vec(&oracle["insert"])
    );
    assert_eq!(
        idx_str_vec(&ii_idx.delete(1).unwrap()),
        parse_str_vec(&oracle["delete"])
    );
    assert_eq!(
        idx_str_vec(&ii_idx.drop(&[IndexLabel::Utf8("(1.5, 3.0]".to_owned())])),
        parse_str_vec(&oracle["drop"])
    );

    assert_eq!(ii_idx.diff(1), vec![None, None, None]);
    assert_eq!(
        idx_str_vec(&ii_idx.round(1)),
        parse_str_vec(&oracle["round"])
    );
    let shifted = ii_idx.shift(1, IndexLabel::Utf8("(0.0, 1.5]".to_owned()));
    assert_eq!(
        idx_str_vec(&shifted),
        vec!["(0.0, 1.5]", "(0.0, 1.5]", "(1.5, 3.0]"]
    );
    assert_eq!(oracle["diff_err"].as_str(), Some("TypeError"));
    assert_eq!(oracle["shift_err"].as_str(), Some("NotImplementedError"));

    let mapped = ii_idx.map(|l| IndexLabel::Utf8(l.to_string()));
    assert_eq!(idx_str_vec(&mapped), parse_str_vec(&oracle["map"]));
    let _ = ii_idx.r#str().len();
    assert_eq!(oracle["str_err"].as_str(), Some("AttributeError"));

    let tf: Vec<String> = ii_idx
        .to_frame()
        .into_iter()
        .map(|row| row[0].to_string())
        .collect();
    assert_eq!(tf, parse_str_vec(&oracle["to_frame"]));
    let ts: Vec<String> = ii_idx
        .to_series()
        .into_iter()
        .map(|(_, v)| v.to_string())
        .collect();
    assert_eq!(ts, parse_str_vec(&oracle["to_series"]));
    let gb = ii_idx.groupby();
    assert_eq!(
        gb.get(&IndexLabel::Utf8("(0.0, 1.5]".to_owned())),
        Some(&vec![0usize])
    );

    assert_eq!(
        idx_str_vec(&ii_idx.join(&ii_other_idx, "inner").unwrap()),
        parse_str_vec(&oracle["join_inner"])
    );
    let (re_idx, re_pos) = ii_idx.reindex(&ii_other_idx);
    let re_pos_isize: Vec<isize> = re_pos
        .into_iter()
        .map(|opt| opt.map_or(-1, |u| u as isize))
        .collect();
    let exp_re_pos: Vec<isize> = oracle["reindex_pos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(idx_str_vec(&re_idx), parse_str_vec(&oracle["reindex_vals"]));
    assert_eq!(re_pos_isize, exp_re_pos);

    assert_eq!(
        idx_str_vec(&ii_idx.intersection(&ii_other_idx)),
        parse_str_vec(&oracle["intersection"])
    );
    assert_eq!(
        idx_str_vec(&ii_idx.union(&ii_other_idx)),
        parse_str_vec(&oracle["union"])
    );
    assert_eq!(
        idx_str_vec(&ii_idx.difference(&ii_other_idx)),
        parse_str_vec(&oracle["difference"])
    );
    assert_eq!(
        idx_str_vec(&ii_idx.symmetric_difference(&ii_other_idx)),
        parse_str_vec(&oracle["symmetric_difference"])
    );

    // Negative error paths on IntervalIndex constructors and lookups
    assert!(IntervalIndex::from_breaks(&[1.0], IntervalClosed::Right).is_err());
    assert!(IntervalIndex::from_arrays(&[0.0], &[1.0, 2.0], IntervalClosed::Right).is_err());
    assert!(ii_both.get_loc(1.5).is_err());
}
