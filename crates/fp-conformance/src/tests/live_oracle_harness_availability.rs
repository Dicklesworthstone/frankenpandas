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
