#![forbid(unsafe_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const RULE_TOTAL_PARITY: &str = "GOV-001";
const RULE_NO_MINIMAL_SCOPE: &str = "GOV-002";
const RULE_CLEAN_ROOM_METHOD_STACK: &str = "GOV-003";
const REPRO_CMD: &str = "./scripts/governance_gate_check.sh";
const REPORT_PATH: &str = "artifacts/ci/governance_gate_report.json";
const BEAD_ID: &str = "bd-2gi.30";

const METHOD_STACK_MARKERS: [&str; 4] = [
    "alien-artifact-coding",
    "extreme-software-optimization",
    "alien-graveyard",
    "raptorq",
];

const TOTAL_PARITY_MARKERS: [&str; 3] = [
    "absolute and total",
    "drop-in replacement",
    "total feature/functionality overlap",
];

const NARROWING_PHRASES: [&str; 7] = [
    "minimal v1",
    "minimal-v1",
    "partial scope",
    "partial subset",
    "subset only",
    "limited subset",
    "narrow scope",
];

const NEGATION_HINTS: [&str; 12] = [
    "no ",
    "not ",
    "never ",
    "reject",
    "reject language",
    "fail if",
    "forbidden",
    "without ",
    "non-negotiable",
    "disallow",
    "must not",
    "ban",
];

const PLANNING_DOCS: [&str; 1] = ["README.md"];

#[derive(Debug, Clone)]
struct CliArgs {
    repo_root: PathBuf,
    json_out: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct GateEvent {
    rule_id: String,
    artifact_id: String,
    artifact_path: String,
    bead_id: Option<String>,
    trace_id: String,
    result: String,
    repro_cmd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct Violation {
    rule_id: String,
    artifact_id: String,
    artifact_path: String,
    bead_id: Option<String>,
    trace_id: String,
    result: String,
    repro_cmd: String,
    message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct GovernanceGateReport {
    bead_id: String,
    generated_unix_ms: u128,
    repo_root: String,
    all_passed: bool,
    violation_count: usize,
    violations: Vec<Violation>,
    events: Vec<GateEvent>,
}

#[derive(Debug, Clone)]
struct ArtifactRef {
    artifact_id: String,
    artifact_path: String,
    bead_id: Option<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            eprintln!("fp-governance-gate error: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let report = run_governance_gate(&args.repo_root)?;

    print_summary(&report);

    if let Some(path) = args.json_out {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, serde_json::to_string_pretty(&report)?)?;
        println!("wrote governance_gate_report={}", path.display());
    }

    Ok(report.all_passed)
}

fn parse_args() -> Result<CliArgs, Box<dyn std::error::Error>> {
    let default_repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut repo_root = default_repo_root;
    let mut json_out = None;

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--repo-root" => {
                let value = args.next().ok_or("--repo-root requires a path")?;
                repo_root = PathBuf::from(value);
            }
            "--json-out" => {
                let value = args.next().ok_or("--json-out requires a path")?;
                json_out = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}").into()),
        }
    }

    Ok(CliArgs {
        repo_root,
        json_out,
    })
}

fn print_help() {
    println!(
        "fp-governance-gate\n\
         Usage:\n\
         \tfp-governance-gate [--repo-root <path>] [--json-out {REPORT_PATH}]\n\
         Rules:\n\
         \t{RULE_TOTAL_PARITY} require total-parity mandate wording on planning surfaces\n\
         \t{RULE_NO_MINIMAL_SCOPE} reject scope-narrowing language unless explicitly negated\n\
         \t{RULE_CLEAN_ROOM_METHOD_STACK} require clean-room wording + method-stack markers\n\
         Options:\n\
         \t--repo-root <path>  repository root to audit (default: workspace root)\n\
         \t--json-out <path>   write machine-readable report\n\
         \t-h, --help          show this help"
    );
}

fn run_governance_gate(
    repo_root: &Path,
) -> Result<GovernanceGateReport, Box<dyn std::error::Error>> {
    let mut events = Vec::new();
    let mut violations = Vec::new();

    for rel in PLANNING_DOCS {
        let path = repo_root.join(rel);
        let text = fs::read_to_string(&path)?;
        let artifact = ArtifactRef {
            artifact_id: rel.to_owned(),
            artifact_path: rel.to_owned(),
            bead_id: None,
        };
        evaluate_artifact(&artifact, &text, &mut events, &mut violations);
    }

    let issues_path = repo_root.join(".beads/issues.jsonl");
    let issues = read_open_issues(&issues_path)?;
    for issue in issues {
        let text = collect_issue_text(&issue);
        let issue_id = issue
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_owned();
        let artifact = ArtifactRef {
            artifact_id: issue_id.clone(),
            artifact_path: ".beads/issues.jsonl".to_owned(),
            bead_id: Some(issue_id),
        };
        evaluate_artifact(&artifact, &text, &mut events, &mut violations);
    }

    let generated_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let all_passed = violations.is_empty();

    Ok(GovernanceGateReport {
        bead_id: BEAD_ID.to_owned(),
        generated_unix_ms,
        repo_root: repo_root.display().to_string(),
        all_passed,
        violation_count: violations.len(),
        violations,
        events,
    })
}

