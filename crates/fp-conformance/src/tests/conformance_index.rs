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
    assert_eq!(r.name(), Some("my_range"));
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
