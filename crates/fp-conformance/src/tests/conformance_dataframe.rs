//! DataFrame parity-matrix conformance suite (br-frankenpandas-mt0v).
//!
//! Per /testing-conformance-harnesses Pattern 1, each test compares the
//! existing Rust DataFrame operation with live upstream pandas for an
//! edge-case input: empty frames, single-row frames, NaN-heavy columns,
//! duplicate row labels, mixed dtypes, and larger ordered slices.

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

fn live_oracle_available(cfg: &HarnessConfig, fixture: &PacketFixture) -> Result<bool, String> {
    match capture_live_oracle_expected(cfg, fixture) {
        Ok(ResolvedExpected::Frame(_) | ResolvedExpected::Series(_)) => Ok(true),
        Ok(other) => Err(format!(
            "unexpected live oracle payload for {}: {other:?}",
            fixture.case_id
        )),
        Err(HarnessError::OracleUnavailable(message)) => {
            eprintln!(
                "live pandas unavailable; skipping DataFrame conformance test {}: {message}",
                fixture.case_id
            );
            Ok(false)
        }
        Err(err) => Err(format!("oracle error on {}: {err}", fixture.case_id)),
    }
}

fn check_dataframe_fixture(fixture: PacketFixture) {
    let cfg = strict_config();
    if !live_oracle_available(&cfg, &fixture).expect("dataframe oracle") {
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
        "pandas DataFrame parity drift for {}: {:?}",
        report.case_id,
        report.drift_records
    );
}

