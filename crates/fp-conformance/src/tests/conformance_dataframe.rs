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
    HarnessConfig::default_paths()
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
            ("a", vec![Scalar::Float64(1.0), Scalar::Float64(2.0), Scalar::Float64(3.0)]),
            ("b", vec![Scalar::Float64(2.0), Scalar::Float64(4.0), Scalar::Float64(6.0)]),
            ("c", vec![Scalar::Float64(5.0), Scalar::Float64(5.0), Scalar::Float64(5.0)]),
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
    assert!(corr.column("c").unwrap().values()[0].is_missing(), "corr(a,c) must be NaN");
    assert!(corr.column("c").unwrap().values()[1].is_missing(), "corr(b,c) must be NaN");
    assert!(
        corr.column("c").unwrap().values()[2].is_missing(),
        "corr(c,c) must be NaN for a zero-variance column (pandas), not 1.0"
    );
    assert!(corr.column("a").unwrap().values()[2].is_missing(), "corr(c,a) must be NaN");

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
        vec![("a", vec![Scalar::Float64(1.0)]), ("b", vec![Scalar::Float64(2.0)])],
    )
    .expect("one-row frame");
    let corr1 = one.corr().expect("corr1");
    assert!(corr1.column("a").unwrap().values()[0].is_missing(), "single-row corr must be NaN");
    assert!(corr1.column("b").unwrap().values()[0].is_missing(), "single-row corr must be NaN");
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
                vec![Scalar::Float64(1.0), Scalar::Float64(2.0), n(()), Scalar::Float64(4.0)],
            ),
            (
                "b",
                vec![Scalar::Float64(2.0), n(()), Scalar::Float64(6.0), Scalar::Float64(8.0)],
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
    assert!((cval("b", 0) - 1.0).abs() < 1e-9, "pairwise corr(a,b) must be 1.0");
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
    assert!((off2("spearman") - (-2.0 / 3.0)).abs() < 1e-9, "spearman heavy ties: got {}", off2("spearman"));
    assert!((off2("kendall") - (-2.0 / 3.0)).abs() < 1e-9, "kendall heavy ties: got {}", off2("kendall"));
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
    let s = Series::new("s", Index::new(labels), Column::from_values(vals).unwrap())
        .expect("series");

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

    let cases: &[(&str, bool, &str, bool, Vec<Option<f64>>)] = &[
        ("average", true, "keep", false, vec![f(5.0), f(2.0), f(2.0), n, f(4.0), f(2.0)]),
        ("min", true, "keep", false, vec![f(5.0), f(1.0), f(1.0), n, f(4.0), f(1.0)]),
        ("max", true, "keep", false, vec![f(5.0), f(3.0), f(3.0), n, f(4.0), f(3.0)]),
        ("first", true, "keep", false, vec![f(5.0), f(1.0), f(2.0), n, f(4.0), f(3.0)]),
        ("dense", true, "keep", false, vec![f(3.0), f(1.0), f(1.0), n, f(2.0), f(1.0)]),
        ("min", true, "bottom", false, vec![f(5.0), f(1.0), f(1.0), f(6.0), f(4.0), f(1.0)]),
        ("min", true, "top", false, vec![f(6.0), f(2.0), f(2.0), f(1.0), f(5.0), f(2.0)]),
        ("average", true, "keep", true, vec![f(1.0), f(0.4), f(0.4), n, f(0.8), f(0.4)]),
        ("min", false, "keep", false, vec![f(1.0), f(3.0), f(3.0), n, f(2.0), f(3.0)]),
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
    let s = Series::new("s", Index::new(labels), Column::from_values(vals).unwrap())
        .expect("series");

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
        r.column().values().iter().filter_map(|v| v.to_f64().ok()).collect()
    };
    let labels_of = |r: &Series| -> Vec<i64> {
        r.index()
            .labels()
            .iter()
            .map(|l| match l {
                IndexLabel::Int64(v) => *v,
                _ => panic!("non-int label"),
            })
            .collect()
    };

    // mode(): multimodal -> ascending-sorted modal values [1, 2].
    let m = mk(
        (0..6).collect(),
        vec![f(2.0), f(2.0), f(1.0), f(1.0), f(3.0), Scalar::Null(NullKind::NaN)],
    );
    assert_eq!(vals_of(&m.mode().expect("mode")), vec![1.0, 2.0]);

    // nlargest/nsmallest keep semantics. t = [5,3,5,1,3,5] at labels 0..5.
    let t = mk((0..6).collect(), vec![f(5.0), f(3.0), f(5.0), f(1.0), f(3.0), f(5.0)]);
    // keep='first': ties resolved by original position -> labels 0,2,5.
    assert_eq!(labels_of(&t.nlargest_keep(3, "first").expect("nl-first")), vec![0, 2, 5]);
    // keep='last': ties resolved reverse -> labels 5,2,0.
    assert_eq!(labels_of(&t.nlargest_keep(3, "last").expect("nl-last")), vec![5, 2, 0]);
    // nsmallest keep='first': 1 (label 3) then first 3 (label 1).
    assert_eq!(labels_of(&t.nsmallest_keep(2, "first").expect("ns-first")), vec![3, 1]);
}
