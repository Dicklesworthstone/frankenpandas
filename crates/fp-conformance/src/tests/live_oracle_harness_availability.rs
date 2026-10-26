#[test]
fn live_oracle_mode_executes_or_returns_structured_failure() {
    let cfg = super::HarnessConfig::default_paths();
    let options = super::SuiteOptions {
        packet_filter: Some("FP-P2C-001".to_owned()),
        oracle_mode: super::OracleMode::LiveLegacyPandas,
    };
    let result = super::run_packet_suite_with_options(&cfg, &options);
    match result {
        Ok(report) => assert!(report.fixture_count >= 1),
        Err(err) => {
            let message = err.to_string();
            assert!(
                message.contains("oracle"),
                "expected oracle-class error, got {message}"
            );
        }
    }
}

#[test]
fn live_oracle_unavailable_propagates_without_fallback() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.oracle_root = "/__fp_missing_legacy_oracle__/pandas".into();
    cfg.allow_system_pandas_fallback = false;
    cfg.allow_fixture_fallback = false;
    cfg.require_live_oracle = false;
    // br-frankenpandas-00de2: `default_paths` now resolves `python_bin` to the
    // repository's pinned oracle venv when it exists, and that interpreter is
    // the DESIGNATED oracle rather than a system fallback, so the scenario
    // under test ("no oracle reachable at all") has to name an interpreter
    // that is not the pinned one. `python3` on this host has no pandas; on a
    // host where it does, the fallback flag above still refuses it.
    cfg.python_bin = "python3".to_owned();

    let report = super::run_packet_by_id(&cfg, "FP-P2C-001", super::OracleMode::LiveLegacyPandas)
        .expect("expected report even when cases fail");
    assert!(
        !report.is_green(),
        "expected non-green report without fallback: {report:?}"
    );
    assert!(
        report.results.iter().all(|case| {
            case.mismatch
                .as_deref()
                .is_some_and(|message| message.contains("legacy oracle root does not exist"))
        }),
        "expected oracle-unavailable mismatches in all failed cases: {report:?}"
    );
}

#[test]
fn live_oracle_unavailable_falls_back_to_fixture_when_enabled() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.oracle_root = "/__fp_missing_legacy_oracle__/pandas".into();
    cfg.allow_system_pandas_fallback = false;
    cfg.allow_fixture_fallback = true;
    cfg.require_live_oracle = false;
    // br-frankenpandas-00de2: opt out of the auto-detected pinned venv so
    // there is genuinely no live oracle and the fixture fallback is exercised.
    cfg.python_bin = "python3".to_owned();

    let report = super::run_packet_by_id(&cfg, "FP-P2C-001", super::OracleMode::LiveLegacyPandas)
        .expect("fixture fallback should recover live-oracle unavailability");
    assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-001"));
    assert!(
        !report.oracle_present,
        "test setup should force the legacy oracle path missing: {report:?}"
    );
    assert!(report.fixture_count >= 1, "expected fallback fixtures");
    assert!(
        report.results.iter().all(|case| {
            !case
                .mismatch
                .as_deref()
                .is_some_and(|message| message.contains("legacy oracle root does not exist"))
        }),
        "fixture fallback should not surface oracle-unavailable mismatches: {report:?}"
    );
}

#[test]
fn system_pandas_permission_never_enables_fixture_fallback_mlnu3() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;
    cfg.allow_fixture_fallback = false;

    let unavailable = super::HarnessError::OracleUnavailable("synthetic outage".to_owned());
    assert!(
        !super::should_fallback_to_fixture(&cfg, &unavailable),
        "system-pandas permission must not silently accept a stored fixture"
    );

    cfg.allow_fixture_fallback = true;
    assert!(
        super::should_fallback_to_fixture(&cfg, &unavailable),
        "fixture fallback needs its own explicit permission"
    );
}

#[test]
fn live_oracle_non_oracle_unavailable_errors_still_propagate() {
    // Needs only the in-repo oracle script and a python that does not exist;
    // it used to skip whenever the legacy pandas checkout was absent, which is
    // every host (4qg5w.2).
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;
    // The fallback opt-in is probe-gated, and the probe fails for a missing
    // python, so without this the legacy-root check fired first and the test
    // passed only where FP_REQUIRE_LIVE_ORACLE=1 (CI); a demand is not gated,
    // so every host reaches the spawn this test is about (br-frankenpandas-2svmt).
    cfg.require_live_oracle = true;
    cfg.python_bin = "/__fp_missing_python__/python3".to_owned();

    let report = super::run_packet_by_id(&cfg, "FP-P2C-001", super::OracleMode::LiveLegacyPandas)
        .expect("expected report even when command spawn fails");
    assert!(
        !report.is_green(),
        "expected non-green report for missing python binary: {report:?}"
    );
    assert!(
        report.results.iter().all(|case| {
            case.mismatch
                .as_deref()
                .is_some_and(|message| message.contains("No such file or directory"))
        }),
        "expected command-spawn io error mismatches in all failed cases: {report:?}"
    );
}

/// br-frankenpandas-rc0923-epic-rust-parity-bugs-4qg5w.2: only a pandas that
/// never loaded is "unavailable". Every live-oracle test skips on
/// `OracleUnavailable`, so mapping a pandas raise (or an adapter refusal) there
/// turns a divergence into a pass.
#[test]
fn oracle_error_classification_keeps_raises_out_of_the_skip_path() {
    use super::HarnessError as E;
    let classify =
        |origin, required| super::classify_oracle_error("m".to_owned(), origin, required);

    assert!(matches!(
        classify(Some("pandas"), false),
        E::OracleRaised(_)
    ));
    assert!(matches!(classify(Some("pandas"), true), E::OracleRaised(_)));
    for origin in ["oracle_adapter", "request", "unexpected"] {
        assert!(
            matches!(classify(Some(origin), false), E::OracleAdapterRefused(_)),
            "{origin}"
        );
    }
    // A pandas that never loaded, or a response too old to carry an origin.
    for origin in [Some("oracle_unavailable"), None] {
        assert!(
            matches!(classify(origin, false), E::OracleUnavailable(_)),
            "{origin:?}"
        );
        assert!(
            matches!(classify(origin, true), E::LiveOracleRequired(_)),
            "{origin:?}"
        );
    }
}