fn read_open_issues(path: &Path) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    let mut issues = Vec::new();
    for (line_number, raw) in content.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(raw)
            .map_err(|err| format!("{}:{} parse error: {err}", path.display(), line_number + 1))?;
        if value.get("status").and_then(Value::as_str) != Some("closed") {
            issues.push(value);
        }
    }
    Ok(issues)
}

fn collect_issue_text(issue: &Value) -> String {
    let mut parts = Vec::new();
    for field in ["title", "description", "acceptance_criteria", "notes"] {
        if let Some(text) = issue.get(field).and_then(Value::as_str) {
            parts.push(text);
        }
    }
    parts.join("\n")
}

fn evaluate_artifact(
    artifact: &ArtifactRef,
    text: &str,
    events: &mut Vec<GateEvent>,
    violations: &mut Vec<Violation>,
) {
    check_total_parity_rule(artifact, text, events, violations);
    check_scope_narrowing_rule(artifact, text, events, violations);
    check_clean_room_method_stack_rule(artifact, text, events, violations);
}

fn check_total_parity_rule(
    artifact: &ArtifactRef,
    text: &str,
    events: &mut Vec<GateEvent>,
    violations: &mut Vec<Violation>,
) {
    let lower = text.to_ascii_lowercase();
    let passed = TOTAL_PARITY_MARKERS
        .iter()
        .any(|marker| lower.contains(marker));
    let trace_id = trace_id(RULE_TOTAL_PARITY, &artifact.artifact_id);

    events.push(build_event(
        RULE_TOTAL_PARITY,
        artifact,
        &trace_id,
        passed,
        REPRO_CMD,
    ));
    if !passed {
        violations.push(build_violation(
            RULE_TOTAL_PARITY,
            artifact,
            &trace_id,
            "missing total-parity mandate marker (expected phrases like 'ABSOLUTE AND TOTAL' or 'drop-in replacement')",
        ));
    }
}

fn check_scope_narrowing_rule(
    artifact: &ArtifactRef,
    text: &str,
    events: &mut Vec<GateEvent>,
    violations: &mut Vec<Violation>,
) {
    let trace_id = trace_id(RULE_NO_MINIMAL_SCOPE, &artifact.artifact_id);
    let narrowing = find_unnegated_narrowing_phrase(text);
    let passed = narrowing.is_none();

    events.push(build_event(
        RULE_NO_MINIMAL_SCOPE,
        artifact,
        &trace_id,
        passed,
        REPRO_CMD,
    ));

    if let Some(phrase) = narrowing {
        violations.push(build_violation(
            RULE_NO_MINIMAL_SCOPE,
            artifact,
            &trace_id,
            &format!("scope-narrowing phrase detected without explicit negation: '{phrase}'"),
        ));
    }
}

fn check_clean_room_method_stack_rule(
    artifact: &ArtifactRef,
    text: &str,
    events: &mut Vec<GateEvent>,
    violations: &mut Vec<Violation>,
) {
    let lower = text.to_ascii_lowercase();
    let has_clean_room = lower.contains("clean-room");
    let missing_markers: Vec<&str> = METHOD_STACK_MARKERS
        .iter()
        .copied()
        .filter(|marker| !lower.contains(marker))
        .collect();
    let passed = has_clean_room && missing_markers.is_empty();
    let trace_id = trace_id(RULE_CLEAN_ROOM_METHOD_STACK, &artifact.artifact_id);

    events.push(build_event(
        RULE_CLEAN_ROOM_METHOD_STACK,
        artifact,
        &trace_id,
        passed,
        REPRO_CMD,
    ));
    if !passed {
        let mut reasons = Vec::new();
        if !has_clean_room {
            reasons.push("missing clean-room wording".to_owned());
        }
        if !missing_markers.is_empty() {
            reasons.push(format!(
                "missing method-stack marker(s): {}",
                missing_markers.join(", ")
            ));
        }
        violations.push(build_violation(
            RULE_CLEAN_ROOM_METHOD_STACK,
            artifact,
            &trace_id,
            &reasons.join("; "),
        ));
    }
}

fn build_event(
    rule_id: &str,
    artifact: &ArtifactRef,
    trace_id: &str,
    passed: bool,
    repro_cmd: &str,
) -> GateEvent {
    GateEvent {
        rule_id: rule_id.to_owned(),
        artifact_id: artifact.artifact_id.clone(),
        artifact_path: artifact.artifact_path.clone(),
        bead_id: artifact.bead_id.clone(),
        trace_id: trace_id.to_owned(),
        result: if passed { "pass" } else { "fail" }.to_owned(),
        repro_cmd: repro_cmd.to_owned(),
    }
}

