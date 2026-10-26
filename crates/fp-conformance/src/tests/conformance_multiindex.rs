//! MultiIndex parity-matrix conformance suite (br-frankenpandas-m8cp).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares existing
//! row-MultiIndex DataFrame behavior with live upstream pandas for edge-case
//! inputs: empty named levels, single tuples, duplicate tuples, mixed level
//! dtypes, partial tuple keys, exact duplicate tuple keys, and level-dropping
//! operations.

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
                "live pandas unavailable; skipping MultiIndex conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_multiindex_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("multiindex oracle") {
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
        "pandas MultiIndex parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

fn empty_named_levels_frame_json() -> serde_json::Value {
    serde_json::json!({
        "index": [],
        "row_multiindex": {
            "tuples": [],
            "names": ["outer", "inner"]
        },
        "column_order": ["value"],
        "columns": {
            "value": []
        }
    })
}

fn mixed_three_level_frame_json() -> serde_json::Value {
    serde_json::json!({
        "index": [
            { "kind": "utf8", "value": "east|apple|2023" },
            { "kind": "utf8", "value": "east|apple|2024" },
            { "kind": "utf8", "value": "east|pear|2023" },
            { "kind": "utf8", "value": "west|apple|2023" },
            { "kind": "utf8", "value": "west|pear|2024" }
        ],
        "row_multiindex": {
            "tuples": [
                [
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "apple" },
                    { "kind": "int64", "value": 2023 }
                ],
                [
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "apple" },
                    { "kind": "int64", "value": 2024 }
                ],
                [
                    { "kind": "utf8", "value": "east" },
                    { "kind": "utf8", "value": "pear" },
                    { "kind": "int64", "value": 2023 }
                ],
                [
                    { "kind": "utf8", "value": "west" },
                    { "kind": "utf8", "value": "apple" },
                    { "kind": "int64", "value": 2023 }
                ],
                [
                    { "kind": "utf8", "value": "west" },
                    { "kind": "utf8", "value": "pear" },
                    { "kind": "int64", "value": 2024 }
                ]
            ],
            "names": ["region", "product", "year"]
        },
        "column_order": ["sales", "cost"],
        "columns": {
            "sales": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 15 },
                { "kind": "int64", "value": 30 },
                { "kind": "int64", "value": 25 }
            ],
            "cost": [
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 7 },
                { "kind": "int64", "value": 6 },
                { "kind": "int64", "value": 12 },
                { "kind": "int64", "value": 9 }
            ]
        }
    })
}

fn duplicate_tuple_frame_json() -> serde_json::Value {
    serde_json::json!({
        "index": [
            { "kind": "utf8", "value": "a|1" },
            { "kind": "utf8", "value": "a|1" },
            { "kind": "utf8", "value": "a|2" },
            { "kind": "utf8", "value": "b|1" }
        ],
        "row_multiindex": {
            "tuples": [
                [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "int64", "value": 1 }
                ],
                [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "int64", "value": 1 }
                ],
                [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "int64", "value": 2 }
                ],
                [
                    { "kind": "utf8", "value": "b" },
                    { "kind": "int64", "value": 1 }
                ]
            ],
            "names": ["letter", "bucket"]
        },
        "column_order": ["value", "label"],
        "columns": {
            "value": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 11 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ],
            "label": [
                { "kind": "utf8", "value": "first" },
                { "kind": "utf8", "value": "second" },
                { "kind": "utf8", "value": "third" },
                { "kind": "utf8", "value": "fourth" }
            ]
        }
    })
}

#[test]
fn conformance_multiindex_identity_empty_named_levels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-ID-001",
        "case_id": "multiindex_identity_empty_named_levels",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": empty_named_levels_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_identity_single_tuple_mixed_dtypes() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-ID-002",
        "case_id": "multiindex_identity_single_tuple_mixed_dtypes",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [{ "kind": "utf8", "value": "north|2024" }],
            "row_multiindex": {
                "tuples": [[
                    { "kind": "utf8", "value": "north" },
                    { "kind": "int64", "value": 2024 }
                ]],
                "names": ["region", "year"]
            },
            "column_order": ["sales", "note"],
            "columns": {
                "sales": [{ "kind": "int64", "value": 42 }],
                "note": [{ "kind": "utf8", "value": "single" }]
            }
        }
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_identity_duplicate_tuples() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-ID-003",
        "case_id": "multiindex_identity_duplicate_tuples",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": duplicate_tuple_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_loc_partial_prefix_mixed_levels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-LOC-001",
        "case_id": "multiindex_loc_partial_prefix_mixed_levels",
        "mode": "strict",
        "operation": "dataframe_loc",
        "oracle_source": "live_legacy_pandas",
        "loc_labels": [{ "kind": "utf8", "value": "east" }],
        "frame": mixed_three_level_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_loc_exact_duplicate_tuple() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-LOC-002",
        "case_id": "multiindex_loc_exact_duplicate_tuple",
        "mode": "strict",
        "operation": "dataframe_loc",
        "oracle_source": "live_legacy_pandas",
        "loc_labels": [
            { "kind": "utf8", "value": "a" },
            { "kind": "int64", "value": 1 }
        ],
        "frame": duplicate_tuple_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_xs_first_level_drops_level() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-XS-001",
        "case_id": "multiindex_xs_first_level_drops_level",
        "mode": "strict",
        "operation": "dataframe_xs",
        "oracle_source": "live_legacy_pandas",
        "xs_key": { "kind": "utf8", "value": "east" },
        "xs_level": 0,
        "frame": mixed_three_level_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_xs_middle_level_reorders_remaining_levels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-XS-002",
        "case_id": "multiindex_xs_middle_level_reorders_remaining_levels",
        "mode": "strict",
        "operation": "dataframe_xs",
        "oracle_source": "live_legacy_pandas",
        "xs_key": { "kind": "utf8", "value": "apple" },
        "xs_level": 1,
        "frame": mixed_three_level_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
}

#[test]
fn conformance_multiindex_reset_index_preserves_level_order() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-MULTIINDEX-RESET-001",
        "case_id": "multiindex_reset_index_preserves_level_order",
        "mode": "strict",
        "operation": "dataframe_reset_index",
        "oracle_source": "live_legacy_pandas",
        "reset_index_drop": false,
        "frame": mixed_three_level_frame_json()
    }))
    .expect("fixture");
    check_multiindex_fixture(fixture);
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
        "pandas MultiIndex oracle script failed (status={}):\nstderr={}\nstdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    Some(serde_json::from_slice(&output.stdout).expect("valid multiindex oracle json"))
}