/// The negative the bead names: pandas raises, the case does not expect an
/// error, and the result must be something the standard skip arm
/// (`if let Err(OracleUnavailable(_))`) does NOT catch.
#[test]
fn live_oracle_pandas_raise_on_a_value_case_is_not_a_skip() {
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.allow_system_pandas_fallback = true;
    let fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-LIVE-RAISE-NOT-SKIP",
        "case_id": "series_asof_string_label_value_case",
        "mode": "strict",
        "operation": "series_asof",
        "oracle_source": "live_legacy_pandas",
        "asof_label": { "kind": "utf8", "value": "b" },
        "left": {
            "name": "vals",
            "index": [
                { "kind": "utf8", "value": "a" },
                { "kind": "utf8", "value": "b" }
            ],
            "values": [
                { "kind": "float64", "value": 1.0 },
                { "kind": "float64", "value": 2.0 }
            ]
        }
    }))
    .expect("fixture");

    let result = super::capture_live_oracle_expected(&cfg, &fixture);
    if let Err(super::HarnessError::OracleUnavailable(message)) = &result {
        eprintln!("live pandas unavailable; skipping raise-classification check: {message}");
        return;
    }
    match result {
        Err(super::HarnessError::OracleRaised(message)) => assert!(
            message.contains("Unknown datetime string format"),
            "{message}"
        ),
        other => panic!("pandas raised on this case; expected OracleRaised, got {other:?}"),
    }
}