fn build_violation(
    rule_id: &str,
    artifact: &ArtifactRef,
    trace_id: &str,
    message: &str,
) -> Violation {
    Violation {
        rule_id: rule_id.to_owned(),
        artifact_id: artifact.artifact_id.clone(),
        artifact_path: artifact.artifact_path.clone(),
        bead_id: artifact.bead_id.clone(),
        trace_id: trace_id.to_owned(),
        result: "fail".to_owned(),
        repro_cmd: REPRO_CMD.to_owned(),
        message: message.to_owned(),
    }
}

fn trace_id(rule_id: &str, artifact_id: &str) -> String {
    format!("{rule_id}::{artifact_id}")
}

fn find_unnegated_narrowing_phrase(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    for phrase in NARROWING_PHRASES {
        for (index, _) in lower.match_indices(phrase) {
            let mut start = index.saturating_sub(128);
            while start > 0 && !lower.is_char_boundary(start) {
                start -= 1;
            }
            let context = &lower[start..index];
            if !NEGATION_HINTS.iter().any(|token| context.contains(token)) {
                return Some(phrase);
            }
        }
    }
    None
}

fn print_summary(report: &GovernanceGateReport) {
    if report.all_passed {
        println!(
            "governance gate: PASS ({} events, {} violations)",
            report.events.len(),
            report.violation_count
        );
        return;
    }

    println!(
        "governance gate: FAIL ({} violations / {} events)",
        report.violation_count,
        report.events.len()
    );
    for violation in &report.violations {
        println!(
            "  - {} {} [{}]: {}",
            violation.rule_id, violation.artifact_id, violation.trace_id, violation.message
        );
        println!("    repro_cmd: {}", violation.repro_cmd);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        RULE_CLEAN_ROOM_METHOD_STACK, RULE_NO_MINIMAL_SCOPE, RULE_TOTAL_PARITY,
        find_unnegated_narrowing_phrase, run_governance_gate,
    };

    fn seed_repo(root: &std::path::Path, agents: &str, readme: &str, issues: &[&str]) {
        fs::write(root.join("AGENTS.md"), agents).expect("agents");
        fs::write(root.join("README.md"), readme).expect("readme");
        fs::create_dir_all(root.join(".beads")).expect("beads dir");
        fs::write(root.join(".beads/issues.jsonl"), issues.join("\n")).expect("issues");
    }

    fn compliant_text() -> &'static str {
        "ABSOLUTE AND TOTAL feature/functionality overlap as a drop-in replacement.\n\
         Clean-room implementation only.\n\
         Method stack: alien-artifact-coding, extreme-software-optimization, alien-graveyard, RaptorQ."
    }

    #[test]
    fn narrowing_phrase_detected_when_not_negated() {
        let text = "we will deliver a minimal v1 subset first";
        assert_eq!(find_unnegated_narrowing_phrase(text), Some("minimal v1"));
    }

    #[test]
    fn narrowing_phrase_ignored_when_negated() {
        let text = "no minimal-v1 reductions are allowed";
        assert_eq!(find_unnegated_narrowing_phrase(text), None);
    }

    #[test]
    fn governance_gate_passes_on_compliant_fixture_repo() {
        let dir = tempdir().expect("tempdir");
        let issue = serde_json::json!({
            "id": "bd-test-1",
            "status": "open",
            "title": compliant_text(),
            "description": "",
            "acceptance_criteria": "",
            "notes": ""
        });
        seed_repo(
            dir.path(),
            compliant_text(),
            compliant_text(),
            &[&issue.to_string()],
        );

        let report = run_governance_gate(dir.path()).expect("report");
        assert!(report.all_passed, "{report:?}");
        assert_eq!(report.violation_count, 0);
    }

    #[test]
    fn governance_gate_fails_on_missing_markers_and_scope_narrowing() {
        let dir = tempdir().expect("tempdir");
        let issue = serde_json::json!({
            "id": "bd-test-2",
            "status": "open",
            "title": "minimal v1 subset",
            "description": "partial scope accepted",
            "acceptance_criteria": "",
            "notes": ""
        });
        seed_repo(
            dir.path(),
            "drop-in replacement but missing clean room and stack",
            "clean-room only",
            &[&issue.to_string()],
        );

        let report = run_governance_gate(dir.path()).expect("report");
        assert!(!report.all_passed);
        assert!(report.violation_count >= 3);
        let rule_ids: Vec<&str> = report
            .violations
            .iter()
            .map(|v| v.rule_id.as_str())
            .collect();
        assert!(rule_ids.contains(&RULE_TOTAL_PARITY));
        assert!(rule_ids.contains(&RULE_NO_MINIMAL_SCOPE));
        assert!(rule_ids.contains(&RULE_CLEAN_ROOM_METHOD_STACK));
    }
}