#[test]
fn conformance_dataframe_identity_empty_columns() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-ID-001",
        "case_id": "dataframe_identity_empty_columns",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [],
            "column_order": ["a", "b"],
            "columns": {
                "a": [],
                "b": []
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_identity_single_row_mixed_dtypes() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-ID-002",
        "case_id": "dataframe_identity_single_row_mixed_dtypes",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [{ "kind": "int64", "value": 0 }],
            "column_order": ["int_col", "float_col", "str_col", "bool_col"],
            "columns": {
                "int_col": [{ "kind": "int64", "value": 42 }],
                "float_col": [{ "kind": "float64", "value": 3.5 }],
                "str_col": [{ "kind": "utf8", "value": "x" }],
                "bool_col": [{ "kind": "bool", "value": true }]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_identity_duplicate_index_labels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-ID-003",
        "case_id": "dataframe_identity_duplicate_index_labels",
        "mode": "strict",
        "operation": "dataframe_identity",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "utf8", "value": "dup" },
                { "kind": "utf8", "value": "dup" },
                { "kind": "utf8", "value": "tail" }
            ],
            "column_order": ["value", "label"],
            "columns": {
                "value": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "label": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "c" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_isna_nan_heavy() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-NULL-001",
        "case_id": "dataframe_isna_nan_heavy",
        "mode": "strict",
        "operation": "dataframe_isna",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 }
            ],
            "column_order": ["a", "b", "c"],
            "columns": {
                "a": [
                    { "kind": "null", "value": "na_n" },
                    { "kind": "float64", "value": 1.0 },
                    { "kind": "null", "value": "null" },
                    { "kind": "float64", "value": 4.0 }
                ],
                "b": [
                    { "kind": "utf8", "value": "x" },
                    { "kind": "null", "value": "null" },
                    { "kind": "utf8", "value": "z" },
                    { "kind": "null", "value": "na_n" }
                ],
                "c": [
                    { "kind": "bool", "value": true },
                    { "kind": "bool", "value": false },
                    { "kind": "null", "value": "null" },
                    { "kind": "bool", "value": true }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_count_mixed_nulls() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-COUNT-001",
        "case_id": "dataframe_count_mixed_nulls",
        "mode": "strict",
        "operation": "dataframe_count",
        "oracle_source": "live_legacy_pandas",
        "frame": {
            "index": [
                { "kind": "int64", "value": 10 },
                { "kind": "int64", "value": 20 },
                { "kind": "int64", "value": 30 }
            ],
            "column_order": ["num", "text", "all_missing"],
            "columns": {
                "num": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "int64", "value": 3 }
                ],
                "text": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "null", "value": "null" }
                ],
                "all_missing": [
                    { "kind": "null", "value": "null" },
                    { "kind": "null", "value": "na_n" },
                    { "kind": "null", "value": "null" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_head_duplicate_index_labels() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-HEAD-001",
        "case_id": "dataframe_head_duplicate_index_labels",
        "mode": "strict",
        "operation": "dataframe_head",
        "oracle_source": "live_legacy_pandas",
        "head_n": 2,
        "frame": {
            "index": [
                { "kind": "utf8", "value": "x" },
                { "kind": "utf8", "value": "x" },
                { "kind": "utf8", "value": "y" }
            ],
            "column_order": ["a", "b"],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 },
                    { "kind": "int64", "value": 3 }
                ],
                "b": [
                    { "kind": "utf8", "value": "first" },
                    { "kind": "utf8", "value": "second" },
                    { "kind": "utf8", "value": "third" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_tail_larger_ordered_slice() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-TAIL-001",
        "case_id": "dataframe_tail_larger_ordered_slice",
        "mode": "strict",
        "operation": "dataframe_tail",
        "oracle_source": "live_legacy_pandas",
        "tail_n": 4,
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 },
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 4 },
                { "kind": "int64", "value": 5 }
            ],
            "column_order": ["value", "bucket"],
            "columns": {
                "value": [
                    { "kind": "int64", "value": 0 },
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 20 },
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 40 },
                    { "kind": "int64", "value": 50 }
                ],
                "bucket": [
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "a" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "b" },
                    { "kind": "utf8", "value": "c" },
                    { "kind": "utf8", "value": "c" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_reindex_columns_with_missing_column() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-REINDEX-COLS-001",
        "case_id": "dataframe_reindex_columns_with_missing_column",
        "mode": "strict",
        "operation": "dataframe_reindex_columns",
        "oracle_source": "live_legacy_pandas",
        "reindex_columns": ["b", "missing", "a"],
        "frame": {
            "index": [
                { "kind": "int64", "value": 0 },
                { "kind": "int64", "value": 1 }
            ],
            "column_order": ["a", "b"],
            "columns": {
                "a": [
                    { "kind": "int64", "value": 1 },
                    { "kind": "int64", "value": 2 }
                ],
                "b": [
                    { "kind": "utf8", "value": "left" },
                    { "kind": "utf8", "value": "right" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn conformance_dataframe_sort_index_unsorted_duplicate_ints() {
    let fixture: PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-CONF-DATAFRAME-SORT-001",
        "case_id": "dataframe_sort_index_unsorted_duplicate_ints",
        "mode": "strict",
        "operation": "dataframe_sort_index",
        "oracle_source": "live_legacy_pandas",
        "sort_ascending": true,
        "frame": {
            "index": [
                { "kind": "int64", "value": 3 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 1 },
                { "kind": "int64", "value": 2 }
            ],
            "column_order": ["value", "tag"],
            "columns": {
                "value": [
                    { "kind": "int64", "value": 30 },
                    { "kind": "int64", "value": 10 },
                    { "kind": "int64", "value": 11 },
                    { "kind": "int64", "value": 20 }
                ],
                "tag": [
                    { "kind": "utf8", "value": "c" },
                    { "kind": "utf8", "value": "a1" },
                    { "kind": "utf8", "value": "a2" },
                    { "kind": "utf8", "value": "b" }
                ]
            }
        }
    }))
    .expect("fixture");
    check_dataframe_fixture(fixture);
}

#[test]
fn dataframe_corr_cov_constant_and_singlerow_edges_match_pandas() {
    // Differential edge cases for the corr/cov matrices (cod is actively
    // rewriting these kernels). Verified vs pandas 2.2.3:
    //   df = {a:[1,2,3], b:[2,4,6], c:[5,5,5]}
    //   corr -> a/b perfectly correlated (1.0); EVERY pairing with the constant
    //           column c (INCLUDING corr(c,c)) is NaN (zero variance), NOT 1.0.
    //   cov  -> cov(a,a)=1, cov(a,b)=2, cov(b,b)=4; every cov with c is 0.0.
    //   single-row frame -> corr is all NaN (n=1).
    use fp_frame::DataFrame;
    use fp_types::Scalar;

    let df = DataFrame::from_dict(
        &["a", "b", "c"],
        vec![
            (
                "a",
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                ],
            ),
            (
                "b",
                vec![
                    Scalar::Float64(2.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(6.0),
                ],
            ),
            (
                "c",
                vec![
                    Scalar::Float64(5.0),
                    Scalar::Float64(5.0),
                    Scalar::Float64(5.0),
                ],
            ),
        ],
    )
    .expect("frame");

    let corr = df.corr().expect("corr");
    let cval = |col: &str, i: usize| corr.column(col).unwrap().values()[i].to_f64();
    // a/b block: perfect correlation.
    assert!((cval("a", 0).unwrap() - 1.0).abs() < 1e-12);
    assert!((cval("b", 0).unwrap() - 1.0).abs() < 1e-12);
    assert!((cval("a", 1).unwrap() - 1.0).abs() < 1e-12);
    // constant column c: every correlation is NaN, INCLUDING the diagonal.
    assert!(
        corr.column("c").unwrap().values()[0].is_missing(),
        "corr(a,c) must be NaN"
    );
    assert!(
        corr.column("c").unwrap().values()[1].is_missing(),
        "corr(b,c) must be NaN"
    );
    assert!(
        corr.column("c").unwrap().values()[2].is_missing(),
        "corr(c,c) must be NaN for a zero-variance column (pandas), not 1.0"
    );
    assert!(
        corr.column("a").unwrap().values()[2].is_missing(),
        "corr(c,a) must be NaN"
    );

    let cov = df.cov().expect("cov");
    let kov = |col: &str, i: usize| cov.column(col).unwrap().values()[i].to_f64().unwrap();
    assert!((kov("a", 0) - 1.0).abs() < 1e-12); // var(a)
    assert!((kov("b", 0) - 2.0).abs() < 1e-12); // cov(a,b)
    assert!((kov("b", 1) - 4.0).abs() < 1e-12); // var(b)
    // constant column: cov is 0.0 (NOT NaN).
    assert!((kov("c", 0) - 0.0).abs() < 1e-12, "cov(a,c) must be 0.0");
    assert!((kov("c", 2) - 0.0).abs() < 1e-12, "cov(c,c) must be 0.0");

    // single-row frame: corr is all NaN.
    let one = DataFrame::from_dict(
        &["a", "b"],
        vec![
            ("a", vec![Scalar::Float64(1.0)]),
            ("b", vec![Scalar::Float64(2.0)]),
        ],
    )
    .expect("one-row frame");
    let corr1 = one.corr().expect("corr1");
    assert!(
        corr1.column("a").unwrap().values()[0].is_missing(),
        "single-row corr must be NaN"
    );
    assert!(
        corr1.column("b").unwrap().values()[0].is_missing(),
        "single-row corr must be NaN"
    );
}

#[test]
fn dataframe_corr_cov_pairwise_nan_deletion_matches_pandas() {
    // pandas corr/cov use PAIRWISE complete-observation deletion, NOT listwise:
    // the off-diagonal cov(a,b) drops rows where EITHER is NaN, but the diagonal
    // cov(a,a) uses a's OWN non-NaN rows. A gram-matrix perf rewrite that shares
    // one listwise mask across all cells would diverge. Verified vs pandas 2.2.3
    // for df = {a:[1,2,NaN,4], b:[2,NaN,6,8]}:
    //   cov(a,a)=2.333333 (var of [1,2,4]), NOT 4.5 (var of listwise [1,4]);
    //   cov(b,b)=9.333333 (var of [2,6,8]); cov(a,b)=9.0 (pairwise rows 0,3);
    //   corr is 1.0 everywhere (the pairwise-complete pairs are collinear).
    use fp_frame::DataFrame;
    use fp_types::{NullKind, Scalar};

    let n = |_: ()| Scalar::Null(NullKind::NaN);
    let df = DataFrame::from_dict(
        &["a", "b"],
        vec![
            (
                "a",
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    n(()),
                    Scalar::Float64(4.0),
                ],
            ),
            (
                "b",
                vec![
                    Scalar::Float64(2.0),
                    n(()),
                    Scalar::Float64(6.0),
                    Scalar::Float64(8.0),
                ],
            ),
        ],
    )
    .expect("frame");

    let cov = df.cov().expect("cov");
    let kov = |col: &str, i: usize| cov.column(col).unwrap().values()[i].to_f64().unwrap();
    assert!(
        (kov("a", 0) - 2.333_333_333_333_333).abs() < 1e-9,
        "cov(a,a) must use a's own non-NaN rows (var[1,2,4]=2.333), got {}",
        kov("a", 0)
    );
    assert!(
        (kov("b", 1) - 9.333_333_333_333_334).abs() < 1e-9,
        "cov(b,b) must be var[2,6,8]=9.333, got {}",
        kov("b", 1)
    );
    assert!(
        (kov("b", 0) - 9.0).abs() < 1e-9,
        "cov(a,b) must use pairwise rows 0,3 -> 9.0, got {}",
        kov("b", 0)
    );

    let corr = df.corr().expect("corr");
    let cval = |col: &str, i: usize| corr.column(col).unwrap().values()[i].to_f64().unwrap();
    assert!((cval("a", 0) - 1.0).abs() < 1e-9);
    assert!(
        (cval("b", 0) - 1.0).abs() < 1e-9,
        "pairwise corr(a,b) must be 1.0"
    );
}

#[test]
fn dataframe_corr_spearman_kendall_ties_and_nan_match_pandas() {
    // Rank-correlation fast paths (avg-rank precompute) must match pandas 2.2.3
    // under (a) tied ranks and (b) pairwise-NaN deletion. Verified vs pandas:
    //   df {a:[1,2,2,NaN,5], b:[3,1,1,4,2]}: spearman a-b=-1/3, kendall a-b=-0.2
    //   df {x:[1,1,1,2,2], y:[5,5,3,3,3]}:   spearman x-y=kendall x-y=-2/3
    use fp_frame::DataFrame;
    use fp_types::{NullKind, Scalar};
    let f = Scalar::Float64;
    let nan = Scalar::Null(NullKind::NaN);

    let df = DataFrame::from_dict(
        &["a", "b"],
        vec![
            ("a", vec![f(1.0), f(2.0), f(2.0), nan.clone(), f(5.0)]),
            ("b", vec![f(3.0), f(1.0), f(1.0), f(4.0), f(2.0)]),
        ],
    )
    .expect("frame");
    let off = |m: &str, df: &DataFrame| {
        df.corr_method_with_numeric_only(m, false)
            .expect("corr")
            .column("b")
            .unwrap()
            .values()[0]
            .to_f64()
            .unwrap()
    };
    assert!(
        (off("spearman", &df) - (-1.0 / 3.0)).abs() < 1e-9,
        "spearman ties+NaN: got {}",
        off("spearman", &df)
    );
    assert!(
        (off("kendall", &df) - (-0.2)).abs() < 1e-9,
        "kendall ties+NaN: got {}",
        off("kendall", &df)
    );

    let df2 = DataFrame::from_dict(
        &["x", "y"],
        vec![
            ("x", vec![f(1.0), f(1.0), f(1.0), f(2.0), f(2.0)]),
            ("y", vec![f(5.0), f(5.0), f(3.0), f(3.0), f(3.0)]),
        ],
    )
    .expect("frame2");
    let off2 = |m: &str| {
        df2.corr_method_with_numeric_only(m, false)
            .expect("corr2")
            .column("y")
            .unwrap()
            .values()[0]
            .to_f64()
            .unwrap()
    };
    assert!(
        (off2("spearman") - (-2.0 / 3.0)).abs() < 1e-9,
        "spearman heavy ties: got {}",
        off2("spearman")
    );
    assert!(
        (off2("kendall") - (-2.0 / 3.0)).abs() < 1e-9,
        "kendall heavy ties: got {}",
        off2("kendall")
    );
}

#[test]
fn series_rank_all_methods_na_options_pct_match_pandas() {
    // Differential guard vs pandas 2.2.3 for Series.rank across every tie-break
    // method, na_option, pct scaling, and descending order. Input
    // s = [3, 1, 1, NaN, 2, 1] (a 3-way tie at value 1, one NaN).
    use fp_columnar::Column;
    use fp_frame::Series;
    use fp_index::{Index, IndexLabel};
    use fp_types::{NullKind, Scalar};

    let labels: Vec<IndexLabel> = (0..6).map(IndexLabel::Int64).collect();
    let vals = vec![
        Scalar::Float64(3.0),
        Scalar::Float64(1.0),
        Scalar::Float64(1.0),
        Scalar::Null(NullKind::NaN),
        Scalar::Float64(2.0),
        Scalar::Float64(1.0),
    ];
    let s =
        Series::new("s", Index::new(labels), Column::from_values(vals).unwrap()).expect("series");

    let got = |r: &Series| -> Vec<Option<f64>> {
        r.column()
            .values()
            .iter()
            .map(|v| v.to_f64().ok().filter(|x| !x.is_nan()))
            .collect()
    };
    let close = |a: &[Option<f64>], b: &[Option<f64>]| {
        a.len() == b.len()
            && a.iter().zip(b).all(|(x, y)| match (x, y) {
                (Some(p), Some(q)) => (p - q).abs() < 1e-9,
                (None, None) => true,
                _ => false,
            })
    };
    let n = None;
    let f = Some;

    type RankCase = (&'static str, bool, &'static str, bool, Vec<Option<f64>>);
    let cases: &[RankCase] = &[
        (
            "average",
            true,
            "keep",
            false,
            vec![f(5.0), f(2.0), f(2.0), n, f(4.0), f(2.0)],
        ),
        (
            "min",
            true,
            "keep",
            false,
            vec![f(5.0), f(1.0), f(1.0), n, f(4.0), f(1.0)],
        ),
        (
            "max",
            true,
            "keep",
            false,
            vec![f(5.0), f(3.0), f(3.0), n, f(4.0), f(3.0)],
        ),
        (
            "first",
            true,
            "keep",
            false,
            vec![f(5.0), f(1.0), f(2.0), n, f(4.0), f(3.0)],
        ),
        (
            "dense",
            true,
            "keep",
            false,
            vec![f(3.0), f(1.0), f(1.0), n, f(2.0), f(1.0)],
        ),
        (
            "min",
            true,
            "bottom",
            false,
            vec![f(5.0), f(1.0), f(1.0), f(6.0), f(4.0), f(1.0)],
        ),
        (
            "min",
            true,
            "top",
            false,
            vec![f(6.0), f(2.0), f(2.0), f(1.0), f(5.0), f(2.0)],
        ),
        (
            "average",
            true,
            "keep",
            true,
            vec![f(1.0), f(0.4), f(0.4), n, f(0.8), f(0.4)],
        ),
        (
            "min",
            false,
            "keep",
            false,
            vec![f(1.0), f(3.0), f(3.0), n, f(2.0), f(3.0)],
        ),
    ];
    for (method, asc, na, pct, want) in cases {
        let r = s.rank_with_pct(method, *asc, na, *pct).expect("rank");
        let g = got(&r);
        assert!(
            close(&g, want),
            "rank(method={method}, asc={asc}, na={na}, pct={pct}) => {g:?}, want {want:?}"
        );
    }
}

#[test]
fn series_quantile_all_interpolations_with_nan_match_pandas() {
    // Differential guard vs pandas 2.2.3 for Series.quantile across all five
    // interpolation modes with a trailing NaN (which must be dropped before
    // interpolating). s = [1, 2, 3, 4, NaN]; q in {0.1,0.25,0.5,0.75,0.9}.
    use fp_columnar::Column;
    use fp_frame::Series;
    use fp_index::{Index, IndexLabel};
    use fp_types::{NullKind, Scalar};

    let labels: Vec<IndexLabel> = (0..5).map(IndexLabel::Int64).collect();
    let vals = vec![
        Scalar::Float64(1.0),
        Scalar::Float64(2.0),
        Scalar::Float64(3.0),
        Scalar::Float64(4.0),
        Scalar::Null(NullKind::NaN),
    ];
    let s =
        Series::new("s", Index::new(labels), Column::from_values(vals).unwrap()).expect("series");

    let qs = [0.1, 0.25, 0.5, 0.75, 0.9];
    let expect: &[(&str, [f64; 5])] = &[
        ("linear", [1.3, 1.75, 2.5, 3.25, 3.7]),
        ("lower", [1.0, 1.0, 2.0, 3.0, 3.0]),
        ("higher", [2.0, 2.0, 3.0, 4.0, 4.0]),
        ("nearest", [1.0, 2.0, 3.0, 3.0, 4.0]),
        ("midpoint", [1.5, 1.5, 2.5, 3.5, 3.5]),
    ];
    for (interp, want) in expect {
        for (q, w) in qs.iter().zip(want) {
            let got = s
                .quantile_with_interpolation(*q, interp)
                .expect("quantile")
                .to_f64()
                .unwrap();
            assert!(
                (got - w).abs() < 1e-9,
                "quantile(q={q}, interp={interp}) => {got}, want {w}"
            );
        }
    }
}

#[test]
fn series_mode_and_nlargest_nsmallest_keep_match_pandas() {
    // Differential guard vs pandas 2.2.3 for tie-sensitive selection ops.
    use fp_columnar::Column;
    use fp_frame::Series;
    use fp_index::{Index, IndexLabel};
    use fp_types::{NullKind, Scalar};

    let mk = |labels: Vec<i64>, vals: Vec<Scalar>| {
        Series::new(
            "s",
            Index::new(labels.into_iter().map(IndexLabel::Int64).collect()),
            Column::from_values(vals).unwrap(),
        )
        .expect("series")
    };
    let f = Scalar::Float64;
    let vals_of = |r: &Series| -> Vec<f64> {
        r.column()
            .values()
            .iter()
            .filter_map(|v| v.to_f64().ok())
            .collect()
    };
    let labels_of = |r: &Series| -> Vec<i64> {
        r.index()
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Int64(v) => *v,
                _ => panic!("non-int label"), // ubs:ignore — test assertion
            })
            .collect()
    };

    // mode(): multimodal -> ascending-sorted modal values [1, 2].
    let m = mk(
        (0..6).collect(),
        vec![
            f(2.0),
            f(2.0),
            f(1.0),
            f(1.0),
            f(3.0),
            Scalar::Null(NullKind::NaN),
        ],
    );
    assert_eq!(vals_of(&m.mode().expect("mode")), vec![1.0, 2.0]);

    // nlargest/nsmallest keep semantics. t = [5,3,5,1,3,5] at labels 0..5.
    let t = mk(
        (0..6).collect(),
        vec![f(5.0), f(3.0), f(5.0), f(1.0), f(3.0), f(5.0)],
    );
    // keep='first': ties resolved by original position -> labels 0,2,5.
    assert_eq!(
        labels_of(&t.nlargest_keep(3, "first").expect("nl-first")),
        vec![0, 2, 5]
    );
    // keep='last': ties resolved reverse -> labels 5,2,0.
    assert_eq!(
        labels_of(&t.nlargest_keep(3, "last").expect("nl-last")),
        vec![5, 2, 0]
    );
    // nsmallest keep='first': 1 (label 3) then first 3 (label 1).
    assert_eq!(
        labels_of(&t.nsmallest_keep(2, "first").expect("ns-first")),
        vec![3, 1]
    );
}