fn tuples_str_i64(mi: &fp_index::MultiIndex) -> Vec<(String, i64)> {
    mi.to_list()
        .into_iter()
        .map(|t| {
            match (&t[0], &t[1]) {
                (fp_index::IndexLabel::Utf8(s), fp_index::IndexLabel::Int64(v)) => {
                    Some((s.clone(), *v))
                }
                _ => None,
            }
            .expect("expected (Utf8, Int64) MultiIndex tuple")
        })
        .collect()
}

fn parse_tuples_str_i64(val: &serde_json::Value) -> Vec<(String, i64)> {
    val.as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let arr = row.as_array().unwrap();
            (
                arr[0].as_str().unwrap().to_owned(),
                arr[1].as_i64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn live_oracle_multiindex_constructors_metadata_levels_and_duplicates_differential() {
    // @covers: MultiIndex.T
    // @covers: MultiIndex.all
    // @covers: MultiIndex.any
    // @covers: MultiIndex.array
    // @covers: MultiIndex.astype
    // @covers: MultiIndex.codes
    // @covers: MultiIndex.copy
    // @covers: MultiIndex.drop_duplicates
    // @covers: MultiIndex.dropna
    // @covers: MultiIndex.dtype
    // @covers: MultiIndex.dtypes
    // @covers: MultiIndex.duplicated
    // @covers: MultiIndex.empty
    // @covers: MultiIndex.equal_levels
    // @covers: MultiIndex.equals
    // @covers: MultiIndex.factorize
    // @covers: MultiIndex.fillna
    // @covers: MultiIndex.format
    // @covers: MultiIndex.from_arrays
    // @covers: MultiIndex.from_frame
    // @covers: MultiIndex.from_product
    // @covers: MultiIndex.from_tuples
    // @covers: MultiIndex.has_duplicates
    // @covers: MultiIndex.hasnans
    // @covers: MultiIndex.holds_integer
    // @covers: MultiIndex.identical
    // @covers: MultiIndex.infer_objects
    // @covers: MultiIndex.inferred_type
    // @covers: MultiIndex.is_
    // @covers: MultiIndex.is_boolean
    // @covers: MultiIndex.is_categorical
    // @covers: MultiIndex.is_floating
    // @covers: MultiIndex.is_integer
    // @covers: MultiIndex.is_interval
    // @covers: MultiIndex.is_monotonic_decreasing
    // @covers: MultiIndex.is_monotonic_increasing
    // @covers: MultiIndex.is_numeric
    // @covers: MultiIndex.is_object
    // @covers: MultiIndex.is_unique
    // @covers: MultiIndex.isin
    // @covers: MultiIndex.isna
    // @covers: MultiIndex.isnull
    // @covers: MultiIndex.item
    // @covers: MultiIndex.levels
    // @covers: MultiIndex.levshape
    // @covers: MultiIndex.memory_usage
    // @covers: MultiIndex.name
    // @covers: MultiIndex.names
    // @covers: MultiIndex.nbytes
    // @covers: MultiIndex.ndim
    // @covers: MultiIndex.nlevels
    // @covers: MultiIndex.notna
    // @covers: MultiIndex.notnull
    // @covers: MultiIndex.nunique
    // @covers: MultiIndex.putmask
    // @covers: MultiIndex.ravel
    // @covers: MultiIndex.remove_unused_levels
    // @covers: MultiIndex.rename
    // @covers: MultiIndex.set_codes
    // @covers: MultiIndex.set_levels
    // @covers: MultiIndex.set_names
    // @covers: MultiIndex.shape
    // @covers: MultiIndex.size
    // @covers: MultiIndex.to_flat_index
    // @covers: MultiIndex.to_frame
    // @covers: MultiIndex.to_list
    // @covers: MultiIndex.to_numpy
    // @covers: MultiIndex.to_series
    // @covers: MultiIndex.tolist
    // @covers: MultiIndex.transpose
    // @covers: MultiIndex.unique
    // @covers: MultiIndex.value_counts
    // @covers: MultiIndex.values
    // @covers: MultiIndex.view
    // @covers: MultiIndex.where
    use fp_index::{DuplicateKeep, IndexLabel, LabelName, MultiIndex, OrderedF64};
    use fp_types::NullKind;

    let python_code = r#"
import json, numpy as np, pandas as pd

mi = pd.MultiIndex.from_tuples(
    [("a", 1), ("a", 2), ("b", 1), ("b", 2)],
    names=["letter", "number"],
)
mi_arr = pd.MultiIndex.from_arrays(
    [["a", "a", "b", "b"], [1, 2, 1, 2]],
    names=["letter", "number"],
)
mi_prod = pd.MultiIndex.from_product(
    [["a", "b"], [1, 2]],
    names=["letter", "number"],
)
mi_frame = pd.MultiIndex.from_frame(
    pd.DataFrame({"letter": ["a", "a", "b", "b"], "number": [1, 2, 1, 2]})
)
mi_desc = pd.MultiIndex.from_tuples(
    [("b", 2), ("b", 1), ("a", 2), ("a", 1)],
    names=["letter", "number"],
)
mi_dup = pd.MultiIndex.from_tuples(
    [("b", 2), ("a", 1), ("b", 2), ("a", 2)],
    names=["letter", "number"],
)
mi_nan = pd.MultiIndex.from_tuples(
    [("a", 1.0), (np.nan, 2.0), ("a", np.nan)],
    names=["letter", "val"],
)
mi_single = pd.MultiIndex.from_tuples([("only", 7)], names=["letter", "number"])

codes_f, uniques_f = mi_dup.factorize()
vc = mi_dup.value_counts()

res = {
    "tuples": [[str(a), int(b)] for a, b in mi.tolist()],
    "arr_eq": bool(mi.equals(mi_arr)),
    "prod_eq": bool(mi.equals(mi_prod)),
    "frame_eq": bool(mi.equals(mi_frame)),
    "name": mi.name,
    "names": [str(x) for x in mi.names],
    "renamed": [str(x) for x in mi.rename(["k1", "k2"]).names],
    "set_names": [str(x) for x in mi.set_names(["s1", "s2"]).names],
    "nlevels": int(mi.nlevels),
    "ndim": int(mi.ndim),
    "size": int(mi.size),
    "shape": list(mi.shape),
    "empty": bool(mi.empty),
    "dtype": str(mi.dtype),
    "dtypes": [str(d) for d in mi.dtypes.tolist()],
    "inferred_type": str(mi.inferred_type),
    "holds_integer": False,
    "is_object": bool(mi.dtype == object),
    "is_unique": bool(mi.is_unique),
    "has_duplicates": bool(mi.has_duplicates),
    "dup_is_unique": bool(mi_dup.is_unique),
    "dup_has_duplicates": bool(mi_dup.has_duplicates),
    "mono_inc": bool(mi.is_monotonic_increasing),
    "mono_dec": bool(mi.is_monotonic_decreasing),
    "desc_mono_inc": bool(mi_desc.is_monotonic_increasing),
    "desc_mono_dec": bool(mi_desc.is_monotonic_decreasing),
    "dup_mono_inc": bool(mi_dup.is_monotonic_increasing),
    "dup_mono_dec": bool(mi_dup.is_monotonic_decreasing),
    "levels_0": [str(x) for x in mi.levels[0].tolist()],
    "levels_1": [int(x) for x in mi.levels[1].tolist()],
    "codes": [[int(c) for c in row] for row in mi.codes],
    "levshape": list(mi.levshape),
    "set_levels": [[str(a), int(b)] for a, b in mi.set_levels([["x", "y"], [10, 20]]).tolist()],
    "set_codes": [[str(a), int(b)] for a, b in mi.set_codes([[1, 1, 0, 0], [1, 0, 1, 0]]).tolist()],
    "single_item": [str(mi_single.item()[0]), int(mi_single.item()[1])],
    "equals_self": bool(mi.equals(mi.copy())),
    "identical_self": bool(mi.identical(mi.copy())),
    "identical_renamed": bool(mi.identical(mi.rename(["x", "y"]))),
    "equal_levels": bool(mi.equal_levels(mi_desc)),
    "isin": [bool(x) for x in mi.isin([("a", 2), ("b", 1), ("z", 9)])],
    "putmask": [[str(a), int(b)] for a, b in mi.putmask(np.array([False, True, False, True]), pd.MultiIndex.from_tuples([("z", 9)] * 4, names=["letter", "number"])).tolist()],
    "where": [[str(a), int(b)] for a, b in mi.putmask(np.array([False, True, False, True]), pd.MultiIndex.from_tuples([("z", 9)] * 4, names=["letter", "number"])).tolist()],
    "dup_unique": [[str(a), int(b)] for a, b in mi_dup.unique().tolist()],
    "dup_nunique": int(mi_dup.nunique()),
    "dup_first": [bool(x) for x in mi_dup.duplicated(keep="first")],
    "dup_last": [bool(x) for x in mi_dup.duplicated(keep="last")],
    "dup_none": [bool(x) for x in mi_dup.duplicated(keep=False)],
    "drop_dup_first": [[str(a), int(b)] for a, b in mi_dup.drop_duplicates(keep="first").tolist()],
    "drop_dup_last": [[str(a), int(b)] for a, b in mi_dup.drop_duplicates(keep="last").tolist()],
    "drop_dup_none": [[str(a), int(b)] for a, b in mi_dup.drop_duplicates(keep=False).tolist()],
    "factorize_codes": [int(c) for c in codes_f],
    "factorize_uniques": [[str(a), int(b)] for a, b in uniques_f.tolist()],
    "vc": [[[str(k[0]), int(k[1])], int(v)] for k, v in vc.items()],
    "dropna_any": [[str(a), float(b)] for a, b in mi_nan.dropna(how="any").tolist()],
    "to_flat_index": [[str(a), int(b)] for a, b in mi.to_flat_index().tolist()],
    "to_flat_index_dtype": str(mi.to_flat_index().dtype),
}
print(json.dumps(res))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        eprintln!("skipping MultiIndex constructors/metadata live-oracle test");
        return;
    };

    let names = vec![
        Some(LabelName::from("letter")),
        Some(LabelName::from("number")),
    ];
    let mi = MultiIndex::from_tuples(vec![
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)],
    ])
    .unwrap()
    .set_names(names.clone());

    let mi_arr = MultiIndex::from_arrays(vec![
        vec![
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Utf8("b".to_owned()),
            IndexLabel::Utf8("b".to_owned()),
        ],
        vec![
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ],
    ])
    .unwrap()
    .set_names(names.clone());

    let mi_prod = MultiIndex::from_product(vec![
        vec![
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Utf8("b".to_owned()),
        ],
        vec![IndexLabel::Int64(1), IndexLabel::Int64(2)],
    ])
    .unwrap()
    .set_names(names.clone());

    let mi_frame = MultiIndex::from_frame(vec![
        (
            Some("letter".to_owned()),
            vec![
                IndexLabel::Utf8("a".to_owned()),
                IndexLabel::Utf8("a".to_owned()),
                IndexLabel::Utf8("b".to_owned()),
                IndexLabel::Utf8("b".to_owned()),
            ],
        ),
        (
            Some("number".to_owned()),
            vec![
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
                IndexLabel::Int64(1),
                IndexLabel::Int64(2),
            ],
        ),
    ])
    .unwrap();

    let mi_desc = MultiIndex::from_tuples(vec![
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(1)],
    ])
    .unwrap()
    .set_names(names.clone());

    let mi_dup = MultiIndex::from_tuples(vec![
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)],
    ])
    .unwrap()
    .set_names(names.clone());

    let mi_nan = MultiIndex::from_tuples(vec![
        vec![
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Float64(OrderedF64(1.0)),
        ],
        vec![
            IndexLabel::Null(NullKind::NaN),
            IndexLabel::Float64(OrderedF64(2.0)),
        ],
        vec![
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Null(NullKind::NaN),
        ],
    ])
    .unwrap();

    let mi_single = MultiIndex::from_tuples(vec![vec![
        IndexLabel::Utf8("only".to_owned()),
        IndexLabel::Int64(7),
    ]])
    .unwrap();

    let exp_tuples = parse_tuples_str_i64(&oracle["tuples"]);
    assert_eq!(tuples_str_i64(&mi), exp_tuples);
    assert_eq!(mi.equals(&mi_arr), oracle["arr_eq"].as_bool().unwrap());
    assert_eq!(mi.equals(&mi_prod), oracle["prod_eq"].as_bool().unwrap());
    assert_eq!(mi.equals(&mi_frame), oracle["frame_eq"].as_bool().unwrap());
    assert_eq!(mi.name(), None);
    assert!(oracle["name"].is_null());
    assert_eq!(mi.names(), names.as_slice());

    let renamed = mi
        .rename(vec![
            Some(LabelName::from("k1")),
            Some(LabelName::from("k2")),
        ])
        .unwrap();
    assert_eq!(
        renamed
            .names()
            .iter()
            .map(|n| n.as_ref().unwrap().as_str().to_owned())
            .collect::<Vec<_>>(),
        vec!["k1".to_owned(), "k2".to_owned()]
    );
    let set_named = mi.clone().set_names(vec![
        Some(LabelName::from("s1")),
        Some(LabelName::from("s2")),
    ]);
    assert_eq!(
        set_named
            .names()
            .iter()
            .map(|n| n.as_ref().unwrap().as_str().to_owned())
            .collect::<Vec<_>>(),
        vec!["s1".to_owned(), "s2".to_owned()]
    );

    assert_eq!(mi.nlevels(), oracle["nlevels"].as_u64().unwrap() as usize);
    assert_eq!(mi.ndim(), oracle["ndim"].as_u64().unwrap() as usize);
    assert_eq!(mi.size(), oracle["size"].as_u64().unwrap() as usize);
    assert_eq!(mi.shape(), (oracle["shape"][0].as_u64().unwrap() as usize,));
    assert_eq!(mi.empty(), oracle["empty"].as_bool().unwrap());
    assert_eq!(mi.dtype(), oracle["dtype"].as_str().unwrap());
    assert_eq!(
        mi.dtypes(),
        oracle["dtypes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        mi.inferred_type(),
        oracle["inferred_type"].as_str().unwrap()
    );
    assert_eq!(
        mi.holds_integer(),
        oracle["holds_integer"].as_bool().unwrap()
    );
    assert_eq!(mi.is_object(), oracle["is_object"].as_bool().unwrap());
    assert!(!mi.is_boolean());
    assert!(!mi.is_categorical());
    assert!(!mi.is_floating());
    assert!(!mi.is_integer());
    assert!(!mi.is_interval());
    assert!(!mi.is_numeric());
    assert_eq!(mi.is_unique(), oracle["is_unique"].as_bool().unwrap());
    assert_eq!(
        mi.has_duplicates(),
        oracle["has_duplicates"].as_bool().unwrap()
    );
    assert_eq!(
        mi_dup.is_unique(),
        oracle["dup_is_unique"].as_bool().unwrap()
    );
    assert_eq!(
        mi_dup.has_duplicates(),
        oracle["dup_has_duplicates"].as_bool().unwrap()
    );
    assert_eq!(
        mi.is_monotonic_increasing(),
        oracle["mono_inc"].as_bool().unwrap()
    );
    assert_eq!(
        mi.is_monotonic_decreasing(),
        oracle["mono_dec"].as_bool().unwrap()
    );
    assert_eq!(
        mi_desc.is_monotonic_increasing(),
        oracle["desc_mono_inc"].as_bool().unwrap()
    );
    assert_eq!(
        mi_desc.is_monotonic_decreasing(),
        oracle["desc_mono_dec"].as_bool().unwrap()
    );
    assert_eq!(
        mi_dup.is_monotonic_increasing(),
        oracle["dup_mono_inc"].as_bool().unwrap()
    );
    assert_eq!(
        mi_dup.is_monotonic_decreasing(),
        oracle["dup_mono_dec"].as_bool().unwrap()
    );

    let lvls = mi.levels();
    assert_eq!(
        lvls[0].labels(),
        &[
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Utf8("b".to_owned()),
        ]
    );
    assert_eq!(
        lvls[1].labels(),
        &[IndexLabel::Int64(1), IndexLabel::Int64(2)]
    );
    let exp_codes: Vec<Vec<isize>> = oracle["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_i64().unwrap() as isize)
                .collect()
        })
        .collect();
    assert_eq!(mi.codes(), exp_codes);
    assert_eq!(
        mi.levshape(),
        oracle["levshape"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        tuples_str_i64(
            &mi.set_levels(vec![
                vec![
                    IndexLabel::Utf8("x".to_owned()),
                    IndexLabel::Utf8("y".to_owned()),
                ],
                vec![IndexLabel::Int64(10), IndexLabel::Int64(20)],
            ])
            .unwrap()
        ),
        parse_tuples_str_i64(&oracle["set_levels"])
    );
    assert_eq!(
        tuples_str_i64(
            &mi.set_codes(vec![vec![1, 1, 0, 0], vec![1, 0, 1, 0]])
                .unwrap()
        ),
        parse_tuples_str_i64(&oracle["set_codes"])
    );
    assert_eq!(mi.remove_unused_levels(), mi);
    assert_eq!(mi.infer_objects(), mi);
    assert_eq!(mi.astype("object").unwrap(), mi);
    // Negative cases: astype("category") and astype("int64") fail on MultiIndex
    assert!(mi.astype("category").is_err());
    assert!(mi.astype("int64").is_err());

    assert!(mi.memory_usage(true) >= mi.nbytes());
    assert!(mi.nbytes() > 0);
    assert!(mi.is_(&mi));
    assert!(!mi.is_(&mi.copy()));
    assert_eq!(mi.copy(), mi);
    assert_eq!(mi.view(), mi);
    assert_eq!(mi.transpose(), mi);
    assert_eq!(mi.T(), mi);
    assert_eq!(mi.to_list(), mi.tolist());
    assert_eq!(mi.to_numpy(), mi.to_list());
    assert_eq!(mi.values(), mi.to_list());
    assert_eq!(mi.array(), mi.to_list());
    assert_eq!(mi.ravel(), mi.to_list());
    assert_eq!(mi.to_frame(), mi.to_list());
    assert_eq!(mi.to_series().len(), 4);
    assert_eq!(mi.format().len(), 4);
    // 9frxo: MultiIndex::to_flat_index(()) / to_tuple_index() returns object Index of tuples matching pandas
    let flat_tuples = mi.to_flat_index(());
    assert_eq!(flat_tuples, mi.to_tuple_index());
    assert_eq!(
        flat_tuples.dtype(),
        oracle["to_flat_index_dtype"].as_str().unwrap()
    );
    let got_flat_tuples: Vec<(String, i64)> = flat_tuples
        .labels()
        .iter()
        .map(|l| {
            let tup = l.as_tuple().expect("tuple label");
            let s = match &tup[0] {
                IndexLabel::Utf8(s) => s.clone(),
                other => other.to_string(),
            };
            let n = match &tup[1] {
                IndexLabel::Int64(v) => *v,
                other => panic!("expected Int64 in tuple, got {other:?}"),
            };
            (s, n)
        })
        .collect();
    assert_eq!(
        got_flat_tuples,
        parse_tuples_str_i64(&oracle["to_flat_index"])
    );
    // Flat string projection with explicit separator still supported
    assert_eq!(
        mi.to_flat_index("|").labels(),
        &[
            IndexLabel::Utf8("a|1".to_owned()),
            IndexLabel::Utf8("a|2".to_owned()),
            IndexLabel::Utf8("b|1".to_owned()),
            IndexLabel::Utf8("b|2".to_owned()),
        ]
    );

    assert_eq!(
        mi_single.item().unwrap(),
        vec![IndexLabel::Utf8("only".to_owned()), IndexLabel::Int64(7)]
    );
    assert!(mi.item().is_err());

    // Negative cases matching pandas: hasnans, isna, isnull, notna, notnull, all, any raise on MultiIndex
    assert!(mi.hasnans().is_err());
    assert!(mi.isna().is_err());
    assert!(mi.isnull().is_err());
    assert!(mi.notna().is_err());
    assert!(mi.notnull().is_err());
    assert!(mi.all().is_err());
    assert!(mi.any().is_err());

    assert_eq!(
        mi.equals(&mi.copy()),
        oracle["equals_self"].as_bool().unwrap()
    );
    assert_eq!(
        mi.identical(&mi.copy()),
        oracle["identical_self"].as_bool().unwrap()
    );
    assert_eq!(
        mi.identical(&renamed),
        oracle["identical_renamed"].as_bool().unwrap()
    );
    assert_eq!(
        mi.equal_levels(&mi_desc),
        oracle["equal_levels"].as_bool().unwrap()
    );

    let exp_isin: Vec<bool> = oracle["isin"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(
        mi.isin(&[
            vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)],
            vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(1)],
            vec![IndexLabel::Utf8("z".to_owned()), IndexLabel::Int64(9)],
        ]),
        exp_isin
    );
    assert_eq!(
        tuples_str_i64(
            &mi.putmask(
                &[false, true, false, true],
                vec![IndexLabel::Utf8("z".to_owned()), IndexLabel::Int64(9)],
            )
            .unwrap()
        ),
        parse_tuples_str_i64(&oracle["putmask"])
    );
    assert!(
        mi.r#where(
            &[true, false, true, false],
            vec![IndexLabel::Utf8("z".to_owned()), IndexLabel::Int64(9)],
        )
        .is_err()
    );
    assert_eq!(
        tuples_str_i64(
            &mi.where_tuple(
                &[true, false, true, false],
                vec![IndexLabel::Utf8("z".to_owned()), IndexLabel::Int64(9)],
            )
            .unwrap()
        ),
        parse_tuples_str_i64(&oracle["where"])
    );

    assert_eq!(
        tuples_str_i64(&mi_dup.unique()),
        parse_tuples_str_i64(&oracle["dup_unique"])
    );
    assert_eq!(
        mi_dup.nunique(),
        oracle["dup_nunique"].as_u64().unwrap() as usize
    );
    for (keep, key_dup, key_drop) in [
        (DuplicateKeep::First, "dup_first", "drop_dup_first"),
        (DuplicateKeep::Last, "dup_last", "drop_dup_last"),
        (DuplicateKeep::None, "dup_none", "drop_dup_none"),
    ] {
        let exp_dup: Vec<bool> = oracle[key_dup]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        assert_eq!(mi_dup.duplicated(keep), exp_dup);
        assert_eq!(
            tuples_str_i64(&mi_dup.drop_duplicates_keep(keep)),
            parse_tuples_str_i64(&oracle[key_drop])
        );
    }
    assert_eq!(
        tuples_str_i64(&mi_dup.drop_duplicates()),
        parse_tuples_str_i64(&oracle["drop_dup_first"])
    );

    let (f_codes, f_uniques) = mi_dup.factorize();
    let exp_f_codes: Vec<isize> = oracle["factorize_codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap() as isize)
        .collect();
    assert_eq!(f_codes, exp_f_codes);
    assert_eq!(
        tuples_str_i64(&f_uniques),
        parse_tuples_str_i64(&oracle["factorize_uniques"])
    );
    let vc: Vec<((String, i64), usize)> = mi_dup
        .value_counts()
        .into_iter()
        .map(|(t, c)| match (&t[0], &t[1]) {
            (IndexLabel::Utf8(s), IndexLabel::Int64(v)) => ((s.clone(), *v), c),
            other => panic!("unexpected vc tuple: {other:?}"),
        })
        .collect();
    let exp_vc: Vec<((String, i64), usize)> = oracle["vc"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let arr = item.as_array().unwrap();
            let tup = arr[0].as_array().unwrap();
            (
                (
                    tup[0].as_str().unwrap().to_owned(),
                    tup[1].as_i64().unwrap(),
                ),
                arr[1].as_u64().unwrap() as usize,
            )
        })
        .collect();
    assert_eq!(vc, exp_vc);

    let dropped_na = mi_nan.dropna();
    assert_eq!(
        dropped_na.to_list(),
        vec![vec![
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Float64(OrderedF64(1.0)),
        ]]
    );
    assert_eq!(
        dropped_na.len(),
        oracle["dropna_any"].as_array().unwrap().len()
    );
    assert!(
        mi_nan
            .fillna(&IndexLabel::Utf8("MISSING".to_owned()))
            .is_err()
    );
}

