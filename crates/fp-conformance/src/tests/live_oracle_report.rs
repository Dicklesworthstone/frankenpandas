use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn parse_live_oracle_report_counts_runs_skips_and_failures() {
    let output = "\
running 5 tests
test a ... ok
live pandas unavailable; skipping dataframe foo oracle test: missing pandas
test b ... ok
test c ... FAILED
test d ... ok
test e ... ok

test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
";

    let report = super::parse_live_oracle_report_output(output, 5, false, Some(101));
    assert_eq!(report.expected_tests, 5);
    assert_eq!(report.total_tests, 5);
    assert_eq!(report.passed, 4);
    assert_eq!(report.ran, 3);
    assert_eq!(report.skipped, 1);
    assert_eq!(report.failed, 1);
    assert!(!report.command_ok);
    assert_eq!(report.exit_code, Some(101));
}

#[test]
fn write_live_oracle_report_json_machine_readable_artifact() {
    let tmp = tempfile::tempdir().expect("tmp");
    let mut cfg = super::HarnessConfig::default_paths();
    cfg.repo_root = tmp.path().to_path_buf();

    let report = super::LiveOracleReport {
        expected_tests: 89,
        total_tests: 89,
        passed: 89,
        ran: 89,
        skipped: 0,
        failed: 0,
        command_ok: true,
        exit_code: Some(0),
    };

    let path = super::write_live_oracle_report(&cfg, &report).expect("write live report");
    assert!(
        path.ends_with("artifacts/ci/live_oracle_report.json"),
        "unexpected live oracle report path: {}",
        path.display()
    );

    let written: super::LiveOracleReport =
        serde_json::from_str(&fs::read_to_string(path).expect("read")).expect("json");
    assert_eq!(written, report);
    assert!(written.is_green(true));
}

#[test]
fn ci_workflow_uploads_live_oracle_report_artifact() {
    let root = repo_root();
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml")).expect("read ci");
    assert!(
        ci.contains("artifacts/ci/live_oracle_report.json"),
        "expected CI artifact upload for live_oracle_report.json"
    );
}