#[test]
fn series_interpolate_linear_boundary_asymmetry_matches_pandas() {
    // pandas default Series.interpolate(method='linear') has an asymmetric
    // boundary rule: a LEADING NaN gap stays NaN (no backward fill), an
    // INTERIOR gap is linearly interpolated, and a TRAILING gap is
    // forward-filled with the last valid value (limit_direction='forward',
    // NOT extrapolated). Verified vs pandas 2.2.3 for
    // s = [NaN, 1, NaN, NaN, 4, NaN] => [NaN, 1, 2, 3, 4, 4].
    use fp_columnar::Column;
    use fp_frame::Series;
    use fp_index::{Index, IndexLabel};
    use fp_types::{NullKind, Scalar};

    let nan = || Scalar::Null(NullKind::NaN);
    let s = Series::new(
        "s",
        Index::new((0..6).map(IndexLabel::Int64).collect()),
        Column::from_values(vec![
            nan(),
            Scalar::Float64(1.0),
            nan(),
            nan(),
            Scalar::Float64(4.0),
            nan(),
        ])
        .unwrap(),
    )
    .expect("series");

    let got: Vec<Option<f64>> = s
        .interpolate()
        .expect("interpolate")
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().ok().filter(|x| !x.is_nan()))
        .collect();
    assert_eq!(
        got,
        vec![None, Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(4.0)],
        "interpolate boundary asymmetry diverged"
    );
}

