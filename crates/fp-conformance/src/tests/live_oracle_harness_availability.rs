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
    cfg.require_live_oracle = false;

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
    cfg.allow_system_pandas_fallback = true;
    cfg.require_live_oracle = false;

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
fn live_oracle_non_oracle_unavailable_errors_still_propagate() {
    let mut cfg = super::HarnessConfig::default_paths();
    if !cfg.oracle_root.exists() {
        eprintln!(
            "oracle repo missing at {}; skipping python-missing check",
            cfg.oracle_root.display()
        );
        return;
    }
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