#[test]
fn live_oracle_multiindex_indexing_set_ops_sorting_and_mutations_differential() {
    // @covers: MultiIndex.append
    // @covers: MultiIndex.argmax
    // @covers: MultiIndex.argmin
    // @covers: MultiIndex.argsort
    // @covers: MultiIndex.asof
    // @covers: MultiIndex.asof_locs
    // @covers: MultiIndex.delete
    // @covers: MultiIndex.diff
    // @covers: MultiIndex.difference
    // @covers: MultiIndex.drop
    // @covers: MultiIndex.droplevel
    // @covers: MultiIndex.get_indexer
    // @covers: MultiIndex.get_indexer_for
    // @covers: MultiIndex.get_indexer_non_unique
    // @covers: MultiIndex.get_level_values
    // @covers: MultiIndex.get_loc
    // @covers: MultiIndex.get_loc_level
    // @covers: MultiIndex.get_locs
    // @covers: MultiIndex.get_slice_bound
    // @covers: MultiIndex.groupby
    // @covers: MultiIndex.insert
    // @covers: MultiIndex.intersection
    // @covers: MultiIndex.join
    // @covers: MultiIndex.map
    // @covers: MultiIndex.max
    // @covers: MultiIndex.min
    // @covers: MultiIndex.reindex
    // @covers: MultiIndex.reorder_levels
    // @covers: MultiIndex.repeat
    // @covers: MultiIndex.round
    // @covers: MultiIndex.searchsorted
    // @covers: MultiIndex.shift
    // @covers: MultiIndex.slice_indexer
    // @covers: MultiIndex.slice_locs
    // @covers: MultiIndex.sort
    // @covers: MultiIndex.sort_values
    // @covers: MultiIndex.sortlevel
    // @covers: MultiIndex.str
    // @covers: MultiIndex.swaplevel
    // @covers: MultiIndex.symmetric_difference
    // @covers: MultiIndex.take
    // @covers: MultiIndex.truncate
    // @covers: MultiIndex.union
    use fp_index::{IndexLabel, LabelName, MultiIndex, MultiIndexOrIndex};

    let python_code = r#"
import json, numpy as np, pandas as pd

mi = pd.MultiIndex.from_tuples(
    [("a", 1), ("a", 2), ("b", 1), ("b", 2)],
    names=["letter", "number"],
)
mi_other = pd.MultiIndex.from_tuples(
    [("a", 2), ("b", 1), ("c", 1)],
    names=["letter", "number"],
)
mi_unsorted = pd.MultiIndex.from_tuples(
    [("b", 2), ("a", 1), ("b", 1), ("a", 2)],
    names=["letter", "number"],
)
mi_dup = pd.MultiIndex.from_tuples(
    [("b", 2), ("a", 1), ("b", 2), ("a", 2)],
    names=["letter", "number"],
)
target = pd.MultiIndex.from_tuples(
    [("b", 2), ("z", 9), ("a", 1)],
    names=["letter", "number"],
)

sl_mi, sl_ord = mi_unsorted.sortlevel()
nu_indexer, nu_missing = mi_dup.get_indexer_non_unique(target)
re_mi, re_pos = mi.reindex(target)
gb = pd.Series(range(len(mi_dup)), index=mi_dup).groupby(level=[0, 1]).apply(list)
gb_vals = mi_dup.groupby([10, 20, 10, 20])

res = {
    "min": [str(mi_unsorted.min()[0]), int(mi_unsorted.min()[1])],
    "max": [str(mi_unsorted.max()[0]), int(mi_unsorted.max()[1])],
    "argmin": int(mi_unsorted.argmin()),
    "argmax": int(mi_unsorted.argmax()),
    "argsort": [int(x) for x in mi_unsorted.argsort()],
    "sort_values": [[str(a), int(b)] for a, b in mi_unsorted.sort_values().tolist()],
    "sortlevel_mi": [[str(a), int(b)] for a, b in sl_mi.tolist()],
    "sortlevel_ord": [int(x) for x in sl_ord],
    "take": [[str(a), int(b)] for a, b in mi.take([3, 0, 2]).tolist()],
    "append": [[str(a), int(b)] for a, b in mi.append(mi_other).tolist()],
    "insert": [[str(a), int(b)] for a, b in mi.insert(2, ("a", 9)).tolist()],
    "delete": [[str(a), int(b)] for a, b in mi.delete(1).tolist()],
    "drop": [[str(a), int(b)] for a, b in mi.drop([("a", 2), ("b", 1)]).tolist()],
    "repeat": [[str(a), int(b)] for a, b in mi.repeat(2).tolist()],
    "round": [[str(a), int(b)] for a, b in mi.round(2).tolist()],
    "map": [str(x) for x in mi.map(lambda t: f"{t[0]}:{t[1]}")],
    "droplevel_0": [int(x) for x in mi.droplevel(0).tolist()],
    "get_level_values_0": [str(x) for x in mi.get_level_values(0).tolist()],
    "get_level_values_1": [int(x) for x in mi.get_level_values(1).tolist()],
    "swaplevel": [[int(a), str(b)] for a, b in mi.swaplevel(0, 1).tolist()],
    "reorder_levels": [[int(a), str(b)] for a, b in mi.reorder_levels([1, 0]).tolist()],
    "get_loc_exact": int(mi.get_loc(("b", 1))),
    "get_locs_prefix": [int(x) for x in mi.get_locs(["a"])],
    "get_indexer": [int(x) for x in mi.get_indexer(target)],
    "get_indexer_for_dup": [int(x) for x in mi_dup.get_indexer_for(target)],
    "nu_indexer": [int(x) for x in nu_indexer],
    "nu_missing": [int(x) for x in nu_missing],
    "reindex_mi": [[str(a), int(b)] for a, b in re_mi.tolist()],
    "reindex_pos": [int(x) for x in re_pos],
    "sb_left": int(mi.get_slice_bound(("a", 2), "left")),
    "sb_right": int(mi.get_slice_bound(("a", 2), "right")),
    "slice_locs": list(mi.slice_locs(("a", 2), ("b", 1))),
    "slice_indexer": [int(mi.slice_indexer(("a", 2), ("b", 1)).start), int(mi.slice_indexer(("a", 2), ("b", 1)).stop)],
    "truncate": [[str(a), int(b)] for a, b in mi.truncate(before="a", after="a").tolist()],
    "searchsorted_left": [int(x) for x in mi.searchsorted(pd.MultiIndex.from_tuples([("a", 2), ("b", 1)])._values, side="left")],
    "searchsorted_right": [int(x) for x in mi.searchsorted(pd.MultiIndex.from_tuples([("a", 2), ("b", 1)])._values, side="right")],
    "union": [[str(a), int(b)] for a, b in mi.union(mi_other).tolist()],
    "union_unsorted_default": [[str(a), int(b)] for a, b in mi_unsorted.union(mi_other).tolist()],
    "union_unsorted_false": [[str(a), int(b)] for a, b in mi_unsorted.union(mi_other, sort=False).tolist()],
    "intersection": [[str(a), int(b)] for a, b in mi.intersection(mi_other, sort=False).tolist()],
    "difference": [[str(a), int(b)] for a, b in mi.difference(mi_other).tolist()],
    "diff_unsorted_default": [[str(a), int(b)] for a, b in mi_unsorted.difference(mi_other).tolist()],
    "diff_unsorted_false": [[str(a), int(b)] for a, b in mi_unsorted.difference(mi_other, sort=False).tolist()],
    "symdiff": [[str(a), int(b)] for a, b in mi.symmetric_difference(mi_other).tolist()],
    "symdiff_unsorted_default": [[str(a), int(b)] for a, b in mi_unsorted.symmetric_difference(mi_other).tolist()],
    "symdiff_unsorted_false": [[str(a), int(b)] for a, b in mi_unsorted.symmetric_difference(mi_other, sort=False).tolist()],
    "join_inner": [[str(a), int(b)] for a, b in mi.join(mi_other, how="inner").tolist()],
    "join_outer": [[str(a), int(b)] for a, b in mi.join(mi_other, how="outer").tolist()],
    "join_left": [[str(a), int(b)] for a, b in mi.join(mi_other, how="left").tolist()],
    "join_right": [[str(a), int(b)] for a, b in mi.join(mi_other, how="right").tolist()],
    "gb_b2": [int(x) for x in gb[("b", 2)]],
    "gb_a1": [int(x) for x in gb[("a", 1)]],
    "gb_vals_10": [[str(a), int(b)] for a, b in gb_vals[10].tolist()],
    "gb_vals_20": [[str(a), int(b)] for a, b in gb_vals[20].tolist()],
}
print(json.dumps(res, default=int))
"#;
    let Some(oracle) = run_pandas_oracle_eval(python_code) else {
        eprintln!("skipping MultiIndex indexing/set-ops live-oracle test");
        return;
    };

    let names = vec![
        Some(LabelName::from("letter")),
        Some(LabelName::from("number")),
    ];
    let mi = MultiIndex::from_tuples(vec![
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)],
    ])
    .unwrap()
    .set_names(names.clone());

    let mi_other = MultiIndex::from_tuples(vec![
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("c".to_owned()), IndexLabel::Int64(1)],
    ])
    .unwrap()
    .set_names(names.clone());

    let mi_unsorted = MultiIndex::from_tuples(vec![
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)],
    ])
    .unwrap()
    .set_names(names.clone());

    let mi_dup = MultiIndex::from_tuples(vec![
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(1)],
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)],
    ])
    .unwrap()
    .set_names(names.clone());

    let target = MultiIndex::from_tuples(vec![
        vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)],
        vec![IndexLabel::Utf8("z".to_owned()), IndexLabel::Int64(9)],
        vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(1)],
    ])
    .unwrap()
    .set_names(names.clone());

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
        mi_unsorted.min(),
        Some(vec![
            IndexLabel::Utf8(oracle["min"][0].as_str().unwrap().to_owned()),
            IndexLabel::Int64(oracle["min"][1].as_i64().unwrap()),
        ])
    );
    assert_eq!(
        mi_unsorted.max(),
        Some(vec![
            IndexLabel::Utf8(oracle["max"][0].as_str().unwrap().to_owned()),
            IndexLabel::Int64(oracle["max"][1].as_i64().unwrap()),
        ])
    );
    assert_eq!(
        mi_unsorted.argmin(),
        Some(oracle["argmin"].as_u64().unwrap() as usize)
    );
    assert_eq!(
        mi_unsorted.argmax(),
        Some(oracle["argmax"].as_u64().unwrap() as usize)
    );
    assert_eq!(mi_unsorted.argsort(), parse_usize_vec(&oracle["argsort"]));
    assert_eq!(
        tuples_str_i64(&mi_unsorted.sort_values()),
        parse_tuples_str_i64(&oracle["sort_values"])
    );
    assert_eq!(
        tuples_str_i64(&mi_unsorted.sort()),
        parse_tuples_str_i64(&oracle["sort_values"])
    );
    let (sl_mi, sl_ord) = mi_unsorted.sortlevel();
    assert_eq!(
        tuples_str_i64(&sl_mi),
        parse_tuples_str_i64(&oracle["sortlevel_mi"])
    );
    assert_eq!(sl_ord, parse_usize_vec(&oracle["sortlevel_ord"]));

    assert_eq!(
        tuples_str_i64(&mi.take(&[3, 0, 2]).unwrap()),
        parse_tuples_str_i64(&oracle["take"])
    );
    assert_eq!(
        tuples_str_i64(&mi.append(&mi_other).unwrap()),
        parse_tuples_str_i64(&oracle["append"])
    );
    assert_eq!(
        tuples_str_i64(
            &mi.insert(
                2,
                vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(9)]
            )
            .unwrap()
        ),
        parse_tuples_str_i64(&oracle["insert"])
    );
    assert_eq!(
        tuples_str_i64(&mi.delete(1).unwrap()),
        parse_tuples_str_i64(&oracle["delete"])
    );
    assert_eq!(
        tuples_str_i64(
            &mi.drop(&[
                vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)],
                vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(1)],
            ])
            .unwrap()
        ),
        parse_tuples_str_i64(&oracle["drop"])
    );
    assert_eq!(
        tuples_str_i64(&mi.repeat(2)),
        parse_tuples_str_i64(&oracle["repeat"])
    );
    assert_eq!(
        tuples_str_i64(&mi.round(2).unwrap()),
        parse_tuples_str_i64(&oracle["round"])
    );
    assert_eq!(
        mi.map(|t| format!("{}:{}", t[0], t[1])),
        oracle["map"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    );

    match mi.droplevel(0).unwrap() {
        MultiIndexOrIndex::Index(idx) => {
            assert_eq!(
                idx.labels(),
                &[
                    IndexLabel::Int64(1),
                    IndexLabel::Int64(2),
                    IndexLabel::Int64(1),
                    IndexLabel::Int64(2),
                ]
            );
        }
        other => panic!("expected flat Index after dropping 1 of 2 levels, got {other:?}"),
    }
    assert_eq!(
        mi.get_level_values(0).unwrap().labels(),
        &[
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Utf8("a".to_owned()),
            IndexLabel::Utf8("b".to_owned()),
            IndexLabel::Utf8("b".to_owned()),
        ]
    );
    assert_eq!(
        mi.get_level_values(1).unwrap().labels(),
        &[
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ]
    );
    let swapped = mi.swaplevel(0, 1).unwrap();
    let reordered = mi.reorder_levels(&[1, 0]).unwrap();
    assert_eq!(swapped, reordered);
    assert_eq!(
        swapped.to_list()[0],
        vec![IndexLabel::Int64(1), IndexLabel::Utf8("a".to_owned())]
    );

    assert_eq!(
        mi.get_loc(
            &[IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(1)],
            None
        )
        .unwrap(),
        vec![oracle["get_loc_exact"].as_u64().unwrap() as usize]
    );
    assert_eq!(
        mi.get_locs(&[IndexLabel::Utf8("a".to_owned())]).unwrap(),
        parse_usize_vec(&oracle["get_locs_prefix"])
    );
    let (loc_pos, loc_rem) = mi
        .get_loc_level(&[IndexLabel::Utf8("a".to_owned())])
        .unwrap();
    assert_eq!(loc_pos, vec![0, 1]);
    assert!(matches!(loc_rem, Some(MultiIndexOrIndex::Index(_))));

    assert_eq!(
        mi.get_indexer(&target).unwrap(),
        parse_isize_vec(&oracle["get_indexer"])
    );
    // Negative case: get_indexer on duplicate-bearing MultiIndex fails
    assert!(mi_dup.get_indexer(&target).is_err());
    assert_eq!(
        mi_dup.get_indexer_for(&target).unwrap(),
        parse_isize_vec(&oracle["get_indexer_for_dup"])
    );
    let (nu_indexer, nu_missing) = mi_dup.get_indexer_non_unique(&target);
    assert_eq!(nu_indexer, parse_isize_vec(&oracle["nu_indexer"]));
    assert_eq!(nu_missing, parse_usize_vec(&oracle["nu_missing"]));

    let (re_mi, re_pos) = mi.reindex(&target).unwrap();
    assert_eq!(
        tuples_str_i64(&re_mi),
        parse_tuples_str_i64(&oracle["reindex_mi"])
    );
    assert_eq!(re_pos, parse_isize_vec(&oracle["reindex_pos"]));

    let key_a2 = [IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(2)];
    let key_b1 = [IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(1)];
    assert_eq!(
        mi.get_slice_bound(&key_a2, "left").unwrap(),
        oracle["sb_left"].as_u64().unwrap() as usize
    );
    assert_eq!(
        mi.get_slice_bound(&key_a2, "right").unwrap(),
        oracle["sb_right"].as_u64().unwrap() as usize
    );
    assert_eq!(
        mi.slice_locs(Some(&key_a2), Some(&key_b1)).unwrap(),
        (
            oracle["slice_locs"][0].as_u64().unwrap() as usize,
            oracle["slice_locs"][1].as_u64().unwrap() as usize,
        )
    );
    assert_eq!(
        mi.slice_indexer(Some(&key_a2), Some(&key_b1)).unwrap(),
        (
            oracle["slice_indexer"][0].as_u64().unwrap() as usize,
            oracle["slice_indexer"][1].as_u64().unwrap() as usize,
        )
    );
    let key_a = [IndexLabel::Utf8("a".to_owned())];
    assert_eq!(
        tuples_str_i64(&mi.truncate(Some(&key_a), Some(&key_a)).unwrap()),
        parse_tuples_str_i64(&oracle["truncate"])
    );

    let ss_target = MultiIndex::from_tuples(vec![key_a2.to_vec(), key_b1.to_vec()]).unwrap();
    assert_eq!(
        mi.searchsorted(&ss_target, "left").unwrap(),
        parse_usize_vec(&oracle["searchsorted_left"])
    );
    assert_eq!(
        mi.searchsorted(&ss_target, "right").unwrap(),
        parse_usize_vec(&oracle["searchsorted_right"])
    );

    assert_eq!(
        tuples_str_i64(&mi.union(&mi_other).unwrap()),
        parse_tuples_str_i64(&oracle["union"])
    );
    assert_eq!(
        tuples_str_i64(&mi.intersection(&mi_other).unwrap()),
        parse_tuples_str_i64(&oracle["intersection"])
    );
    assert_eq!(
        tuples_str_i64(&mi.difference(&mi_other).unwrap()),
        parse_tuples_str_i64(&oracle["difference"])
    );
    assert_eq!(
        tuples_str_i64(&mi.symmetric_difference(&mi_other).unwrap()),
        parse_tuples_str_i64(&oracle["symdiff"])
    );
    // 9cj5a: MultiIndex union, difference, symmetric_difference default to sort=None (sorted) vs sort=Some(false)
    assert_eq!(
        tuples_str_i64(&mi_unsorted.union(&mi_other).unwrap()),
        parse_tuples_str_i64(&oracle["union_unsorted_default"])
    );
    assert_eq!(
        tuples_str_i64(&mi_unsorted.union_sorted(&mi_other, Some(false)).unwrap()),
        parse_tuples_str_i64(&oracle["union_unsorted_false"])
    );
    assert_eq!(
        tuples_str_i64(&mi_unsorted.difference(&mi_other).unwrap()),
        parse_tuples_str_i64(&oracle["diff_unsorted_default"])
    );
    assert_eq!(
        tuples_str_i64(
            &mi_unsorted
                .difference_sorted(&mi_other, Some(false))
                .unwrap()
        ),
        parse_tuples_str_i64(&oracle["diff_unsorted_false"])
    );
    assert_eq!(
        tuples_str_i64(&mi_unsorted.symmetric_difference(&mi_other).unwrap()),
        parse_tuples_str_i64(&oracle["symdiff_unsorted_default"])
    );
    assert_eq!(
        tuples_str_i64(
            &mi_unsorted
                .symmetric_difference_sorted(&mi_other, Some(false))
                .unwrap()
        ),
        parse_tuples_str_i64(&oracle["symdiff_unsorted_false"])
    );
    assert_eq!(
        tuples_str_i64(&mi.join(&mi_other, "inner").unwrap()),
        parse_tuples_str_i64(&oracle["join_inner"])
    );
    assert_eq!(
        tuples_str_i64(&mi.join(&mi_other, "outer").unwrap()),
        parse_tuples_str_i64(&oracle["join_outer"])
    );
    assert_eq!(
        tuples_str_i64(&mi.join(&mi_other, "left").unwrap()),
        parse_tuples_str_i64(&oracle["join_left"])
    );
    assert_eq!(
        tuples_str_i64(&mi.join(&mi_other, "right").unwrap()),
        parse_tuples_str_i64(&oracle["join_right"])
    );

    let gb = mi_dup.groupby_positions();
    assert_eq!(
        gb[&vec![IndexLabel::Utf8("b".to_owned()), IndexLabel::Int64(2)]],
        parse_usize_vec(&oracle["gb_b2"])
    );
    assert_eq!(
        gb[&vec![IndexLabel::Utf8("a".to_owned()), IndexLabel::Int64(1)]],
        parse_usize_vec(&oracle["gb_a1"])
    );
    // jiuhh: MultiIndex::groupby(&values) groups self tuples by external values array
    let gb_vals = mi_dup
        .groupby(&[
            IndexLabel::Int64(10),
            IndexLabel::Int64(20),
            IndexLabel::Int64(10),
            IndexLabel::Int64(20),
        ])
        .unwrap();
    assert_eq!(
        tuples_str_i64(&gb_vals[&IndexLabel::Int64(10)]),
        parse_tuples_str_i64(&oracle["gb_vals_10"])
    );
    assert_eq!(
        tuples_str_i64(&gb_vals[&IndexLabel::Int64(20)]),
        parse_tuples_str_i64(&oracle["gb_vals_20"])
    );

    // Negative error paths matching pandas MultiIndex exceptions
    assert!(mi.shift(1, None).is_err());
    assert!(mi.diff(1).is_err());
    assert!(mi.r#str().is_err());
    assert!(mi.asof(&key_a2).is_err());
    assert!(mi.asof_locs(&mi_other, None).is_err());
    assert!(mi.droplevel(99).is_err());
    assert!(mi.swaplevel(0, 99).is_err());
    assert!(mi.reorder_levels(&[0, 0]).is_err());
    assert!(mi.delete(99).is_err());
    assert!(mi.insert(99, key_a2.to_vec()).is_err());
    assert!(
        mi.drop(&[vec![
            IndexLabel::Utf8("missing".to_owned()),
            IndexLabel::Int64(99)
        ]])
        .is_err()
    );
}