#[test]
fn dataframe_binary_alias_packets_run_green_offline_7p5r3() -> Result<(), Box<dyn std::error::Error>>
{
    // br-frankenpandas-7p5r3: banked oracle fixtures pinning all 25 DataFrame binary aliases.
    // Generated by live pandas 2.2.3 with fixture_provenance; replayed offline.
    let cfg = super::HarnessConfig::default_paths();
    let report = super::run_packet_by_id(&cfg, "FP-P2D-482", super::OracleMode::FixtureExpected)
        .map_err(|err| format!("FP-P2D-482: {err}"))?;
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-482"));
    assert_eq!(
        report.fixture_count, 25,
        "expected 25 dataframe_binary_alias fixtures"
    );
    assert!(
        report.is_green(),
        "FP-P2D-482: expected all fixtures green, got {report:?}"
    );
    Ok(())
}

#[test]
fn dataframe_apply_alias_packets_run_green_offline_7p5r3() -> Result<(), Box<dyn std::error::Error>>
{
    // br-frankenpandas-7p5r3: banked oracle fixtures pinning DataFrame apply aliases
    // (sem_axis0, nunique_axis0, prod_axis1, product_axis1).
    // Generated by live pandas 2.2.3 with fixture_provenance; replayed offline.
    let cfg = super::HarnessConfig::default_paths();
    let report = super::run_packet_by_id(&cfg, "FP-P2D-483", super::OracleMode::FixtureExpected)
        .map_err(|err| format!("FP-P2D-483: {err}"))?;
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2D-483"));
    assert_eq!(
        report.fixture_count, 8,
        "expected 8 dataframe_apply_* fixtures"
    );
    assert!(
        report.is_green(),
        "FP-P2D-483: expected all fixtures green, got {report:?}"
    );
    Ok(())
}