#[test]
#[allow(clippy::too_many_lines)]
fn live_oracle_error_exception_parity_campaign_j0lg5_7() {
    use std::{
        collections::BTreeMap,
        io::Write,
        process::{Command, Stdio},
    };

    use fp_columnar::Column;
    use fp_frame::{DataFrame, Series, calendar_date_range};
    use fp_index::{DuplicateKeep, Index, IndexError};
    use fp_io::{JsonOrient, read_csv_str, read_json_str};
    use fp_join::{
        JoinType, MergeExecutionOptions, MergeValidateMode, merge_dataframes_on_with_options,
    };
    use fp_types::{DType, NullKind, Scalar, Timedelta};

    use super::{
        ExpectedErrorCategory as Cat, classify_error_message_category,
        classify_frame_error_category, classify_index_error_category, classify_io_error_category,
        classify_join_error_category, classify_python_exception_class_category,
        compare_expected_error_category,
    };

    // 1. NEGATIVE self-check: a mismatch between FrankenPandas' error category/class
    // and pandas' exception category/class MUST fail the comparator and oracle resolver.
    let key_err =
        fp_frame::FrameError::CompatibilityRejected("column 'missing' not found".to_owned());
    let val_err = fp_frame::FrameError::CompatibilityRejected(
        "attempt to get argmin of an empty sequence".to_owned(),
    );
    let type_err = fp_frame::FrameError::CompatibilityRejected(
        "Cannot convert tz-naive timestamps".to_owned(),
    );
    let idx_err = fp_frame::FrameError::Index(IndexError::OutOfBounds {
        position: 5,
        length: 2,
    });
    assert!(compare_expected_error_category(&key_err, Cat::Key));
    assert!(!compare_expected_error_category(&key_err, Cat::Value));
    assert!(!compare_expected_error_category(&val_err, Cat::Key));
    assert!(!compare_expected_error_category(&type_err, Cat::Value));
    assert!(!compare_expected_error_category(&idx_err, Cat::Key));

    let mismatch_fixture: super::PacketFixture = serde_json::from_value(serde_json::json!({
        "packet_id": "FP-P2D-ERR-CAT-NEG",
        "case_id": "negative_category_mismatch_must_fail",
        "mode": "strict",
        "operation": "series_loc",
        "expected_error_category": "type"
    }))
    .expect("mismatch_fixture");
    let neg_res = super::resolve_expected_oracle_error(
        &mismatch_fixture,
        "KeyError: 'loc label not found'".to_owned(),
    );
    assert!(
        neg_res.is_err(),
        "NEGATIVE check failed: expected category mismatch (Type vs Key) to fail, got {neg_res:?}"
    );

    // 2. Categorize all active error fixtures in crates/fp-conformance/fixtures/packets/*.json
    let cfg = super::HarnessConfig::default_paths();
    let packets_dir = cfg.repo_root.join("crates/fp-conformance/fixtures/packets");
    let mut active_fixtures = 0usize;
    let mut error_fixtures = 0usize;
    let mut categorized_fixtures = 0usize;
    let mut cat_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for entry in std::fs::read_dir(&packets_dir).expect("read packets dir") {
        let entry = entry.expect("dir entry");
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = std::fs::read_to_string(entry.path()).expect("read fixture");
        let val: serde_json::Value = serde_json::from_str(&raw).expect("parse fixture json");
        if val.get("retired").is_some_and(|r| !r.is_null()) {
            continue;
        }
        active_fixtures += 1;
        if let Some(substr) = val.get("expected_error_contains").and_then(|v| v.as_str()) {
            error_fixtures += 1;
            let (cat, py_cls) = classify_error_message_category(substr);
            assert!(
                classify_python_exception_class_category(py_cls) == Some(cat),
                "inconsistent class->category mapping for {substr:?}: ({cat:?}, {py_cls})"
            );
            categorized_fixtures += 1;
            *cat_counts.entry(cat.as_str()).or_insert(0) += 1;
        }
    }
    assert!(
        error_fixtures >= 80 && categorized_fixtures == error_fixtures,
        "expected all active error fixtures to be categorized: {categorized_fixtures}/{error_fixtures} (active={active_fixtures})"
    );

    // 3. Live-oracle differential campaign across all 9 SCOPE families of j0lg5.7
    let script = r#"
import json
import pandas as pd
import numpy as np

cases = [
    # Family 1: Missing column / label (KeyError)
    ("df_getitem_missing_col", lambda: pd.DataFrame({"a": [1, 2]})[["missing"]]),
    ("df_drop_missing_col", lambda: pd.DataFrame({"a": [1, 2]}).drop(columns=["missing"])),
    ("df_set_index_missing_col", lambda: pd.DataFrame({"a": [1, 2]}).set_index("missing")),
    ("df_sort_values_missing_col", lambda: pd.DataFrame({"a": [1, 2]}).sort_values("missing")),
    ("df_drop_duplicates_missing_subset", lambda: pd.DataFrame({"a": [1, 2]}).drop_duplicates(subset=["missing"])),
    ("df_explode_missing_col", lambda: pd.DataFrame({"a": ["1,2", "3,4"]}).explode("missing")),
    ("df_loc_missing_label", lambda: pd.DataFrame({"a": [1, 2]}, index=["x", "y"]).loc[["z"]]),
    ("series_loc_missing_label", lambda: pd.Series([1, 2], index=["x", "y"]).loc[["z"]]),
    ("df_xs_missing_label", lambda: pd.DataFrame({"a": [1, 2]}, index=["x", "y"]).xs("z")),
    ("series_xs_missing_label", lambda: pd.Series([1, 2], index=["x", "y"]).xs("z")),
    ("groupby_get_group_missing", lambda: pd.DataFrame({"k": ["a", "b"], "v": [1, 2]}).groupby("k").get_group("z")),
    ("df_pivot_missing_col", lambda: pd.DataFrame({"a": [1], "b": [2]}).pivot(index="missing", columns="a", values="b")),
    ("index_get_loc_missing", lambda: pd.Index(["a", "b"]).get_loc("z")),

    # Family 2: Bad dtype cast & type mismatches (ValueError / TypeError / IntCastingNaNError)
    ("series_astype_str_to_int64", lambda: pd.Series(["a", "b"]).astype("int64")),
    ("series_astype_str_to_float64", lambda: pd.Series(["not_float"]).astype("float64")),
    ("series_astype_nan_to_int64", lambda: pd.Series([1.0, np.nan]).astype("int64")),
    ("to_datetime_invalid_str_raise", lambda: pd.to_datetime(pd.Series(["not-a-date"]), errors="raise")),
    ("to_timedelta_invalid_str_raise", lambda: pd.to_timedelta(pd.Series(["not-a-td"]), errors="raise")),
    ("to_numeric_invalid_str_raise", lambda: pd.to_numeric(pd.Series(["not-a-num"]), errors="raise")),
    ("series_between_incomparable_str_int", lambda: pd.Series(["a", "c"]).between(0, 2)),
    ("series_between_incomparable_dt_int", lambda: pd.Series(pd.to_datetime(["2024-01-01", "2024-01-02"])).between(0, 2)),
    ("series_dt_tz_convert_naive", lambda: pd.Series(pd.to_datetime(["2024-01-01"])).dt.tz_convert("UTC")),
    ("series_dt_tz_localize_already_aware", lambda: pd.Series(pd.to_datetime(["2024-01-01"]).tz_localize("UTC")).dt.tz_localize("UTC")),
    ("series_mean_utf8", lambda: pd.Series(["a", "b"]).mean()),
    ("series_var_utf8", lambda: pd.Series(["a", "b"]).var()),
    ("series_std_utf8", lambda: pd.Series(["a", "b"]).std()),
    ("series_bool_sub", lambda: pd.Series([True, False]) - pd.Series([True, True])),
    ("categorical_unordered_min", lambda: pd.Series(pd.Categorical(["b", "a"], ordered=False)).min()),

    # Family 3: Positional & boolean indexing errors (IndexError / IndexingError / ValueError)
    ("series_iloc_oob_positive", lambda: pd.Series([10, 20]).iloc[[5]]),
    ("series_iloc_oob_negative", lambda: pd.Series([10, 20]).iloc[[-5]]),
    ("df_iloc_oob_positive", lambda: pd.DataFrame({"a": [10, 20]}).iloc[[5]]),
    ("df_iloc_oob_negative", lambda: pd.DataFrame({"a": [10, 20]}).iloc[[-5]]),
    ("series_take_oob", lambda: pd.Series([10, 20]).take([5])),
    ("df_take_oob", lambda: pd.DataFrame({"a": [10, 20]}).take([5])),
    ("series_bool_mask_wrong_length", lambda: pd.Series([10, 20, 30])[[True, False]]),
    ("df_bool_mask_wrong_length", lambda: pd.DataFrame({"a": [10, 20, 30]})[[True, False]]),
    ("series_bool_series_unalignable", lambda: pd.Series([10, 20, 30], index=["a", "b", "c"])[pd.Series([True, False], index=["a", "z"])]),
    ("df_bool_series_unalignable", lambda: pd.DataFrame({"a": [10, 20, 30]}, index=["a", "b", "c"]).loc[pd.Series([True, False], index=["a", "z"])]),
    ("series_iloc_bool_series_int_index", lambda: pd.Series([10, 20, 30]).iloc[pd.Series([True, False, True])]),

    # Family 4: Merge validate & join errors (MergeError / ValueError)
    ("merge_validate_one_to_one_dup_left", lambda: pd.DataFrame({"k": [1, 1], "v": [10, 20]}).merge(pd.DataFrame({"k": [1, 2], "w": [30, 40]}), on="k", validate="one_to_one")),
    ("merge_validate_one_to_one_dup_right", lambda: pd.DataFrame({"k": [1, 2], "v": [10, 20]}).merge(pd.DataFrame({"k": [1, 1], "w": [30, 40]}), on="k", validate="one_to_one")),
    ("merge_validate_one_to_many_dup_left", lambda: pd.DataFrame({"k": [1, 1], "v": [10, 20]}).merge(pd.DataFrame({"k": [1, 2], "w": [30, 40]}), on="k", validate="one_to_many")),
    ("merge_validate_many_to_one_dup_right", lambda: pd.DataFrame({"k": [1, 2], "v": [10, 20]}).merge(pd.DataFrame({"k": [1, 1], "w": [30, 40]}), on="k", validate="many_to_one")),
    ("merge_empty_suffixes_overlap", lambda: pd.DataFrame({"k": [1], "v": [10]}).merge(pd.DataFrame({"k": [1], "v": [30]}), on="k", suffixes=("", ""))),
    ("merge_cross_with_on", lambda: pd.DataFrame({"k": [1]}).merge(pd.DataFrame({"k": [1]}), how="cross", on="k")),

    # Family 5: Invalid frequency strings (ValueError)
    ("date_range_invalid_freq", lambda: pd.date_range("2024-01-01", periods=3, freq="INVALID_FREQ")),
    ("bdate_range_invalid_freq", lambda: pd.bdate_range("2024-01-01", periods=3, freq="INVALID_FREQ")),
    ("timedelta_range_invalid_freq", lambda: pd.timedelta_range("0 days", periods=3, freq="INVALID_FREQ")),
    ("period_range_invalid_freq", lambda: pd.period_range("2024-01", periods=3, freq="INVALID_FREQ")),
    ("series_resample_invalid_freq", lambda: pd.Series([1, 2], index=pd.date_range("2024-01-01", periods=2, freq="D")).resample("INVALID_FREQ")),
    ("df_resample_invalid_freq", lambda: pd.DataFrame({"a": [1, 2]}, index=pd.date_range("2024-01-01", periods=2, freq="D")).resample("INVALID_FREQ")),
    ("series_dt_floor_invalid_freq", lambda: pd.Series(pd.date_range("2024-01-01", periods=2, freq="D")).dt.floor("INVALID_FREQ")),
    ("series_dt_ceil_invalid_freq", lambda: pd.Series(pd.date_range("2024-01-01", periods=2, freq="D")).dt.ceil("INVALID_FREQ")),
    ("series_dt_round_invalid_freq", lambda: pd.Series(pd.date_range("2024-01-01", periods=2, freq="D")).dt.round("INVALID_FREQ")),

    # Family 6: Duplicate labels with allows_duplicate_labels=False & uniqueness (DuplicateLabelError / ValueError)
    ("series_set_flags_disallow_dup_index", lambda: pd.Series([1, 2], index=["a", "a"]).set_flags(allows_duplicate_labels=False)),
    ("df_set_flags_disallow_dup_index", lambda: pd.DataFrame({"x": [1, 2]}, index=["a", "a"]).set_flags(allows_duplicate_labels=False)),
    ("df_set_flags_disallow_dup_columns", lambda: pd.DataFrame([[1, 2]], columns=["a", "a"]).set_flags(allows_duplicate_labels=False)),
    ("series_reindex_dup_source", lambda: pd.Series([1, 2], index=["a", "a"]).reindex(["a", "b"])),
    ("df_reindex_dup_source", lambda: pd.DataFrame({"x": [1, 2]}, index=["a", "a"]).reindex(["a", "b"])),
    ("df_pivot_dup_entries", lambda: pd.DataFrame({"i": ["a", "a"], "c": ["x", "x"], "v": [1, 2]}).pivot(index="i", columns="c", values="v")),
    ("df_set_index_verify_integrity_dup", lambda: pd.DataFrame({"k": ["a", "a"], "v": [1, 2]}).set_index("k", verify_integrity=True)),
    ("df_insert_dup_column", lambda: (lambda d: (d.insert(0, "a", [3, 4]), d)[1])(pd.DataFrame({"a": [1, 2]}))),

    # Family 7: Empty-input reductions & concatenations (ValueError)
    ("series_empty_idxmin", lambda: pd.Series([], dtype="float64").idxmin()),
    ("series_empty_idxmax", lambda: pd.Series([], dtype="float64").idxmax()),
    ("series_empty_argmin", lambda: pd.Series([], dtype="float64").argmin()),
    ("series_empty_argmax", lambda: pd.Series([], dtype="float64").argmax()),
    ("df_empty_idxmin", lambda: pd.DataFrame({"a": pd.Series([], dtype="float64")}).idxmin()),
    ("df_empty_idxmax", lambda: pd.DataFrame({"a": pd.Series([], dtype="float64")}).idxmax()),
    ("concat_empty_list", lambda: pd.concat([])),
    ("series_agg_empty_list", lambda: pd.Series([1.0, 2.0]).agg([])),

    # Family 8: read_csv / IO malformed input & missing files (ParserError / FileNotFoundError / ValueError)
    ("read_csv_extra_field", lambda: pd.read_csv(pd.io.common.StringIO("a,b\n1,2\n1,2,3\n"))),
    ("read_csv_unclosed_quote", lambda: pd.read_csv(pd.io.common.StringIO("a,b\n\"unclosed,2\n"))),
    ("read_csv_missing_file", lambda: pd.read_csv("/nonexistent/frankenpandas_missing_file_j0lg5_7.csv")),
    ("read_json_malformed", lambda: pd.read_json(pd.io.common.StringIO("{malformed_json"))),
]

out = {}
for cid, fn in cases:
    try:
        fn()
        out[cid] = {"raised": False}
    except Exception as e:
        out[cid] = {
            "raised": True,
            "class": type(e).__name__,
            "mro": [b.__name__ for b in type(e).__mro__],
            "message": str(e),
        }
print(json.dumps(out))
"#;

    let mut child = Command::new(&cfg.python_bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python oracle");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("write script");
    let output = child.wait_with_output().expect("wait python oracle");
    assert!(
        output.status.success(),
        "live oracle script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let oracle: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse oracle JSON");

    let assert_case = |case_id: &str, fp_cat: Cat, fp_py_cls: &str| {
        let entry = oracle.get(case_id).expect(case_id);
        assert_eq!(
            entry.get("raised").and_then(|v| v.as_bool()),
            Some(true),
            "oracle did not raise for {case_id}: {entry}"
        );
        let pd_cls = entry
            .get("class")
            .and_then(|v| v.as_str())
            .expect("oracle exception class");
        let pd_mro: Vec<&str> = entry
            .get("mro")
            .and_then(|v| v.as_array())
            .expect("oracle exception mro")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        let pd_cat = classify_python_exception_class_category(pd_cls).expect(case_id);
        assert_eq!(
            fp_cat, pd_cat,
            "category mismatch on {case_id}: FrankenPandas={fp_cat:?} ({fp_py_cls}) vs pandas={pd_cat:?} ({pd_cls}, mro={pd_mro:?})"
        );
        assert!(
            pd_cls == fp_py_cls || pd_mro.contains(&fp_py_cls),
            "Python exception class mismatch on {case_id}: FrankenPandas mapped to {fp_py_cls}, pandas raised {pd_cls} (mro={pd_mro:?})"
        );
    };

    // Helper fixtures
    let df_ab = DataFrame::from_dict(
        &["a"],
        vec![("a", vec![Scalar::Int64(1), Scalar::Int64(2)])],
    )
    .unwrap();
    let df_xy = DataFrame::new_with_column_order(
        Index::new(vec!["x".into(), "y".into()]),
        BTreeMap::from([(
            "a".to_owned(),
            Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).unwrap(),
        )]),
        vec!["a".to_owned()],
    )
    .unwrap();
    let s_xy = Series::from_values(
        "a",
        vec!["x".into(), "y".into()],
        vec![Scalar::Int64(1), Scalar::Int64(2)],
    )
    .unwrap();

    // Family 1: Missing column / label (KeyError)
    let (c, cls) = classify_frame_error_category(&df_ab.select_columns(&["missing"]).unwrap_err());
    assert_case("df_getitem_missing_col", c, cls);
    let (c, cls) = classify_frame_error_category(&df_ab.drop_columns(&["missing"]).unwrap_err());
    assert_case("df_drop_missing_col", c, cls);
    let (c, cls) = classify_frame_error_category(
        &df_ab
            .set_index_with_verify_integrity("missing", true, false)
            .unwrap_err(),
    );
    assert_case("df_set_index_missing_col", c, cls);
    let (c, cls) = classify_frame_error_category(&df_ab.sort_values("missing", true).unwrap_err());
    assert_case("df_sort_values_missing_col", c, cls);
    let (c, cls) = classify_frame_error_category(
        &df_ab
            .drop_duplicates(Some(&["missing".to_owned()]), DuplicateKeep::First, false)
            .unwrap_err(),
    );
    assert_case("df_drop_duplicates_missing_subset", c, cls);
    let (c, cls) = classify_frame_error_category(&df_ab.explode("missing", ",").unwrap_err());
    assert_case("df_explode_missing_col", c, cls);
    let (c, cls) = classify_frame_error_category(&df_xy.loc(&["z".into()]).unwrap_err());
    assert_case("df_loc_missing_label", c, cls);
    let (c, cls) = classify_frame_error_category(&s_xy.loc(&["z".into()]).unwrap_err());
    assert_case("series_loc_missing_label", c, cls);
    let (c, cls) = classify_frame_error_category(&df_xy.xs(&"z".into()).unwrap_err());
    assert_case("df_xs_missing_label", c, cls);
    let (c, cls) = classify_frame_error_category(&s_xy.xs(&"z".into()).unwrap_err());
    assert_case("series_xs_missing_label", c, cls);
    let df_kv = DataFrame::from_dict(
        &["k", "v"],
        vec![
            (
                "k",
                vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into())],
            ),
            ("v", vec![Scalar::Int64(1), Scalar::Int64(2)]),
        ],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(
        &df_kv
            .groupby_full_options(&["k"], true, true, true)
            .unwrap()
            .get_group("z")
            .unwrap_err(),
    );
    assert_case("groupby_get_group_missing", c, cls);
    let df_piv_src = DataFrame::from_dict(
        &["a", "b"],
        vec![("a", vec![Scalar::Int64(1)]), ("b", vec![Scalar::Int64(2)])],
    )
    .unwrap();
    let (c, cls) =
        classify_frame_error_category(&df_piv_src.pivot("missing", "a", "b").unwrap_err());
    assert_case("df_pivot_missing_col", c, cls);
    let (c, cls) = classify_index_error_category(&IndexError::LabelNotFound("z".into()));
    assert_case("index_get_loc_missing", c, cls);

    // Family 2: Bad dtype cast & type mismatches (ValueError / TypeError / IntCastingNaNError)
    let s_str = Series::from_values(
        "s",
        vec![0_i64.into(), 1_i64.into()],
        vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into())],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&s_str.astype(DType::Int64).unwrap_err());
    assert_case("series_astype_str_to_int64", c, cls);
    let (c, cls) = classify_frame_error_category(&s_str.astype(DType::Float64).unwrap_err());
    assert_case("series_astype_str_to_float64", c, cls);
    let s_nan = Series::from_values(
        "s",
        vec![0_i64.into(), 1_i64.into()],
        vec![Scalar::Float64(1.0), Scalar::Null(NullKind::NaN)],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&s_nan.astype(DType::Int64).unwrap_err());
    assert_case("series_astype_nan_to_int64", c, cls);
    let s_bad_dt = Series::from_values(
        "s",
        vec![0_i64.into()],
        vec![Scalar::Utf8("not-a-date".into())],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&fp_frame::to_datetime(&s_bad_dt).unwrap_err());
    assert_case("to_datetime_invalid_str_raise", c, cls);
    let s_bad_td = Series::from_values(
        "s",
        vec![0_i64.into()],
        vec![Scalar::Utf8("not-a-td".into())],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&fp_frame::to_timedelta(&s_bad_td).unwrap_err());
    assert_case("to_timedelta_invalid_str_raise", c, cls);
    let (c, cls) = classify_frame_error_category(&fp_frame::to_numeric(&s_str).unwrap_err());
    assert_case("to_numeric_invalid_str_raise", c, cls);
    let (c, cls) = classify_frame_error_category(
        &s_str
            .between(&Scalar::Int64(0), &Scalar::Int64(2), "both")
            .unwrap_err(),
    );
    assert_case("series_between_incomparable_str_int", c, cls);
    let s_dt = fp_frame::to_datetime(
        &Series::from_values(
            "dt",
            vec![0_i64.into(), 1_i64.into()],
            vec![
                Scalar::Utf8("2024-01-01".into()),
                Scalar::Utf8("2024-01-02".into()),
            ],
        )
        .unwrap(),
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(
        &s_dt
            .between(&Scalar::Int64(0), &Scalar::Int64(2), "both")
            .unwrap_err(),
    );
    assert_case("series_between_incomparable_dt_int", c, cls);
    let (c, cls) = classify_frame_error_category(&s_dt.dt().tz_convert(Some("UTC")).unwrap_err());
    assert_case("series_dt_tz_convert_naive", c, cls);
    let s_dt_utc = s_dt.dt().tz_localize(Some("UTC")).unwrap();
    let (c, cls) =
        classify_frame_error_category(&s_dt_utc.dt().tz_localize(Some("UTC")).unwrap_err());
    assert_case("series_dt_tz_localize_already_aware", c, cls);
    let (c, cls) = classify_frame_error_category(&s_str.mean().unwrap_err());
    assert_case("series_mean_utf8", c, cls);
    let (c, cls) = classify_frame_error_category(&s_str.var().unwrap_err());
    assert_case("series_var_utf8", c, cls);
    let (c, cls) = classify_frame_error_category(&s_str.std().unwrap_err());
    assert_case("series_std_utf8", c, cls);
    let s_bool1 = Series::from_values(
        "b",
        vec![0_i64.into(), 1_i64.into()],
        vec![Scalar::Bool(true), Scalar::Bool(false)],
    )
    .unwrap();
    let s_bool2 = Series::from_values(
        "b",
        vec![0_i64.into(), 1_i64.into()],
        vec![Scalar::Bool(true), Scalar::Bool(true)],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&s_bool1.sub(&s_bool2).unwrap_err());
    assert_case("series_bool_sub", c, cls);
    let s_cat_unord = Series::from_categorical(
        "c",
        vec![Scalar::Utf8("a".into()), Scalar::Utf8("b".into())],
        false,
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&s_cat_unord.min().unwrap_err());
    assert_case("categorical_unordered_min", c, cls);

    // Family 3: Positional & boolean indexing errors (IndexError / IndexingError / ValueError)
    let (c, cls) = classify_frame_error_category(&s_xy.iloc(&[5]).unwrap_err());
    assert_case("series_iloc_oob_positive", c, cls);
    let (c, cls) = classify_frame_error_category(&s_xy.take(&[-5]).unwrap_err());
    assert_case("series_iloc_oob_negative", c, cls);
    let (c, cls) = classify_frame_error_category(&df_ab.iloc(&[5]).unwrap_err());
    assert_case("df_iloc_oob_positive", c, cls);
    let (c, cls) = classify_frame_error_category(&df_ab.take(&[-5], 0).unwrap_err());
    assert_case("df_iloc_oob_negative", c, cls);
    let (c, cls) = classify_frame_error_category(&s_xy.take(&[5]).unwrap_err());
    assert_case("series_take_oob", c, cls);
    let (c, cls) = classify_frame_error_category(&df_ab.take(&[5], 0).unwrap_err());
    assert_case("df_take_oob", c, cls);
    let s_3 = Series::from_values(
        "s",
        vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&s_3.iloc_bool(&[true, false]).unwrap_err());
    assert_case("series_bool_mask_wrong_length", c, cls);
    let s_abc = Series::from_values(
        "s",
        vec!["a".into(), "b".into(), "c".into()],
        vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)],
    )
    .unwrap();
    let df_abc = DataFrame::new_with_column_order(
        Index::new(vec!["a".into(), "b".into(), "c".into()]),
        BTreeMap::from([(
            "a".to_owned(),
            Column::from_values(vec![Scalar::Int64(10), Scalar::Int64(20), Scalar::Int64(30)])
                .unwrap(),
        )]),
        vec!["a".to_owned()],
    )
    .unwrap();
    let mask_az = Series::from_values(
        "m",
        vec!["a".into(), "z".into()],
        vec![Scalar::Bool(true), Scalar::Bool(false)],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&s_abc.filter(&mask_az).unwrap_err());
    assert_case("series_bool_series_unalignable", c, cls);
    let (c, cls) = classify_frame_error_category(&df_abc.loc_bool_series(&mask_az).unwrap_err());
    assert_case("df_bool_series_unalignable", c, cls);
    let mask_3 = Series::from_values(
        "m",
        vec![0_i64.into(), 1_i64.into(), 2_i64.into()],
        vec![Scalar::Bool(true), Scalar::Bool(false), Scalar::Bool(true)],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&s_3.iloc_bool_series(&mask_3).unwrap_err());
    assert_case("series_iloc_bool_series_int_index", c, cls);

    // Family 4: Merge validate & join errors (MergeError / ValueError)
    let df_dup_l = DataFrame::from_dict(
        &["k", "v"],
        vec![
            ("k", vec![Scalar::Int64(1), Scalar::Int64(1)]),
            ("v", vec![Scalar::Int64(10), Scalar::Int64(20)]),
        ],
    )
    .unwrap();
    let df_uniq_r = DataFrame::from_dict(
        &["k", "w"],
        vec![
            ("k", vec![Scalar::Int64(1), Scalar::Int64(2)]),
            ("w", vec![Scalar::Int64(30), Scalar::Int64(40)]),
        ],
    )
    .unwrap();
    let (c, cls) = classify_join_error_category(
        &merge_dataframes_on_with_options(
            &df_dup_l,
            &df_uniq_r,
            &["k"],
            &["k"],
            JoinType::Inner,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::OneToOne),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap_err(),
    );
    assert_case("merge_validate_one_to_one_dup_left", c, cls);
    let (c, cls) = classify_join_error_category(
        &merge_dataframes_on_with_options(
            &df_uniq_r,
            &df_dup_l,
            &["k"],
            &["k"],
            JoinType::Inner,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::OneToOne),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap_err(),
    );
    assert_case("merge_validate_one_to_one_dup_right", c, cls);
    let (c, cls) = classify_join_error_category(
        &merge_dataframes_on_with_options(
            &df_dup_l,
            &df_uniq_r,
            &["k"],
            &["k"],
            JoinType::Inner,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::OneToMany),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap_err(),
    );
    assert_case("merge_validate_one_to_many_dup_left", c, cls);
    let (c, cls) = classify_join_error_category(
        &merge_dataframes_on_with_options(
            &df_uniq_r,
            &df_dup_l,
            &["k"],
            &["k"],
            JoinType::Inner,
            MergeExecutionOptions {
                validate_mode: Some(MergeValidateMode::ManyToOne),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap_err(),
    );
    assert_case("merge_validate_many_to_one_dup_right", c, cls);
    let (c, cls) = classify_join_error_category(
        &merge_dataframes_on_with_options(
            &df_dup_l,
            &df_dup_l,
            &["k"],
            &["k"],
            JoinType::Inner,
            MergeExecutionOptions {
                suffixes: Some([None, None]),
                ..MergeExecutionOptions::default()
            },
        )
        .unwrap_err(),
    );
    assert_case("merge_empty_suffixes_overlap", c, cls);

    // Family 5: Invalid frequency strings (ValueError)
    let dti = fp_index::date_range(
        Some("2024-01-01"),
        None,
        Some(2),
        Timedelta::NANOS_PER_DAY,
        None,
    )
    .unwrap();
    let s_dti = Series::from_values(
        "s",
        dti.labels().to_vec(),
        vec![Scalar::Int64(1), Scalar::Int64(2)],
    )
    .unwrap();
    let df_dti = DataFrame::new_with_column_order(
        dti.clone(),
        BTreeMap::from([(
            "a".to_owned(),
            Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).unwrap(),
        )]),
        vec!["a".to_owned()],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(
        &calendar_date_range(Some(0), None, Some(3), "INVALID_FREQ").unwrap_err(),
    );
    assert_case("date_range_invalid_freq", c, cls);
    let (c, cls) = classify_error_message_category(
        &fp_index::date_range(Some("2024-01-01"), None, Some(3), 0, None)
            .unwrap_err()
            .to_string(),
    );
    assert_case("bdate_range_invalid_freq", c, cls);
    let (c, cls) = classify_error_message_category(
        &fp_index::timedelta_range(Some(0), None, Some(3), 0, None)
            .unwrap_err()
            .to_string(),
    );
    assert_case("timedelta_range_invalid_freq", c, cls);
    let (c, cls) = classify_frame_error_category(&s_dti.to_period("INVALID_FREQ").unwrap_err());
    assert_case("period_range_invalid_freq", c, cls);
    let (c, cls) =
        classify_frame_error_category(&s_dti.resample("INVALID_FREQ").sum().unwrap_err());
    assert_case("series_resample_invalid_freq", c, cls);
    let (c, cls) =
        classify_frame_error_category(&df_dti.resample("INVALID_FREQ").sum().unwrap_err());
    assert_case("df_resample_invalid_freq", c, cls);
    let (c, cls) = classify_frame_error_category(&s_dt.dt().floor("INVALID_FREQ").unwrap_err());
    assert_case("series_dt_floor_invalid_freq", c, cls);
    let (c, cls) = classify_frame_error_category(&s_dt.dt().ceil("INVALID_FREQ").unwrap_err());
    assert_case("series_dt_ceil_invalid_freq", c, cls);
    let (c, cls) = classify_frame_error_category(&s_dt.dt().round("INVALID_FREQ").unwrap_err());
    assert_case("series_dt_round_invalid_freq", c, cls);

    // Family 6: Duplicate labels with allows_duplicate_labels=False & uniqueness
    let s_dup = Series::from_values(
        "s",
        vec!["a".into(), "a".into()],
        vec![Scalar::Int64(1), Scalar::Int64(2)],
    )
    .unwrap();
    let df_dup = DataFrame::new_with_column_order(
        Index::new(vec!["a".into(), "a".into()]),
        BTreeMap::from([(
            "x".to_owned(),
            Column::from_values(vec![Scalar::Int64(1), Scalar::Int64(2)]).unwrap(),
        )]),
        vec!["x".to_owned()],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&s_dup.set_flags(Some(false)).unwrap_err());
    assert_case("series_set_flags_disallow_dup_index", c, cls);
    let (c, cls) = classify_frame_error_category(&df_dup.set_flags(Some(false)).unwrap_err());
    assert_case("df_set_flags_disallow_dup_index", c, cls);
    let (c, cls) =
        classify_frame_error_category(&s_dup.reindex(vec!["a".into(), "b".into()]).unwrap_err());
    assert_case("series_reindex_dup_source", c, cls);
    let (c, cls) =
        classify_frame_error_category(&df_dup.reindex(vec!["a".into(), "b".into()]).unwrap_err());
    assert_case("df_reindex_dup_source", c, cls);
    let df_piv_dup = DataFrame::from_dict(
        &["i", "c", "v"],
        vec![
            (
                "i",
                vec![Scalar::Utf8("a".into()), Scalar::Utf8("a".into())],
            ),
            (
                "c",
                vec![Scalar::Utf8("x".into()), Scalar::Utf8("x".into())],
            ),
            ("v", vec![Scalar::Int64(1), Scalar::Int64(2)]),
        ],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&df_piv_dup.pivot("i", "c", "v").unwrap_err());
    assert_case("df_pivot_dup_entries", c, cls);
    let (c, cls) = classify_frame_error_category(
        &df_piv_dup
            .set_index_with_verify_integrity("i", true, true)
            .unwrap_err(),
    );
    assert_case("df_set_index_verify_integrity_dup", c, cls);
    let (c, cls) = classify_frame_error_category(
        &df_ab
            .insert(
                0,
                "a",
                Column::from_values(vec![Scalar::Int64(3), Scalar::Int64(4)]).unwrap(),
            )
            .unwrap_err(),
    );
    assert_case("df_insert_dup_column", c, cls);

    // Family 7: Empty-input reductions & concatenations (ValueError)
    let s_empty = Series::new(
        "s",
        Index::new(vec![]),
        Column::new(DType::Float64, vec![]).unwrap(),
    )
    .unwrap();
    let df_empty = DataFrame::new_with_column_order(
        Index::new(vec![]),
        BTreeMap::from([("a".to_owned(), Column::new(DType::Float64, vec![]).unwrap())]),
        vec!["a".to_owned()],
    )
    .unwrap();
    let (c, cls) = classify_frame_error_category(&s_empty.idxmin().unwrap_err());
    assert_case("series_empty_idxmin", c, cls);
    let (c, cls) = classify_frame_error_category(&s_empty.idxmax().unwrap_err());
    assert_case("series_empty_idxmax", c, cls);
    let (c, cls) = classify_frame_error_category(&s_empty.argmin().unwrap_err());
    assert_case("series_empty_argmin", c, cls);
    let (c, cls) = classify_frame_error_category(&s_empty.argmax().unwrap_err());
    assert_case("series_empty_argmax", c, cls);
    let (c, cls) = classify_frame_error_category(&df_empty.idxmin().unwrap_err());
    assert_case("df_empty_idxmin", c, cls);
    let (c, cls) = classify_frame_error_category(&df_empty.idxmax().unwrap_err());
    assert_case("df_empty_idxmax", c, cls);
    let (c, cls) = classify_frame_error_category(&fp_frame::concat_series(&[]).unwrap_err());
    assert_case("concat_empty_list", c, cls);
    let (c, cls) = classify_frame_error_category(&fp_frame::concat_dataframes(&[]).unwrap_err());
    assert_case("concat_empty_list", c, cls);
    let (c, cls) = classify_frame_error_category(&s_3.agg(&[]).unwrap_err());
    assert_case("series_agg_empty_list", c, cls);
    let (c, cls) = classify_frame_error_category(
        &df_ab
            .agg(&std::collections::HashMap::new())
            .unwrap_err(),
    );
    assert_case("series_agg_empty_list", c, cls);

    // Family 8: read_csv / IO malformed input & missing files (ParserError / FileNotFoundError / ValueError)
    let (c, cls) = classify_io_error_category(&read_csv_str("a,b\n1,2\n1,2,3\n").unwrap_err());
    assert_case("read_csv_extra_field", c, cls);
    let (c, cls) = classify_io_error_category(&read_csv_str("a,b\n\"unclosed,2\n").unwrap_err());
    assert_case("read_csv_unclosed_quote", c, cls);
    let io_not_found = fp_io::IoError::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "No such file or directory",
    ));
    let (c, cls) = classify_io_error_category(&io_not_found);
    assert_case("read_csv_missing_file", c, cls);
    let (c, cls) = classify_io_error_category(
        &read_json_str("{malformed_json", JsonOrient::Records).unwrap_err(),
    );
    assert_case("read_json_malformed", c, cls);

    // Family 9: SettingWithCopy-free semantics (derived slices/filters mutate independently without error or parent mutation)
    let mask_first = Series::from_values(
        "m",
        vec![0_i64.into(), 1_i64.into()],
        vec![Scalar::Bool(true), Scalar::Bool(false)],
    )
    .unwrap();
    let filtered = df_ab.filter_rows(&mask_first).unwrap();
    let mutated = filtered
        .assign(vec![(
            "a",
            Column::from_values(vec![Scalar::Int64(999)]).unwrap(),
        )])
        .unwrap();
    assert_eq!(
        mutated.column("a").unwrap().values(),
        &[Scalar::Int64(999)],
        "derived frame mutation should succeed without SettingWithCopyError"
    );
    assert_eq!(
        df_ab.column("a").unwrap().values(),
        &[Scalar::Int64(1), Scalar::Int64(2)],
        "parent frame must remain unmodified (SettingWithCopy-free value semantics)"
    );
}