#[test]
fn conformance_dataframe_add_prefix_suffix_differential() {
    use fp_frame::DataFrame;
    use fp_index::IndexLabel;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({"val1": [1.0, 2.0], "val2": [3.0, 4.0]}, index=["r1", "r2"])
p_col = df.add_prefix("col_")
p_idx = df.add_prefix("idx_", axis=0)
s_col = df.add_suffix("_end")
s_idx = df.add_suffix("_row", axis=0)
res = {
    "p_cols": p_col.columns.tolist(),
    "p_col_idx": p_col.index.tolist(),
    "p_idx_cols": p_idx.columns.tolist(),
    "p_idx_idx": p_idx.index.tolist(),
    "s_cols": s_col.columns.tolist(),
    "s_col_idx": s_col.index.tolist(),
    "s_idx_cols": s_idx.columns.tolist(),
    "s_idx_idx": s_idx.index.tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping DataFrame add_prefix/suffix differential test"
            );
            return;
        }
    };

    let df = DataFrame::from_dict_with_index(
        vec![
            ("val1", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
            ("val2", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
        ],
        vec![IndexLabel::Utf8("r1".into()), IndexLabel::Utf8("r2".into())],
    )
    .expect("df");

    let p_col = df.add_prefix("col_").expect("add_prefix");
    let p_idx = df.add_prefix_axis("idx_", 0).expect("add_prefix_axis");
    let s_col = df.add_suffix("_end").expect("add_suffix");
    let s_idx = df.add_suffix_axis("_row", 0).expect("add_suffix_axis");

    let p_cols_oracle: Vec<String> = oracle["p_cols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let p_cols_actual: Vec<String> = p_col.column_names().into_iter().cloned().collect();
    assert_eq!(p_cols_actual, p_cols_oracle);

    let p_idx_oracle: Vec<String> = oracle["p_idx_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let p_idx_labels: Vec<String> = p_idx
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    assert_eq!(p_idx_labels, p_idx_oracle);

    let s_cols_oracle: Vec<String> = oracle["s_cols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let s_cols_actual: Vec<String> = s_col.column_names().into_iter().cloned().collect();
    assert_eq!(s_cols_actual, s_cols_oracle);

    let s_idx_oracle: Vec<String> = oracle["s_idx_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let s_idx_labels: Vec<String> = s_idx
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    assert_eq!(s_idx_labels, s_idx_oracle);
}

#[test]
fn conformance_dataframe_squeeze_differential() {
    use fp_frame::DataFrame;
    use fp_index::IndexLabel;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df_1x1 = pd.DataFrame({"a": [42.0]})
df_1x2 = pd.DataFrame({"a": [1.0], "b": [2.0]}, index=["row0"])
df_2x1 = pd.DataFrame({"a": [10.0, 20.0]}, index=["r1", "r2"])

s_1x2_ax0 = df_1x2.squeeze(axis=0)
s_2x1_ax1 = df_2x1.squeeze(axis=1)

res = {
    "df_1x1_val": float(df_1x1.squeeze()),
    "s_1x2_name": str(s_1x2_ax0.name),
    "s_1x2_index": s_1x2_ax0.index.tolist(),
    "s_1x2_values": s_1x2_ax0.tolist(),
    "s_2x1_name": str(s_2x1_ax1.name),
    "s_2x1_index": s_2x1_ax1.index.tolist(),
    "s_2x1_values": s_2x1_ax1.tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping DataFrame squeeze differential test");
            return;
        }
    };

    let df_1x1 =
        DataFrame::from_dict(&["a"], vec![("a", vec![Scalar::Float64(42.0)])]).expect("df_1x1");
    let s_1x1 = df_1x1.squeeze(1).expect("squeeze 1x1");
    assert_eq!(
        s_1x1.column().values()[0].to_f64().unwrap(),
        oracle["df_1x1_val"].as_f64().unwrap()
    );

    let df_1x2 = DataFrame::from_dict_with_index(
        vec![
            ("a", vec![Scalar::Float64(1.0)]),
            ("b", vec![Scalar::Float64(2.0)]),
        ],
        vec![IndexLabel::Utf8("row0".into())],
    )
    .expect("df_1x2");
    let s_1x2 = df_1x2.squeeze(0).expect("squeeze axis 0");
    assert_eq!(s_1x2.name(), oracle["s_1x2_name"].as_str().unwrap());
    let s_1x2_idx: Vec<String> = s_1x2
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let s_1x2_idx_oracle: Vec<String> = oracle["s_1x2_index"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(s_1x2_idx, s_1x2_idx_oracle);
    let s_1x2_vals: Vec<f64> = s_1x2
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let s_1x2_vals_oracle: Vec<f64> = oracle["s_1x2_values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(s_1x2_vals, s_1x2_vals_oracle);

    let df_2x1 = DataFrame::from_dict_with_index(
        vec![("a", vec![Scalar::Float64(10.0), Scalar::Float64(20.0)])],
        vec![IndexLabel::Utf8("r1".into()), IndexLabel::Utf8("r2".into())],
    )
    .expect("df_2x1");
    let s_2x1 = df_2x1.squeeze(1).expect("squeeze axis 1");
    assert_eq!(s_2x1.name(), oracle["s_2x1_name"].as_str().unwrap());
    let s_2x1_idx: Vec<String> = s_2x1
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let s_2x1_idx_oracle: Vec<String> = oracle["s_2x1_index"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(s_2x1_idx, s_2x1_idx_oracle);
    let s_2x1_vals: Vec<f64> = s_2x1
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let s_2x1_vals_oracle: Vec<f64> = oracle["s_2x1_values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(s_2x1_vals, s_2x1_vals_oracle);

    let df_2x2 = DataFrame::from_dict(
        &["a", "b"],
        vec![
            ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
            ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
        ],
    )
    .expect("df_2x2");
    assert!(df_2x2.squeeze(0).is_err());
    assert!(df_2x2.squeeze(1).is_err());
}

#[test]
fn conformance_dataframe_truncate_differential() {
    use fp_frame::DataFrame;
    use fp_index::IndexLabel;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({"x": [1.0, 2.0, 3.0, 4.0, 5.0], "y": [10.0, 20.0, 30.0, 40.0, 50.0]}, index=["a", "b", "c", "d", "e"])
t1 = df.truncate(before="b", after="d")
t2 = df.truncate(after="c")
t3 = df.truncate(before="c")
t4 = df.truncate(before="z")
res = {
    "t1_idx": t1.index.tolist(),
    "t1_x": t1["x"].tolist(),
    "t2_idx": t2.index.tolist(),
    "t2_x": t2["x"].tolist(),
    "t3_idx": t3.index.tolist(),
    "t3_x": t3["x"].tolist(),
    "t4_len": len(t4),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping DataFrame truncate differential test");
            return;
        }
    };

    let df = DataFrame::from_dict_with_index(
        vec![
            (
                "x",
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                    Scalar::Float64(4.0),
                    Scalar::Float64(5.0),
                ],
            ),
            (
                "y",
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(30.0),
                    Scalar::Float64(40.0),
                    Scalar::Float64(50.0),
                ],
            ),
        ],
        vec![
            IndexLabel::Utf8("a".into()),
            IndexLabel::Utf8("b".into()),
            IndexLabel::Utf8("c".into()),
            IndexLabel::Utf8("d".into()),
            IndexLabel::Utf8("e".into()),
        ],
    )
    .expect("df");

    let b_label = IndexLabel::Utf8("b".into());
    let c_label = IndexLabel::Utf8("c".into());
    let d_label = IndexLabel::Utf8("d".into());
    let z_label = IndexLabel::Utf8("z".into());

    let t1 = df
        .truncate(Some(&b_label), Some(&d_label))
        .expect("truncate t1");
    let t1_idx: Vec<String> = t1.index().labels().iter().map(|l| l.to_string()).collect();
    let t1_idx_oracle: Vec<String> = oracle["t1_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(t1_idx, t1_idx_oracle);
    let t1_x: Vec<f64> = t1
        .column("x")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let t1_x_oracle: Vec<f64> = oracle["t1_x"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(t1_x, t1_x_oracle);

    let t2 = df.truncate(None, Some(&c_label)).expect("truncate t2");
    let t2_idx: Vec<String> = t2.index().labels().iter().map(|l| l.to_string()).collect();
    let t2_idx_oracle: Vec<String> = oracle["t2_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(t2_idx, t2_idx_oracle);

    let t3 = df.truncate(Some(&c_label), None).expect("truncate t3");
    let t3_idx: Vec<String> = t3.index().labels().iter().map(|l| l.to_string()).collect();
    let t3_idx_oracle: Vec<String> = oracle["t3_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(t3_idx, t3_idx_oracle);

    let t4 = df.truncate(Some(&z_label), None).expect("truncate t4");
    assert_eq!(t4.len(), oracle["t4_len"].as_u64().unwrap() as usize);
}

#[test]
fn conformance_dataframe_select_dtypes_differential() {
    use fp_frame::DataFrame;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({"int_c": [1, 2], "float_c": [1.5, 2.5], "utf8_c": ["x", "y"], "bool_c": [True, False]})
res = {
    "num": df.select_dtypes(include="number").columns.tolist(),
    "int": df.select_dtypes(include="int").columns.tolist(),
    "float": df.select_dtypes(include="float").columns.tolist(),
    "no_obj": df.select_dtypes(exclude="object").columns.tolist(),
    "bool_int": df.select_dtypes(include=["bool", "int"]).columns.tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping DataFrame select_dtypes differential test"
            );
            return;
        }
    };

    let df = DataFrame::from_dict(
        &["int_c", "float_c", "utf8_c", "bool_c"],
        vec![
            ("int_c", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ("float_c", vec![Scalar::Float64(1.5), Scalar::Float64(2.5)]),
            (
                "utf8_c",
                vec![Scalar::Utf8("x".into()), Scalar::Utf8("y".into())],
            ),
            ("bool_c", vec![Scalar::Bool(true), Scalar::Bool(false)]),
        ],
    )
    .expect("df");

    let num_df = df.select_dtypes_by_name(&["number"], &[]).expect("num");
    let num_cols_oracle: Vec<String> = oracle["num"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let num_cols_actual: Vec<String> = num_df.column_names().into_iter().cloned().collect();
    assert_eq!(num_cols_actual, num_cols_oracle);

    let int_df = df.select_dtypes_by_name(&["int"], &[]).expect("int");
    let int_cols_oracle: Vec<String> = oracle["int"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let int_cols_actual: Vec<String> = int_df.column_names().into_iter().cloned().collect();
    assert_eq!(int_cols_actual, int_cols_oracle);

    let float_df = df.select_dtypes_by_name(&["float"], &[]).expect("float");
    let float_cols_oracle: Vec<String> = oracle["float"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let float_cols_actual: Vec<String> = float_df.column_names().into_iter().cloned().collect();
    assert_eq!(float_cols_actual, float_cols_oracle);

    let no_obj_df = df.select_dtypes_by_name(&[], &["object"]).expect("no_obj");
    let no_obj_cols_oracle: Vec<String> = oracle["no_obj"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let no_obj_cols_actual: Vec<String> = no_obj_df.column_names().into_iter().cloned().collect();
    assert_eq!(no_obj_cols_actual, no_obj_cols_oracle);

    let bool_int_df = df
        .select_dtypes_by_name(&["bool", "int"], &[])
        .expect("bool_int");
    let bool_int_cols_oracle: Vec<String> = oracle["bool_int"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let bool_int_cols_actual: Vec<String> =
        bool_int_df.column_names().into_iter().cloned().collect();
    assert_eq!(bool_int_cols_actual, bool_int_cols_oracle);
}

#[test]
fn conformance_dataframe_update_differential() {
    use fp_frame::DataFrame;
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, pandas as pd
df1 = pd.DataFrame({"a": [1.0, 2.0, 3.0], "b": [10.0, 20.0, 30.0]})
df2 = pd.DataFrame({"a": [float("nan"), 99.0, float("nan")], "b": [100.0, float("nan"), 300.0]})
df1.update(df2)
res = {
    "a": df1["a"].tolist(),
    "b": df1["b"].tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping DataFrame update differential test");
            return;
        }
    };

    let df1 = DataFrame::from_dict(
        &["a", "b"],
        vec![
            (
                "a",
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Float64(2.0),
                    Scalar::Float64(3.0),
                ],
            ),
            (
                "b",
                vec![
                    Scalar::Float64(10.0),
                    Scalar::Float64(20.0),
                    Scalar::Float64(30.0),
                ],
            ),
        ],
    )
    .expect("df1");

    let df2 = DataFrame::from_dict(
        &["a", "b"],
        vec![
            (
                "a",
                vec![
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(99.0),
                    Scalar::Null(NullKind::NaN),
                ],
            ),
            (
                "b",
                vec![
                    Scalar::Float64(100.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(300.0),
                ],
            ),
        ],
    )
    .expect("df2");

    let updated = df1.update(&df2).expect("update");
    let a_vals: Vec<f64> = updated
        .column("a")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let a_oracle: Vec<f64> = oracle["a"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(a_vals, a_oracle);

    let b_vals: Vec<f64> = updated
        .column("b")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let b_oracle: Vec<f64> = oracle["b"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(b_vals, b_oracle);
}

#[test]
fn conformance_dataframe_reindex_like_differential() {
    use fp_frame::DataFrame;
    use fp_index::IndexLabel;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df1 = pd.DataFrame({"val": [10.0, 20.0, 30.0]}, index=[0, 1, 2])
other = pd.DataFrame({"marker": [1, 2, 3]}, index=[2, 0, 5])
reindexed = df1.reindex(other.index)
res = {
    "idx": [int(x) for x in reindexed.index],
    "val": [None if pd.isna(x) else float(x) for x in reindexed["val"]],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping DataFrame reindex_like differential test"
            );
            return;
        }
    };

    let df1 = DataFrame::from_dict_with_index(
        vec![(
            "val",
            vec![
                Scalar::Float64(10.0),
                Scalar::Float64(20.0),
                Scalar::Float64(30.0),
            ],
        )],
        vec![
            IndexLabel::Int64(0),
            IndexLabel::Int64(1),
            IndexLabel::Int64(2),
        ],
    )
    .expect("df1");

    let other = DataFrame::from_dict_with_index(
        vec![(
            "marker",
            vec![Scalar::Int64(1), Scalar::Int64(2), Scalar::Int64(3)],
        )],
        vec![
            IndexLabel::Int64(2),
            IndexLabel::Int64(0),
            IndexLabel::Int64(5),
        ],
    )
    .expect("other");

    let reindexed = df1.reindex_like(&other).expect("reindex_like");
    let idx_vals: Vec<i64> = reindexed
        .index()
        .labels()
        .iter()
        .filter_map(|l| match l {
            IndexLabel::Int64(i) => Some(*i),
            _ => None,
        })
        .collect();
    let idx_oracle: Vec<i64> = oracle["idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(idx_vals, idx_oracle);

    assert_eq!(
        reindexed.column("val").unwrap().values()[0],
        Scalar::Float64(30.0)
    );
    assert_eq!(
        reindexed.column("val").unwrap().values()[1],
        Scalar::Float64(10.0)
    );
    assert!(reindexed.column("val").unwrap().values()[2].is_missing());
}

#[test]
fn conformance_dataframe_pop_and_equals_differential() {
    use fp_frame::DataFrame;
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({"col1": [1.0, 2.0], "col2": [3.0, 4.0]})
df_copy = df.copy()
popped = df.pop("col1")
df_with_nan1 = pd.DataFrame({"a": [1.0, float("nan")]})
df_with_nan2 = pd.DataFrame({"a": [1.0, float("nan")]})
df_diff = pd.DataFrame({"a": [1.0, 99.0]})

res = {
    "popped_name": str(popped.name),
    "popped_vals": popped.tolist(),
    "remaining_cols": df.columns.tolist(),
    "equals_self": df_copy.equals(df_copy),
    "equals_nan": df_with_nan1.equals(df_with_nan2),
    "equals_diff": df_with_nan1.equals(df_diff),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping DataFrame pop and equals differential test"
            );
            return;
        }
    };

    let df = DataFrame::from_dict(
        &["col1", "col2"],
        vec![
            ("col1", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
            ("col2", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
        ],
    )
    .expect("df");

    let df_copy = df.copy();
    assert_eq!(
        df.equals(&df_copy),
        oracle["equals_self"].as_bool().unwrap()
    );

    let (popped, remaining) = df.pop("col1").expect("pop");
    assert_eq!(popped.name(), oracle["popped_name"].as_str().unwrap());
    let popped_vals: Vec<f64> = popped
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let popped_oracle: Vec<f64> = oracle["popped_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(popped_vals, popped_oracle);
    let remaining_cols: Vec<String> = remaining.column_names().into_iter().cloned().collect();
    assert_eq!(remaining_cols, vec!["col2".to_string()]);

    let df_with_nan1 = DataFrame::from_dict(
        &["a"],
        vec![("a", vec![Scalar::Float64(1.0), Scalar::Null(NullKind::NaN)])],
    )
    .expect("df_nan1");
    let df_with_nan2 = DataFrame::from_dict(
        &["a"],
        vec![("a", vec![Scalar::Float64(1.0), Scalar::Null(NullKind::NaN)])],
    )
    .expect("df_nan2");
    let df_diff = DataFrame::from_dict(
        &["a"],
        vec![("a", vec![Scalar::Float64(1.0), Scalar::Float64(99.0)])],
    )
    .expect("df_diff");

    assert_eq!(
        df_with_nan1.equals(&df_with_nan2),
        oracle["equals_nan"].as_bool().unwrap()
    );
    assert_eq!(
        df_with_nan1.equals(&df_diff),
        oracle["equals_diff"].as_bool().unwrap()
    );
}

#[test]
fn conformance_dataframe_ffill_bfill_differential() {
    use fp_frame::DataFrame;
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({
    "a": [1.0, float("nan"), float("nan"), 4.0],
    "b": [float("nan"), 10.0, float("nan"), 40.0],
})
ffill_df = df.ffill()
ffill_lim1 = df.ffill(limit=1)
bfill_df = df.bfill()
bfill_lim1 = df.bfill(limit=1)
res = {
    "ffill_a": [None if pd.isna(x) else float(x) for x in ffill_df["a"]],
    "ffill_b": [None if pd.isna(x) else float(x) for x in ffill_df["b"]],
    "ffill_lim1_a": [None if pd.isna(x) else float(x) for x in ffill_lim1["a"]],
    "bfill_a": [None if pd.isna(x) else float(x) for x in bfill_df["a"]],
    "bfill_b": [None if pd.isna(x) else float(x) for x in bfill_df["b"]],
    "bfill_lim1_a": [None if pd.isna(x) else float(x) for x in bfill_lim1["a"]],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping DataFrame ffill/bfill differential test"
            );
            return;
        }
    };

    let df = DataFrame::from_dict(
        &["a", "b"],
        vec![
            (
                "a",
                vec![
                    Scalar::Float64(1.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(4.0),
                ],
            ),
            (
                "b",
                vec![
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(10.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(40.0),
                ],
            ),
        ],
    )
    .expect("df");

    let ffill_df = df.ffill(None).expect("ffill");
    let ffill_a: Vec<Option<f64>> = ffill_df
        .column("a")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let ffill_a_oracle: Vec<Option<f64>> = oracle["ffill_a"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(ffill_a, ffill_a_oracle);

    let ffill_b: Vec<Option<f64>> = ffill_df
        .column("b")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let ffill_b_oracle: Vec<Option<f64>> = oracle["ffill_b"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(ffill_b, ffill_b_oracle);

    let ffill_lim1 = df.ffill(Some(1)).expect("ffill_lim1");
    let ffill_lim1_a: Vec<Option<f64>> = ffill_lim1
        .column("a")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let ffill_lim1_a_oracle: Vec<Option<f64>> = oracle["ffill_lim1_a"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(ffill_lim1_a, ffill_lim1_a_oracle);

    let bfill_df = df.bfill(None).expect("bfill");
    let bfill_a: Vec<Option<f64>> = bfill_df
        .column("a")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let bfill_a_oracle: Vec<Option<f64>> = oracle["bfill_a"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(bfill_a, bfill_a_oracle);

    let bfill_b: Vec<Option<f64>> = bfill_df
        .column("b")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let bfill_b_oracle: Vec<Option<f64>> = oracle["bfill_b"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(bfill_b, bfill_b_oracle);

    let bfill_lim1 = df.bfill(Some(1)).expect("bfill_lim1");
    let bfill_lim1_a: Vec<Option<f64>> = bfill_lim1
        .column("a")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let bfill_lim1_a_oracle: Vec<Option<f64>> = oracle["bfill_lim1_a"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(bfill_lim1_a, bfill_lim1_a_oracle);
}

#[test]
fn conformance_dataframe_isin_differential() {
    use fp_frame::DataFrame;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({
    "num": [1, 2, 3, 4],
    "txt": ["a", "b", "c", "d"],
})
res_list = df.isin([2, 4, "b"])
res = {
    "num": res_list["num"].tolist(),
    "txt": res_list["txt"].tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping DataFrame isin differential test");
            return;
        }
    };

    let df = DataFrame::from_dict(
        &["num", "txt"],
        vec![
            (
                "num",
                vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(3),
                    Scalar::Int64(4),
                ],
            ),
            (
                "txt",
                vec![
                    Scalar::Utf8("a".into()),
                    Scalar::Utf8("b".into()),
                    Scalar::Utf8("c".into()),
                    Scalar::Utf8("d".into()),
                ],
            ),
        ],
    )
    .expect("df");

    let isin_df = df
        .isin(&[Scalar::Int64(2), Scalar::Int64(4), Scalar::Utf8("b".into())])
        .expect("isin");

    let num_actual: Vec<bool> = isin_df
        .column("num")
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Bool(b) => *b,
            _ => false,
        })
        .collect();
    let num_oracle: Vec<bool> = oracle["num"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(num_actual, num_oracle);

    let txt_actual: Vec<bool> = isin_df
        .column("txt")
        .unwrap()
        .values()
        .iter()
        .map(|v| match v {
            Scalar::Bool(b) => *b,
            _ => false,
        })
        .collect();
    let txt_oracle: Vec<bool> = oracle["txt"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    assert_eq!(txt_actual, txt_oracle);
}

#[test]
fn conformance_dataframe_first_last_valid_index_differential() {
    use fp_frame::DataFrame;
    use fp_index::IndexLabel;
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({
    "a": [float("nan"), float("nan"), 3.0, float("nan")],
    "b": [float("nan"), 20.0, float("nan"), float("nan")],
}, index=["r0", "r1", "r2", "r3"])
res = {
    "first_valid": df.first_valid_index(),
    "last_valid": df.last_valid_index(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping DataFrame first/last valid index differential test"
            );
            return;
        }
    };

    let df = DataFrame::from_dict_with_index(
        vec![
            (
                "a",
                vec![
                    Scalar::Null(NullKind::NaN),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(3.0),
                    Scalar::Null(NullKind::NaN),
                ],
            ),
            (
                "b",
                vec![
                    Scalar::Null(NullKind::NaN),
                    Scalar::Float64(20.0),
                    Scalar::Null(NullKind::NaN),
                    Scalar::Null(NullKind::NaN),
                ],
            ),
        ],
        vec![
            IndexLabel::Utf8("r0".into()),
            IndexLabel::Utf8("r1".into()),
            IndexLabel::Utf8("r2".into()),
            IndexLabel::Utf8("r3".into()),
        ],
    )
    .expect("df");

    let first_valid = df.first_valid_index().map(|l| l.to_string());
    assert_eq!(first_valid.as_deref(), oracle["first_valid"].as_str());

    let last_valid = df.last_valid_index().map(|l| l.to_string());
    assert_eq!(last_valid.as_deref(), oracle["last_valid"].as_str());
}

#[test]
fn conformance_dataframe_dot_differential() {
    use fp_frame::DataFrame;
    use fp_index::IndexLabel;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df1 = pd.DataFrame({"a": [1.0, 2.0], "b": [3.0, 4.0]})
df2 = pd.DataFrame({"x": [5.0, 6.0], "y": [7.0, 8.0]}, index=["a", "b"])
dot_res = df1.dot(df2)
res = {
    "cols": dot_res.columns.tolist(),
    "x": dot_res["x"].tolist(),
    "y": dot_res["y"].tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping DataFrame dot differential test");
            return;
        }
    };

    let df1 = DataFrame::from_dict(
        &["a", "b"],
        vec![
            ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
            ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
        ],
    )
    .expect("df1");

    let df2 = DataFrame::from_dict_with_index(
        vec![
            ("x", vec![Scalar::Float64(5.0), Scalar::Float64(6.0)]),
            ("y", vec![Scalar::Float64(7.0), Scalar::Float64(8.0)]),
        ],
        vec![IndexLabel::Utf8("a".into()), IndexLabel::Utf8("b".into())],
    )
    .expect("df2");

    let res = df1.dot(&df2).expect("dot");
    let actual_cols: Vec<String> = res.column_names().into_iter().cloned().collect();
    let oracle_cols: Vec<String> = oracle["cols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_cols, oracle_cols);

    let x_vals: Vec<f64> = res
        .column("x")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let x_oracle: Vec<f64> = oracle["x"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(x_vals, x_oracle);

    let y_vals: Vec<f64> = res
        .column("y")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let y_oracle: Vec<f64> = oracle["y"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(y_vals, y_oracle);
}

#[test]
fn conformance_dataframe_align_differential() {
    use fp_frame::DataFrame;
    use fp_index::{AlignMode, IndexLabel};
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df1 = pd.DataFrame({"a": [1.0, 2.0]}, index=[1, 2])
df2 = pd.DataFrame({"a": [20.0, 30.0]}, index=[2, 3])
a1, a2 = df1.align(df2, join="outer")
res = {
    "idx": [int(x) for x in a1.index],
    "a1": [None if pd.isna(x) else float(x) for x in a1["a"]],
    "a2": [None if pd.isna(x) else float(x) for x in a2["a"]],
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping DataFrame align differential test");
            return;
        }
    };

    let df1 = DataFrame::from_dict_with_index(
        vec![("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)])],
        vec![IndexLabel::Int64(1), IndexLabel::Int64(2)],
    )
    .expect("df1");

    let df2 = DataFrame::from_dict_with_index(
        vec![("a", vec![Scalar::Float64(20.0), Scalar::Float64(30.0)])],
        vec![IndexLabel::Int64(2), IndexLabel::Int64(3)],
    )
    .expect("df2");

    let (a1, a2) = df1.align(&df2, AlignMode::Outer).expect("align");

    let idx_vals: Vec<i64> = a1
        .index()
        .labels()
        .iter()
        .filter_map(|l| match l {
            IndexLabel::Int64(i) => Some(*i),
            _ => None,
        })
        .collect();
    let idx_oracle: Vec<i64> = oracle["idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(idx_vals, idx_oracle);

    let a1_vals: Vec<Option<f64>> = a1
        .column("a")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let a1_oracle: Vec<Option<f64>> = oracle["a1"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(a1_vals, a1_oracle);

    let a2_vals: Vec<Option<f64>> = a2
        .column("a")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().ok())
        .collect();
    let a2_oracle: Vec<Option<f64>> = oracle["a2"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64())
        .collect();
    assert_eq!(a2_vals, a2_oracle);
}

#[test]
fn conformance_dataframe_interpolate_differential() {
    use fp_frame::DataFrame;
    use fp_types::{NullKind, Scalar};

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({"val": [1.0, float("nan"), float("nan"), 4.0]})
interp = df.interpolate()
res = {
    "val": interp["val"].tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping DataFrame interpolate differential test"
            );
            return;
        }
    };

    let df = DataFrame::from_dict(
        &["val"],
        vec![(
            "val",
            vec![
                Scalar::Float64(1.0),
                Scalar::Null(NullKind::NaN),
                Scalar::Null(NullKind::NaN),
                Scalar::Float64(4.0),
            ],
        )],
    )
    .expect("df");

    let interp = df.interpolate().expect("interpolate");
    let actual_vals: Vec<f64> = interp
        .column("val")
        .unwrap()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let oracle_vals: Vec<f64> = oracle["val"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_vals, oracle_vals);
}

#[test]
fn conformance_dataframe_set_axis_and_rename_axis_differential() {
    use fp_frame::DataFrame;
    use fp_index::IndexLabel;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({"a": [1.0, 2.0], "b": [3.0, 4.0]}, index=["r0", "r1"])
renamed_rows = df.set_axis(["row_x", "row_y"], axis=0)
renamed_cols = df.set_axis(["col_u", "col_v"], axis=1)
renamed_ax = df.rename_axis("sample_id")
res = {
    "rows_idx": [str(x) for x in renamed_rows.index],
    "cols_names": renamed_cols.columns.tolist(),
    "ax_name": renamed_ax.index.name,
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping DataFrame set_axis/rename_axis differential test"
            );
            return;
        }
    };

    let df = DataFrame::from_dict_with_index(
        vec![
            ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
            ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
        ],
        vec![IndexLabel::Utf8("r0".into()), IndexLabel::Utf8("r1".into())],
    )
    .expect("df");

    let renamed_rows = df
        .set_axis(
            vec![
                IndexLabel::Utf8("row_x".into()),
                IndexLabel::Utf8("row_y".into()),
            ],
            0,
        )
        .expect("set_axis 0");
    let actual_rows_idx: Vec<String> = renamed_rows
        .index()
        .labels()
        .iter()
        .map(|l| l.to_string())
        .collect();
    let oracle_rows_idx: Vec<String> = oracle["rows_idx"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_rows_idx, oracle_rows_idx);

    let renamed_cols = df
        .set_axis(
            vec![
                IndexLabel::Utf8("col_u".into()),
                IndexLabel::Utf8("col_v".into()),
            ],
            1,
        )
        .expect("set_axis 1");
    let actual_cols: Vec<String> = renamed_cols.column_names().into_iter().cloned().collect();
    let oracle_cols: Vec<String> = oracle["cols_names"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(actual_cols, oracle_cols);

    let renamed_ax = df.rename_axis("sample_id").expect("rename_axis");
    assert_eq!(renamed_ax.index().name(), oracle["ax_name"].as_str());
}

#[test]
fn conformance_dataframe_get_differential() {
    use fp_frame::DataFrame;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({"a": [1.0, 2.0], "b": [3.0, 4.0]})
got_a = df.get("a")
got_missing = df.get("missing", default="fallback")
res = {
    "a_vals": got_a.tolist(),
    "missing": str(got_missing),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!("pandas oracle unavailable; skipping DataFrame get differential test");
            return;
        }
    };

    let df = DataFrame::from_dict(
        &["a", "b"],
        vec![
            ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0)]),
            ("b", vec![Scalar::Float64(3.0), Scalar::Float64(4.0)]),
        ],
    )
    .expect("df");

    let got_a = df.get("a").expect("get a").expect("some");
    let actual_a_vals: Vec<f64> = got_a
        .column()
        .values()
        .iter()
        .map(|v| v.to_f64().unwrap())
        .collect();
    let oracle_a_vals: Vec<f64> = oracle["a_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    assert_eq!(actual_a_vals, oracle_a_vals);

    let got_missing = df.get("missing").expect("get missing");
    assert!(got_missing.is_none());
}

#[test]
fn conformance_dataframe_value_counts_differential() {
    use fp_frame::DataFrame;
    use fp_types::Scalar;

    let python_code = r#"
import json, pandas as pd
df = pd.DataFrame({"a": [1, 2, 1, 1], "b": [10, 20, 10, 10]})
vc_all = df.value_counts()
vc_sub = df.value_counts(subset=["a"])
res = {
    "all_vals": vc_all.tolist(),
    "sub_vals": vc_sub.tolist(),
}
print(json.dumps(res))
"#;

    let oracle = match run_pandas_oracle_eval(python_code) {
        Some(val) => val,
        None => {
            eprintln!(
                "pandas oracle unavailable; skipping DataFrame value_counts differential test"
            );
            return;
        }
    };

    let df = DataFrame::from_dict(
        &["a", "b"],
        vec![
            (
                "a",
                vec![
                    Scalar::Int64(1),
                    Scalar::Int64(2),
                    Scalar::Int64(1),
                    Scalar::Int64(1),
                ],
            ),
            (
                "b",
                vec![
                    Scalar::Int64(10),
                    Scalar::Int64(20),
                    Scalar::Int64(10),
                    Scalar::Int64(10),
                ],
            ),
        ],
    )
    .expect("df");

    let vc_all = df.value_counts().expect("value_counts");
    let actual_all_vals: Vec<i64> = vc_all
        .column()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap_or(0))
        .collect();
    let oracle_all_vals: Vec<i64> = oracle["all_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_all_vals, oracle_all_vals);

    let vc_sub = df.value_counts_subset(&["a"]).expect("value_counts_subset");
    let actual_sub_vals: Vec<i64> = vc_sub
        .column()
        .values()
        .iter()
        .map(|v| v.to_i64().unwrap_or(0))
        .collect();
    let oracle_sub_vals: Vec<i64> = oracle["sub_vals"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_i64().unwrap())
        .collect();
    assert_eq!(actual_sub_vals, oracle_sub_vals);
}
