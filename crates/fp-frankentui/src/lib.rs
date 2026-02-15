#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use fp_conformance::{
    CaseStatus, DecodeProofArtifact, DecodeProofStatus, DifferentialReport, DifferentialResult,
    DriftLevel, E2eConfig, E2eReport, FailureForensicsReport, FixtureOracleSource, ForensicEvent,
    ForensicEventKind, HarnessConfig, HarnessError, NoopHooks, OracleMode, PacketDriftHistoryEntry,
    PacketGateResult, PacketParityReport, SuiteOptions, build_failure_forensics,
    run_differential_by_id, run_differential_suite, run_e2e_suite,
};
use fp_runtime::{ConformalGuard, EvidenceLedger, RuntimeMode, RuntimePolicy, decision_to_card};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const PHASE2C_DIR: &str = "artifacts/phase2c";
const DRIFT_HISTORY_FILE: &str = "drift_history.jsonl";
const DIFFERENTIAL_REPORT_FILE: &str = "differential_report.json";
const CI_DIR: &str = "ci";
const GOVERNANCE_GATE_REPORT_FILE: &str = "governance_gate_report.json";

#[derive(Debug, Error)]
pub enum FtuiError {
    #[error("artifact root is not accessible: {path}")]
    ArtifactRootMissing { path: PathBuf },
    #[error("invalid packet id: {packet_id}")]
    InvalidPacketId { packet_id: String },
    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactIssueKind {
    MissingFile,
    ParseError,
    IoError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactIssue {
    pub path: PathBuf,
    pub kind: ArtifactIssueKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PacketSnapshot {
    pub packet_id: String,
    pub parity_report: Option<PacketParityReport>,
    pub gate_result: Option<PacketGateResult>,
    pub decode_status: Option<DecodeProofStatus>,
    pub mismatch_count: Option<usize>,
    pub differential_report: Option<DifferentialReport>,
    pub differential_validation: Option<DifferentialValidationSummary>,
    pub issues: Vec<ArtifactIssue>,
}

impl PacketSnapshot {
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.parity_report
            .as_ref()
            .is_some_and(PacketParityReport::is_green)
            && self.gate_result.as_ref().is_some_and(|gate| gate.pass)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DriftHistorySnapshot {
    pub entries: Vec<PacketDriftHistoryEntry>,
    pub malformed_lines: usize,
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySnapshot {
    pub mode: RuntimeMode,
    pub fail_closed_unknown_features: bool,
    pub hardened_join_row_cap: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConformalSnapshot {
    pub calibrated: bool,
    pub calibration_count: usize,
    pub quantile_threshold: Option<f64>,
    pub empirical_coverage: f64,
    pub coverage_alert: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionCardSnapshot {
    pub mode: RuntimeMode,
    pub card: String,
    pub evidence_terms_total: usize,
    pub evidence_terms_shown: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionDashboardSnapshot {
    pub total_records: usize,
    pub strict_records: usize,
    pub hardened_records: usize,
    pub policy: PolicySnapshot,
    pub cards: Vec<DecisionCardSnapshot>,
    pub evidence_term_cap: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketTrendSnapshot {
    pub packet_id: String,
    pub samples: usize,
    pub latest_gate_pass: Option<bool>,
    pub latest_failed_cases: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConformanceDashboardSnapshot {
    pub packets: Vec<PacketSnapshot>,
    pub drift_history: DriftHistorySnapshot,
    pub packet_trends: Vec<PacketTrendSnapshot>,
    pub total_packets: usize,
    pub green_packets: usize,
    pub failing_packets: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DifferentialValidationSummary {
    pub total_cases: usize,
    pub strict_critical_violations: usize,
    pub hardened_critical_violations: usize,
    pub hardened_allowlisted_divergence: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DifferentialValidationLogEntry {
    pub packet_id: String,
    pub case_id: String,
    pub mode: RuntimeMode,
    pub trace_id: String,
    pub oracle_source: FixtureOracleSource,
    pub mismatch_class: String,
    pub replay_key: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DifferentialValidationSnapshot {
    pub report: DifferentialReport,
    pub summary: DifferentialValidationSummary,
    pub logs: Vec<DifferentialValidationLogEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrankentuiE2eScenarioKind {
    Golden,
    Regression,
    FailureInjection,
}

impl FrankentuiE2eScenarioKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Golden => "golden",
            Self::Regression => "regression",
            Self::FailureInjection => "failure_injection",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrankentuiE2eScenario {
    pub scenario_id: String,
    pub kind: FrankentuiE2eScenarioKind,
    pub packet_filter: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrankentuiE2eReplayBundleEntry {
    pub scenario_id: String,
    pub packet_id: String,
    pub case_id: String,
    pub mode: RuntimeMode,
    pub trace_id: String,
    pub step_id: String,
    pub decision_action: String,
    pub latency_ms: u64,
    pub replay_key: String,
    pub replay_cmd: String,
    pub mismatch_class: Option<String>,
    pub failure_diagnostics: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrankentuiE2eScenarioReport {
    pub scenario: FrankentuiE2eScenario,
    pub report: E2eReport,
    pub forensics: FailureForensicsReport,
    pub replay_bundles: Vec<FrankentuiE2eReplayBundleEntry>,
}

impl ConformanceDashboardSnapshot {
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.total_packets > 0 && self.failing_packets == 0
    }

    #[must_use]
    pub fn render_plain(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "Conformance packets={} green={} failing={} drift_entries={} malformed_drift_lines={}\n",
            self.total_packets,
            self.green_packets,
            self.failing_packets,
            self.drift_history.entries.len(),
            self.drift_history.malformed_lines
        ));
        for packet in &self.packets {
            out.push_str(&format!(
                "- {} gate={} parity={} mismatches={} issues={}\n",
                packet.packet_id,
                packet.gate_result.as_ref().map(|g| g.pass).unwrap_or(false),
                packet
                    .parity_report
                    .as_ref()
                    .is_some_and(PacketParityReport::is_green),
                packet.mismatch_count.unwrap_or(0),
                packet.issues.len()
            ));
        }
        out
    }
}

#[must_use]
pub fn build_differential_validation_snapshot(
    report: DifferentialReport,
) -> DifferentialValidationSnapshot {
    let summary = summarize_differential_validation(&report);
    let logs = differential_validation_log_entries(&report);
    DifferentialValidationSnapshot {
        report,
        summary,
        logs,
    }
}

pub fn run_differential_validation_suite(
    config: &HarnessConfig,
    options: &SuiteOptions,
) -> Result<DifferentialValidationSnapshot, HarnessError> {
    let report = run_differential_suite(config, options)?;
    Ok(build_differential_validation_snapshot(report))
}

pub fn run_differential_validation_by_packet(
    config: &HarnessConfig,
    packet_id: &str,
    oracle_mode: OracleMode,
) -> Result<DifferentialValidationSnapshot, HarnessError> {
    let report = run_differential_by_id(config, packet_id, oracle_mode)?;
    Ok(build_differential_validation_snapshot(report))
}

#[must_use]
pub fn harness_config_from_repo_root(repo_root: impl AsRef<Path>) -> HarnessConfig {
    let repo_root = repo_root.as_ref().to_path_buf();
    HarnessConfig {
        repo_root: repo_root.clone(),
        oracle_root: repo_root.join("legacy_pandas_code/pandas"),
        fixture_root: repo_root.join("crates/fp-conformance/fixtures"),
        strict_mode: true,
        python_bin: "python3".to_owned(),
        allow_system_pandas_fallback: false,
    }
}

#[must_use]
pub fn default_frankentui_e2e_scenarios() -> Vec<FrankentuiE2eScenario> {
    vec![
        FrankentuiE2eScenario {
            scenario_id: "frankentui_e2e_golden".to_owned(),
            kind: FrankentuiE2eScenarioKind::Golden,
            packet_filter: Some("FP-P2C-001".to_owned()),
        },
        FrankentuiE2eScenario {
            scenario_id: "frankentui_e2e_regression".to_owned(),
            kind: FrankentuiE2eScenarioKind::Regression,
            packet_filter: Some("FP-P2C-001".to_owned()),
        },
        FrankentuiE2eScenario {
            scenario_id: "frankentui_e2e_failure_injection".to_owned(),
            kind: FrankentuiE2eScenarioKind::FailureInjection,
            packet_filter: Some("FP-P2C-001".to_owned()),
        },
    ]
}

pub fn run_frankentui_e2e_matrix(
    config: &HarnessConfig,
    scenarios: &[FrankentuiE2eScenario],
) -> Result<Vec<FrankentuiE2eScenarioReport>, HarnessError> {
    let mut reports = Vec::with_capacity(scenarios.len());
    for scenario in scenarios {
        reports.push(run_frankentui_e2e_scenario(config, scenario)?);
    }
    Ok(reports)
}

pub fn run_frankentui_e2e_scenario(
    config: &HarnessConfig,
    scenario: &FrankentuiE2eScenario,
) -> Result<FrankentuiE2eScenarioReport, HarnessError> {
    let mut hooks = NoopHooks;
    let e2e_config = E2eConfig {
        harness: config.clone(),
        options: SuiteOptions {
            packet_filter: scenario.packet_filter.clone(),
            oracle_mode: OracleMode::FixtureExpected,
        },
        write_artifacts: false,
        enforce_gates: false,
        append_drift_history: false,
        forensic_log_path: None,
    };
    let mut report = run_e2e_suite(&e2e_config, &mut hooks)?;
    if matches!(scenario.kind, FrankentuiE2eScenarioKind::FailureInjection) {
        inject_synthetic_failure_case(&mut report, &scenario.scenario_id);
    }
    let forensics = build_failure_forensics(&report);
    let replay_bundles = build_frankentui_e2e_replay_bundles(&report, &forensics);
    Ok(FrankentuiE2eScenarioReport {
        scenario: scenario.clone(),
        report,
        forensics,
        replay_bundles,
    })
}

#[must_use]
pub fn build_frankentui_e2e_replay_bundles(
    e2e: &E2eReport,
    forensics: &FailureForensicsReport,
) -> Vec<FrankentuiE2eReplayBundleEntry> {
    let mode_index = build_case_mode_index(e2e);
    build_frankentui_e2e_replay_bundles_with_mode_lookup(e2e, forensics, |packet_id, case_id| {
        mode_index.get(&(packet_id, case_id)).copied()
    })
}

fn build_case_mode_index(e2e: &E2eReport) -> HashMap<(&str, &str), RuntimeMode> {
    let mut mode_index = HashMap::<(&str, &str), RuntimeMode>::new();
    for packet in &e2e.packet_reports {
        for result in &packet.results {
            mode_index
                .entry((result.packet_id.as_str(), result.case_id.as_str()))
                .or_insert(result.mode);
        }
    }
    mode_index
}

fn build_frankentui_e2e_replay_bundles_with_mode_lookup<F>(
    e2e: &E2eReport,
    forensics: &FailureForensicsReport,
    mut fallback_mode_lookup: F,
) -> Vec<FrankentuiE2eReplayBundleEntry>
where
    F: FnMut(&str, &str) -> Option<RuntimeMode>,
{
    let mut failure_diagnostics = BTreeMap::<(String, String, String), String>::new();
    for failure in &forensics.failures {
        failure_diagnostics.insert(
            (
                failure.packet_id.clone(),
                failure.case_id.clone(),
                runtime_mode_slug(failure.mode).to_owned(),
            ),
            failure.mismatch_summary.clone(),
        );
    }

    let mut bundles = Vec::new();
    for event in &e2e.forensic_log.events {
        if let ForensicEventKind::CaseEnd {
            scenario_id,
            packet_id,
            case_id,
            trace_id,
            step_id,
            result: _,
            replay_cmd,
            decision_action,
            replay_key,
            mismatch_class,
            status: _,
            evidence_records: _,
            elapsed_us,
            seed: _,
            assertion_path: _,
        } = &event.event
        {
            let Some(mode) = mode_from_replay_key(replay_key)
                .or_else(|| fallback_mode_lookup(packet_id, case_id))
            else {
                continue;
            };

            let diagnostics = failure_diagnostics
                .get(&(
                    packet_id.clone(),
                    case_id.clone(),
                    runtime_mode_slug(mode).to_owned(),
                ))
                .cloned()
                .unwrap_or_else(|| "(none)".to_owned());

            bundles.push(FrankentuiE2eReplayBundleEntry {
                scenario_id: scenario_id.clone(),
                packet_id: packet_id.clone(),
                case_id: case_id.clone(),
                mode,
                trace_id: trace_id.clone(),
                step_id: step_id.clone(),
                decision_action: decision_action.clone(),
                latency_ms: elapsed_us_to_latency_ms(*elapsed_us),
                replay_key: replay_key.clone(),
                replay_cmd: replay_cmd.clone(),
                mismatch_class: mismatch_class.clone(),
                failure_diagnostics: diagnostics,
            });
        }
    }

    bundles.sort_by(|left, right| {
        left.scenario_id
            .cmp(&right.scenario_id)
            .then(left.packet_id.cmp(&right.packet_id))
            .then(left.case_id.cmp(&right.case_id))
            .then(mode_sort_key(left.mode).cmp(&mode_sort_key(right.mode)))
    });
    bundles
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ReplayBundleBuildStats {
    mode_lookup_steps: usize,
}

#[cfg(test)]
fn resolve_mode_by_linear_scan(
    e2e: &E2eReport,
    packet_id: &str,
    case_id: &str,
    stats: &mut ReplayBundleBuildStats,
) -> Option<RuntimeMode> {
    for packet in &e2e.packet_reports {
        for result in &packet.results {
            stats.mode_lookup_steps += 1;
            if result.packet_id == packet_id && result.case_id == case_id {
                return Some(result.mode);
            }
        }
    }
    None
}

#[cfg(test)]
fn build_frankentui_e2e_replay_bundles_baseline_with_stats(
    e2e: &E2eReport,
    forensics: &FailureForensicsReport,
) -> (Vec<FrankentuiE2eReplayBundleEntry>, ReplayBundleBuildStats) {
    let mut stats = ReplayBundleBuildStats::default();
    let bundles = build_frankentui_e2e_replay_bundles_with_mode_lookup(
        e2e,
        forensics,
        |packet_id, case_id| resolve_mode_by_linear_scan(e2e, packet_id, case_id, &mut stats),
    );
    (bundles, stats)
}

#[cfg(test)]
fn build_frankentui_e2e_replay_bundles_optimized_with_stats(
    e2e: &E2eReport,
    forensics: &FailureForensicsReport,
) -> (Vec<FrankentuiE2eReplayBundleEntry>, ReplayBundleBuildStats) {
    let mode_index = build_case_mode_index(e2e);
    let mut stats = ReplayBundleBuildStats::default();
    let bundles = build_frankentui_e2e_replay_bundles_with_mode_lookup(
        e2e,
        forensics,
        |packet_id, case_id| {
            stats.mode_lookup_steps += 1;
            mode_index.get(&(packet_id, case_id)).copied()
        },
    );
    (bundles, stats)
}

fn elapsed_us_to_latency_ms(elapsed_us: u64) -> u64 {
    elapsed_us.saturating_add(999).saturating_div(1_000).max(1)
}

fn mode_from_replay_key(replay_key: &str) -> Option<RuntimeMode> {
    if replay_key.ends_with("/strict") {
        Some(RuntimeMode::Strict)
    } else if replay_key.ends_with("/hardened") {
        Some(RuntimeMode::Hardened)
    } else {
        None
    }
}

fn inject_synthetic_failure_case(report: &mut E2eReport, scenario_id: &str) {
    let mut injected_case: Option<(String, String, RuntimeMode)> = None;

    if let Some(packet_report) = report.packet_reports.first_mut() {
        if let Some(case) = packet_report.results.first_mut() {
            case.status = CaseStatus::Fail;
            case.mismatch = Some(format!("synthetic failure injection for {scenario_id}"));
            case.mismatch_class = Some("synthetic_failure_injection".to_owned());
            if case.trace_id.is_empty() {
                case.trace_id =
                    deterministic_differential_trace_id(&case.packet_id, &case.case_id, case.mode);
            }
            if case.replay_key.is_empty() {
                case.replay_key = deterministic_differential_replay_key(
                    &case.packet_id,
                    &case.case_id,
                    case.mode,
                );
            }
            injected_case = Some((case.packet_id.clone(), case.case_id.clone(), case.mode));
        }

        packet_report.failed = packet_report
            .results
            .iter()
            .filter(|result| matches!(result.status, CaseStatus::Fail))
            .count();
        packet_report.passed = packet_report
            .results
            .len()
            .saturating_sub(packet_report.failed);
    }

    report.total_fixtures = report
        .packet_reports
        .iter()
        .map(|entry| entry.fixture_count)
        .sum();
    report.total_passed = report.packet_reports.iter().map(|entry| entry.passed).sum();
    report.total_failed = report.packet_reports.iter().map(|entry| entry.failed).sum();
    report.gates_pass = false;

    if let Some((packet_id, case_id, mode)) = injected_case {
        if let Some(gate) = report
            .gate_results
            .iter_mut()
            .find(|gate| gate.packet_id == packet_id)
        {
            gate.pass = false;
            if !gate
                .reasons
                .iter()
                .any(|reason| reason.contains("synthetic failure injection"))
            {
                gate.reasons
                    .push(format!("synthetic failure injection: {scenario_id}"));
            }
            match mode {
                RuntimeMode::Strict => gate.strict_failed = gate.strict_failed.max(1),
                RuntimeMode::Hardened => gate.hardened_failed = gate.hardened_failed.max(1),
            }
        }

        for event in &mut report.forensic_log.events {
            if let ForensicEventKind::CaseEnd {
                packet_id: event_packet_id,
                case_id: event_case_id,
                decision_action,
                result,
                mismatch_class,
                status,
                ..
            } = &mut event.event
                && *event_packet_id == packet_id
                && *event_case_id == case_id
            {
                *decision_action = "repair".to_owned();
                *result = "fail".to_owned();
                *mismatch_class = Some("synthetic_failure_injection".to_owned());
                *status = CaseStatus::Fail;
                break;
            }
        }
    }
}

#[must_use]
pub fn summarize_differential_validation(
    report: &DifferentialReport,
) -> DifferentialValidationSummary {
    let mut strict_critical_violations = 0_usize;
    let mut hardened_critical_violations = 0_usize;
    let mut hardened_allowlisted_divergence = 0_usize;

    for result in &report.differential_results {
        let has_critical = result
            .drift_records
            .iter()
            .any(|drift| matches!(drift.level, DriftLevel::Critical));
        let has_non_critical_or_info = result.drift_records.iter().any(|drift| {
            matches!(
                drift.level,
                DriftLevel::NonCritical | DriftLevel::Informational
            )
        });

        match result.mode {
            RuntimeMode::Strict => {
                if has_critical {
                    strict_critical_violations += 1;
                }
            }
            RuntimeMode::Hardened => {
                if has_critical {
                    hardened_critical_violations += 1;
                } else if has_non_critical_or_info {
                    hardened_allowlisted_divergence += 1;
                }
            }
        }
    }

    DifferentialValidationSummary {
        total_cases: report.differential_results.len(),
        strict_critical_violations,
        hardened_critical_violations,
        hardened_allowlisted_divergence,
    }
}

#[must_use]
pub fn differential_validation_log_entries(
    report: &DifferentialReport,
) -> Vec<DifferentialValidationLogEntry> {
    let mut entries = report
        .differential_results
        .iter()
        .map(differential_log_entry_from_result)
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        left.packet_id
            .cmp(&right.packet_id)
            .then(left.case_id.cmp(&right.case_id))
            .then(mode_sort_key(left.mode).cmp(&mode_sort_key(right.mode)))
    });

    entries
}

fn differential_log_entry_from_result(
    result: &DifferentialResult,
) -> DifferentialValidationLogEntry {
    DifferentialValidationLogEntry {
        packet_id: result.packet_id.clone(),
        case_id: result.case_id.clone(),
        mode: result.mode,
        trace_id: if result.trace_id.is_empty() {
            deterministic_differential_trace_id(&result.packet_id, &result.case_id, result.mode)
        } else {
            result.trace_id.clone()
        },
        oracle_source: result.oracle_source,
        mismatch_class: mismatch_class_for_differential_result(result),
        replay_key: if result.replay_key.is_empty() {
            deterministic_differential_replay_key(&result.packet_id, &result.case_id, result.mode)
        } else {
            result.replay_key.clone()
        },
    }
}

fn mismatch_class_for_differential_result(result: &DifferentialResult) -> String {
    result
        .drift_records
        .iter()
        .find(|drift| matches!(drift.level, DriftLevel::Critical))
        .or_else(|| result.drift_records.first())
        .map(|drift| drift.mismatch_class.clone())
        .unwrap_or_else(|| {
            if matches!(result.status, CaseStatus::Fail) {
                "execution_critical".to_owned()
            } else {
                "none".to_owned()
            }
        })
}

fn runtime_mode_slug(mode: RuntimeMode) -> &'static str {
    match mode {
        RuntimeMode::Strict => "strict",
        RuntimeMode::Hardened => "hardened",
    }
}

fn deterministic_differential_trace_id(
    packet_id: &str,
    case_id: &str,
    mode: RuntimeMode,
) -> String {
    format!("{packet_id}:{case_id}:{}", runtime_mode_slug(mode))
}

fn deterministic_differential_replay_key(
    packet_id: &str,
    case_id: &str,
    mode: RuntimeMode,
) -> String {
    format!("{packet_id}/{case_id}/{}", runtime_mode_slug(mode))
}

fn mode_sort_key(mode: RuntimeMode) -> u8 {
    match mode {
        RuntimeMode::Strict => 0,
        RuntimeMode::Hardened => 1,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForensicLogSnapshot {
    pub events: Vec<ForensicEvent>,
    pub malformed_lines: usize,
    pub missing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceGateSnapshot {
    pub path: PathBuf,
    pub all_passed: bool,
    pub violation_count: usize,
    pub generated_unix_ms: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DashboardView {
    Conformance,
    Decision,
    Forensics,
    Policy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FtuiAppState {
    pub active_view: DashboardView,
    pub packet_cursor: usize,
    pub packet_ids: Vec<String>,
}

impl FtuiAppState {
    #[must_use]
    pub fn new(mut packet_ids: Vec<String>) -> Self {
        packet_ids.sort();
        Self {
            active_view: DashboardView::Conformance,
            packet_cursor: 0,
            packet_ids,
        }
    }

    pub fn cycle_next_view(&mut self) {
        self.active_view = match self.active_view {
            DashboardView::Conformance => DashboardView::Decision,
            DashboardView::Decision => DashboardView::Forensics,
            DashboardView::Forensics => DashboardView::Policy,
            DashboardView::Policy => DashboardView::Conformance,
        };
    }

    pub fn select_next_packet(&mut self) {
        if self.packet_ids.is_empty() {
            self.packet_cursor = 0;
            return;
        }
        self.packet_cursor = (self.packet_cursor + 1) % self.packet_ids.len();
    }

    #[must_use]
    pub fn selected_packet(&self) -> Option<&str> {
        self.packet_ids.get(self.packet_cursor).map(String::as_str)
    }
}

pub trait FtuiDataSource {
    fn discover_packet_ids(&self) -> Result<Vec<String>, FtuiError>;
    fn load_packet_snapshot(&self, packet_id: &str) -> Result<PacketSnapshot, FtuiError>;
    fn load_drift_history(&self) -> Result<DriftHistorySnapshot, FtuiError>;
    fn load_conformance_dashboard(&self) -> Result<ConformanceDashboardSnapshot, FtuiError>;
    fn load_forensic_log(&self, path: &Path) -> Result<ForensicLogSnapshot, FtuiError>;
    fn load_governance_gate_snapshot(&self) -> Result<Option<GovernanceGateSnapshot>, FtuiError>;
}

#[derive(Debug, Clone)]
pub struct FsFtuiDataSource {
    phase2c_root: PathBuf,
}

impl FsFtuiDataSource {
    #[must_use]
    pub fn from_repo_root(repo_root: impl AsRef<Path>) -> Self {
        Self {
            phase2c_root: repo_root.as_ref().join(PHASE2C_DIR),
        }
    }

    #[must_use]
    pub fn from_phase2c_root(phase2c_root: impl Into<PathBuf>) -> Self {
        Self {
            phase2c_root: phase2c_root.into(),
        }
    }

    #[must_use]
    pub fn phase2c_root(&self) -> &Path {
        &self.phase2c_root
    }

    fn ensure_root_exists(&self) -> Result<(), FtuiError> {
        if self.phase2c_root.is_dir() {
            Ok(())
        } else {
            Err(FtuiError::ArtifactRootMissing {
                path: self.phase2c_root.clone(),
            })
        }
    }
}

impl FtuiDataSource for FsFtuiDataSource {
    fn discover_packet_ids(&self) -> Result<Vec<String>, FtuiError> {
        self.ensure_root_exists()?;
        let mut packet_ids = Vec::new();
        let entries = fs::read_dir(&self.phase2c_root).map_err(|source| FtuiError::Io {
            path: self.phase2c_root.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| FtuiError::Io {
                path: self.phase2c_root.clone(),
                source,
            })?;
            if !entry
                .file_type()
                .map_err(|source| FtuiError::Io {
                    path: entry.path(),
                    source,
                })?
                .is_dir()
            {
                continue;
            }
            let name = entry.file_name();
            let packet_id = name.to_string_lossy();
            if is_valid_packet_id(packet_id.as_ref()) {
                packet_ids.push(packet_id.into_owned());
            }
        }
        packet_ids.sort();
        Ok(packet_ids)
    }

    fn load_packet_snapshot(&self, packet_id: &str) -> Result<PacketSnapshot, FtuiError> {
        self.ensure_root_exists()?;
        if !is_valid_packet_id(packet_id) {
            return Err(FtuiError::InvalidPacketId {
                packet_id: packet_id.to_owned(),
            });
        }

        let packet_root = self.phase2c_root.join(packet_id);
        let mut issues = Vec::new();

        let parity_report: Option<PacketParityReport> =
            read_json_optional(&packet_root.join("parity_report.json"), &mut issues);
        let gate_result: Option<PacketGateResult> =
            read_json_optional(&packet_root.join("parity_gate_result.json"), &mut issues);
        let decode_status = read_json_optional::<DecodeProofArtifact>(
            &packet_root.join("parity_report.decode_proof.json"),
            &mut issues,
        )
        .map(|artifact| artifact.status);
        let mismatch_count = read_json_optional::<MismatchCorpus>(
            &packet_root.join("parity_mismatch_corpus.json"),
            &mut issues,
        )
        .map(|corpus| corpus.mismatch_count);
        let differential_report = read_json_if_present::<DifferentialReport>(
            &packet_root.join(DIFFERENTIAL_REPORT_FILE),
            &mut issues,
        );
        let differential_validation = differential_report
            .as_ref()
            .map(summarize_differential_validation);

        Ok(PacketSnapshot {
            packet_id: packet_id.to_owned(),
            parity_report,
            gate_result,
            decode_status,
            mismatch_count,
            differential_report,
            differential_validation,
            issues,
        })
    }

    fn load_drift_history(&self) -> Result<DriftHistorySnapshot, FtuiError> {
        self.ensure_root_exists()?;
        let path = self.phase2c_root.join(DRIFT_HISTORY_FILE);
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DriftHistorySnapshot {
                    entries: Vec::new(),
                    malformed_lines: 0,
                    missing: true,
                });
            }
            Err(source) => return Err(FtuiError::Io { path, source }),
        };

        let mut entries = Vec::new();
        let mut malformed_lines = 0;

        for line in BufReader::new(file).lines() {
            let line = line.map_err(|source| FtuiError::Io {
                path: path.clone(),
                source,
            })?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<PacketDriftHistoryEntry>(&line) {
                Ok(entry) => entries.push(entry),
                Err(_) => malformed_lines += 1,
            }
        }

        Ok(DriftHistorySnapshot {
            entries,
            malformed_lines,
            missing: false,
        })
    }

    fn load_conformance_dashboard(&self) -> Result<ConformanceDashboardSnapshot, FtuiError> {
        let packet_ids = self.discover_packet_ids()?;
        let packets = packet_ids
            .iter()
            .map(|packet_id| self.load_packet_snapshot(packet_id))
            .collect::<Result<Vec<_>, _>>()?;
        let drift_history = self.load_drift_history()?;

        let green_packets = packets.iter().filter(|packet| packet.is_green()).count();
        let total_packets = packets.len();
        let failing_packets = total_packets.saturating_sub(green_packets);

        let packet_trends = packets
            .iter()
            .map(|packet| {
                let mut samples = 0_usize;
                let mut latest: Option<&PacketDriftHistoryEntry> = None;
                for entry in &drift_history.entries {
                    if entry.packet_id == packet.packet_id {
                        samples += 1;
                        if latest.is_none_or(|current| entry.ts_unix_ms >= current.ts_unix_ms) {
                            latest = Some(entry);
                        }
                    }
                }
                PacketTrendSnapshot {
                    packet_id: packet.packet_id.clone(),
                    samples,
                    latest_gate_pass: latest.map(|entry| entry.gate_pass),
                    latest_failed_cases: latest.map(|entry| entry.failed),
                }
            })
            .collect();

        Ok(ConformanceDashboardSnapshot {
            packets,
            drift_history,
            packet_trends,
            total_packets,
            green_packets,
            failing_packets,
        })
    }

    fn load_forensic_log(&self, path: &Path) -> Result<ForensicLogSnapshot, FtuiError> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ForensicLogSnapshot {
                    events: Vec::new(),
                    malformed_lines: 0,
                    missing: true,
                });
            }
            Err(source) => {
                return Err(FtuiError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };

        let mut events = Vec::new();
        let mut malformed_lines = 0;
        for line in BufReader::new(file).lines() {
            let line = line.map_err(|source| FtuiError::Io {
                path: path.to_path_buf(),
                source,
            })?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ForensicEvent>(&line) {
                Ok(event) => events.push(event),
                Err(_) => malformed_lines += 1,
            }
        }

        Ok(ForensicLogSnapshot {
            events,
            malformed_lines,
            missing: false,
        })
    }

    fn load_governance_gate_snapshot(&self) -> Result<Option<GovernanceGateSnapshot>, FtuiError> {
        let Some(artifacts_root) = self.phase2c_root.parent() else {
            return Ok(None);
        };
        let path = artifacts_root
            .join(CI_DIR)
            .join(GOVERNANCE_GATE_REPORT_FILE);
        let payload = match fs::read_to_string(&path) {
            Ok(payload) => payload,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(FtuiError::Io { path, source }),
        };

        let report: GovernanceGateReport =
            serde_json::from_str(&payload).map_err(|source| FtuiError::Io {
                path: path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
            })?;

        Ok(Some(GovernanceGateSnapshot {
            path,
            all_passed: report.all_passed,
            violation_count: report.violation_count,
            generated_unix_ms: report.generated_unix_ms,
        }))
    }
}

#[derive(Debug, Deserialize)]
struct MismatchCorpus {
    mismatch_count: usize,
}

#[derive(Debug, Deserialize)]
struct GovernanceGateReport {
    generated_unix_ms: u128,
    all_passed: bool,
    violation_count: usize,
}

fn read_json_optional<T: DeserializeOwned>(
    path: &Path,
    issues: &mut Vec<ArtifactIssue>,
) -> Option<T> {
    match fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<T>(&content) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                issues.push(ArtifactIssue {
                    path: path.to_path_buf(),
                    kind: ArtifactIssueKind::ParseError,
                    detail: error.to_string(),
                });
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            issues.push(ArtifactIssue {
                path: path.to_path_buf(),
                kind: ArtifactIssueKind::MissingFile,
                detail: "file not found".to_owned(),
            });
            None
        }
        Err(error) => {
            issues.push(ArtifactIssue {
                path: path.to_path_buf(),
                kind: ArtifactIssueKind::IoError,
                detail: error.to_string(),
            });
            None
        }
    }
}

fn read_json_if_present<T: DeserializeOwned>(
    path: &Path,
    issues: &mut Vec<ArtifactIssue>,
) -> Option<T> {
    match fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str::<T>(&content) {
            Ok(parsed) => Some(parsed),
            Err(error) => {
                issues.push(ArtifactIssue {
                    path: path.to_path_buf(),
                    kind: ArtifactIssueKind::ParseError,
                    detail: error.to_string(),
                });
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            issues.push(ArtifactIssue {
                path: path.to_path_buf(),
                kind: ArtifactIssueKind::IoError,
                detail: error.to_string(),
            });
            None
        }
    }
}

#[must_use]
pub fn is_valid_packet_id(packet_id: &str) -> bool {
    let Some(rest) = packet_id.strip_prefix("FP-P2C-") else {
        return false;
    };
    rest.len() == 3 && rest.as_bytes().iter().all(u8::is_ascii_digit)
}

#[must_use]
pub fn summarize_conformal_guard(guard: &ConformalGuard) -> ConformalSnapshot {
    ConformalSnapshot {
        calibrated: guard.is_calibrated(),
        calibration_count: guard.calibration_count(),
        quantile_threshold: guard.conformal_quantile(),
        empirical_coverage: guard.empirical_coverage(),
        coverage_alert: guard.coverage_alert(),
    }
}

#[must_use]
pub fn summarize_decision_dashboard(
    ledger: &EvidenceLedger,
    policy: &RuntimePolicy,
    evidence_term_cap: usize,
) -> DecisionDashboardSnapshot {
    let mut strict_records = 0;
    let mut hardened_records = 0;
    let mut cards = Vec::with_capacity(ledger.records().len());

    for record in ledger.records() {
        match record.mode {
            RuntimeMode::Strict => strict_records += 1,
            RuntimeMode::Hardened => hardened_records += 1,
        }
        let evidence_terms_total = record.evidence.len();
        cards.push(DecisionCardSnapshot {
            mode: record.mode,
            card: decision_to_card(record).render_plain(),
            evidence_terms_total,
            evidence_terms_shown: evidence_terms_total.min(evidence_term_cap),
        });
    }

    DecisionDashboardSnapshot {
        total_records: strict_records + hardened_records,
        strict_records,
        hardened_records,
        policy: PolicySnapshot {
            mode: policy.mode,
            fail_closed_unknown_features: policy.fail_closed_unknown_features,
            hardened_join_row_cap: policy.hardened_join_row_cap,
        },
        cards,
        evidence_term_cap,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ArtifactIssueKind, DashboardView, FsFtuiDataSource, FtuiAppState, FtuiDataSource,
        build_frankentui_e2e_replay_bundles_baseline_with_stats,
        build_frankentui_e2e_replay_bundles_optimized_with_stats, default_frankentui_e2e_scenarios,
        differential_validation_log_entries, is_valid_packet_id,
        run_differential_validation_by_packet, run_frankentui_e2e_scenario,
        summarize_decision_dashboard, summarize_differential_validation,
    };
    use fp_conformance::{
        CaseStatus, ComparisonCategory, DifferentialReport, DifferentialResult, DriftLevel,
        DriftRecord, E2eReport, FixtureOperation, FixtureOracleSource, ForensicEventKind,
        HarnessConfig, OracleMode, PacketDriftHistoryEntry,
    };
    use fp_runtime::{EvidenceLedger, RuntimePolicy};
    use proptest::prelude::*;
    use serde_json::json;
    use std::fs;
    use std::hint::black_box;
    use std::time::Instant;
    use tempfile::tempdir;

    fn emit_test_log(
        packet_id: &str,
        case_id: &str,
        mode: &str,
        seed: u64,
        assertion_path: &str,
        result: &str,
    ) {
        let trace_id = format!("{packet_id}::{case_id}::seed-{seed}");
        println!(
            "{}",
            json!({
                "packet_id": packet_id,
                "case_id": case_id,
                "mode": mode,
                "seed": seed,
                "trace_id": trace_id,
                "assertion_path": assertion_path,
                "result": result
            })
        );
    }

    fn emit_differential_validation_log(entry: &super::DifferentialValidationLogEntry) {
        println!(
            "{}",
            json!({
                "packet_id": entry.packet_id,
                "case_id": entry.case_id,
                "mode": entry.mode,
                "trace_id": entry.trace_id,
                "oracle_source": entry.oracle_source,
                "mismatch_class": entry.mismatch_class,
                "replay_key": entry.replay_key
            })
        );
    }

    fn emit_e2e_replay_log(entry: &super::FrankentuiE2eReplayBundleEntry) {
        println!(
            "{}",
            json!({
                "scenario_id": entry.scenario_id,
                "packet_id": entry.packet_id,
                "mode": entry.mode,
                "trace_id": entry.trace_id,
                "step_id": entry.step_id,
                "decision_action": entry.decision_action,
                "latency_ms": entry.latency_ms,
                "failure_diagnostics": entry.failure_diagnostics,
                "replay_key": entry.replay_key
            })
        );
    }

    fn force_replay_bundle_mode_fallback(report: &mut E2eReport) {
        for event in &mut report.forensic_log.events {
            if let ForensicEventKind::CaseEnd {
                packet_id,
                case_id,
                replay_key,
                ..
            } = &mut event.event
            {
                *replay_key = format!("{packet_id}/{case_id}/mode_unknown");
            }
        }
    }

    fn amplify_replay_bundle_mode_lookups(report: &mut E2eReport, repeats: usize) {
        let Some(packet_report) = report.packet_reports.first_mut() else {
            return;
        };
        let base_results = packet_report.results.clone();
        let base_case_end_events: Vec<_> = report
            .forensic_log
            .events
            .iter()
            .filter(|event| matches!(event.event, ForensicEventKind::CaseEnd { .. }))
            .cloned()
            .collect();

        for repeat in 0..repeats {
            for template in &base_results {
                let mut result = template.clone();
                let new_case_id = format!("{}::amp{repeat}", result.case_id);
                result.case_id = new_case_id.clone();
                result.trace_id = format!("{}::amp{repeat}", result.trace_id);
                result.replay_key = format!("{}/{new_case_id}/mode_unknown", result.packet_id);
                packet_report.results.push(result);
            }
            for template in &base_case_end_events {
                let mut event = template.clone();
                if let ForensicEventKind::CaseEnd {
                    packet_id,
                    case_id,
                    trace_id,
                    step_id,
                    replay_key,
                    ..
                } = &mut event.event
                {
                    let new_case_id = format!("{case_id}::amp{repeat}");
                    *case_id = new_case_id.clone();
                    *trace_id = format!("{trace_id}::amp{repeat}");
                    *step_id = format!("{step_id}::amp{repeat}");
                    *replay_key = format!("{packet_id}/{new_case_id}/mode_unknown");
                }
                report.forensic_log.events.push(event);
            }
        }
    }

    fn quantile_from_sorted(samples: &[u128], pct: usize) -> u128 {
        let len = samples.len();
        assert!(len > 0);
        let idx = (len.saturating_sub(1) * pct) / 100;
        samples[idx]
    }

    fn latency_quantiles(mut samples_ns: Vec<u128>) -> (u128, u128, u128) {
        samples_ns.sort_unstable();
        (
            quantile_from_sorted(&samples_ns, 50),
            quantile_from_sorted(&samples_ns, 95),
            quantile_from_sorted(&samples_ns, 99),
        )
    }

    #[test]
    fn packet_id_validation_accepts_expected_shape() {
        emit_test_log(
            "FRANKENTUI-E",
            "packet_id_validation_accepts_expected_shape",
            "n/a",
            1,
            "is_valid_packet_id static examples",
            "pass",
        );
        assert!(is_valid_packet_id("FP-P2C-001"));
        assert!(is_valid_packet_id("FP-P2C-999"));
        assert!(!is_valid_packet_id("FP-P2C-12"));
        assert!(!is_valid_packet_id("FP-P2C-1000"));
        assert!(!is_valid_packet_id("fp-p2c-001"));
        assert!(!is_valid_packet_id("../FP-P2C-001"));
    }

    #[test]
    fn discover_packet_ids_filters_non_packet_entries() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("artifacts/phase2c");
        fs::create_dir_all(root.join("FP-P2C-001")).expect("packet dir");
        fs::create_dir_all(root.join("scratch")).expect("scratch dir");
        fs::write(root.join("drift_history.jsonl"), "").expect("history file");
        let source = FsFtuiDataSource::from_repo_root(dir.path());
        let ids = source.discover_packet_ids().expect("discover ids");
        assert_eq!(ids, vec!["FP-P2C-001".to_owned()]);
    }

    #[test]
    fn load_drift_history_handles_missing_file() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("artifacts/phase2c");
        fs::create_dir_all(&root).expect("phase2c root");
        let source = FsFtuiDataSource::from_repo_root(dir.path());
        let history = source.load_drift_history().expect("load history");
        assert!(history.missing);
        assert_eq!(history.malformed_lines, 0);
        assert!(history.entries.is_empty());
    }

    #[test]
    fn load_drift_history_skips_malformed_lines() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("artifacts/phase2c");
        fs::create_dir_all(&root).expect("phase2c root");
        let entry = PacketDriftHistoryEntry {
            ts_unix_ms: 1,
            packet_id: "FP-P2C-001".to_owned(),
            suite: "phase2c_packets".to_owned(),
            fixture_count: 2,
            passed: 2,
            failed: 0,
            strict_failed: 0,
            hardened_failed: 0,
            gate_pass: true,
            report_hash: "sha256:abc".to_owned(),
        };
        let line = serde_json::to_string(&entry).expect("serialize entry");
        let history_path = root.join("drift_history.jsonl");
        fs::write(&history_path, format!("{line}\nnot-json\n{{\"partial\":"))
            .expect("write history");

        let source = FsFtuiDataSource::from_repo_root(dir.path());
        let history = source.load_drift_history().expect("load history");
        assert!(!history.missing);
        assert_eq!(history.entries.len(), 1);
        assert_eq!(history.malformed_lines, 2);
    }

    #[test]
    fn packet_snapshot_collects_missing_and_parse_issues() {
        let dir = tempdir().expect("tempdir");
        let packet_root = dir.path().join("artifacts/phase2c/FP-P2C-001");
        fs::create_dir_all(&packet_root).expect("packet root");
        fs::write(packet_root.join("parity_report.json"), "{\"bad\":").expect("bad json");
        fs::write(
            packet_root.join("parity_mismatch_corpus.json"),
            "{\"packet_id\":\"FP-P2C-001\",\"mismatch_count\":7,\"mismatches\":[]}",
        )
        .expect("mismatch corpus");
        fs::write(
            packet_root.join("parity_report.decode_proof.json"),
            "{\"packet_id\":\"FP-P2C-001\",\"decode_proofs\":[],\"status\":\"recovered\"}",
        )
        .expect("decode proof");

        let source = FsFtuiDataSource::from_repo_root(dir.path());
        let snapshot = source
            .load_packet_snapshot("FP-P2C-001")
            .expect("packet snapshot");

        assert!(snapshot.parity_report.is_none());
        assert!(snapshot.gate_result.is_none());
        assert_eq!(snapshot.mismatch_count, Some(7));
        assert!(snapshot.decode_status.is_some());
        assert!(
            snapshot
                .issues
                .iter()
                .any(|i| i.kind == ArtifactIssueKind::ParseError),
            "expected parse issue"
        );
        assert!(
            snapshot
                .issues
                .iter()
                .any(|i| i.kind == ArtifactIssueKind::MissingFile),
            "expected missing-file issue"
        );
        assert!(snapshot.differential_report.is_none());
        assert!(snapshot.differential_validation.is_none());
    }

    #[test]
    fn packet_snapshot_flags_truncated_differential_report() {
        let dir = tempdir().expect("tempdir");
        let packet_root = dir.path().join("artifacts/phase2c/FP-P2C-001");
        fs::create_dir_all(&packet_root).expect("packet root");
        fs::write(
            packet_root.join("parity_report.json"),
            json!({
                "suite": "phase2c_packets",
                "packet_id": "FP-P2C-001",
                "oracle_present": true,
                "fixture_count": 1,
                "passed": 1,
                "failed": 0,
                "results": []
            })
            .to_string(),
        )
        .expect("parity");
        fs::write(
            packet_root.join("parity_gate_result.json"),
            json!({
                "packet_id": "FP-P2C-001",
                "pass": true,
                "fixture_count": 1,
                "strict_total": 1,
                "strict_failed": 0,
                "hardened_total": 0,
                "hardened_failed": 0,
                "reasons": []
            })
            .to_string(),
        )
        .expect("gate");
        fs::write(
            packet_root.join("parity_mismatch_corpus.json"),
            "{\"packet_id\":\"FP-P2C-001\",\"mismatch_count\":0,\"mismatches\":[]}",
        )
        .expect("mismatch");
        fs::write(
            packet_root.join("parity_report.decode_proof.json"),
            "{\"packet_id\":\"FP-P2C-001\",\"decode_proofs\":[],\"status\":\"recovered\"}",
        )
        .expect("decode proof");
        fs::write(packet_root.join("differential_report.json"), "{\"report\":")
            .expect("truncated diff");

        let source = FsFtuiDataSource::from_repo_root(dir.path());
        let snapshot = source
            .load_packet_snapshot("FP-P2C-001")
            .expect("packet snapshot");

        assert!(snapshot.differential_report.is_none());
        assert!(snapshot.differential_validation.is_none());
        assert!(
            snapshot.issues.iter().any(|issue| {
                issue.kind == ArtifactIssueKind::ParseError
                    && issue.path.ends_with("differential_report.json")
            }),
            "expected parse error for differential report"
        );
    }

    #[test]
    fn differential_validation_separates_strict_vs_hardened_taxonomy() {
        let report = DifferentialReport {
            report: fp_conformance::PacketParityReport {
                suite: "phase2c_packets:FP-P2C-001".to_owned(),
                packet_id: Some("FP-P2C-001".to_owned()),
                oracle_present: true,
                fixture_count: 3,
                passed: 1,
                failed: 2,
                results: Vec::new(),
            },
            differential_results: vec![
                DifferentialResult {
                    case_id: "strict_critical".to_owned(),
                    packet_id: "FP-P2C-001".to_owned(),
                    operation: FixtureOperation::SeriesAdd,
                    mode: fp_runtime::RuntimeMode::Strict,
                    replay_key: String::new(),
                    trace_id: String::new(),
                    oracle_source: FixtureOracleSource::Fixture,
                    status: CaseStatus::Fail,
                    drift_records: vec![DriftRecord {
                        category: ComparisonCategory::Value,
                        level: DriftLevel::Critical,
                        mismatch_class: "value_critical".to_owned(),
                        location: "series.values[0]".to_owned(),
                        message: "forced strict drift".to_owned(),
                    }],
                    evidence_records: 0,
                },
                DifferentialResult {
                    case_id: "hardened_allowlisted".to_owned(),
                    packet_id: "FP-P2C-001".to_owned(),
                    operation: FixtureOperation::SeriesAdd,
                    mode: fp_runtime::RuntimeMode::Hardened,
                    replay_key: String::new(),
                    trace_id: String::new(),
                    oracle_source: FixtureOracleSource::LiveLegacyPandas,
                    status: CaseStatus::Pass,
                    drift_records: vec![DriftRecord {
                        category: ComparisonCategory::Index,
                        level: DriftLevel::NonCritical,
                        mismatch_class: "index_non_critical".to_owned(),
                        location: "series.index".to_owned(),
                        message: "forced hardened allowlist drift".to_owned(),
                    }],
                    evidence_records: 0,
                },
                DifferentialResult {
                    case_id: "hardened_critical".to_owned(),
                    packet_id: "FP-P2C-001".to_owned(),
                    operation: FixtureOperation::SeriesAdd,
                    mode: fp_runtime::RuntimeMode::Hardened,
                    replay_key: String::new(),
                    trace_id: String::new(),
                    oracle_source: FixtureOracleSource::Fixture,
                    status: CaseStatus::Fail,
                    drift_records: vec![DriftRecord {
                        category: ComparisonCategory::Shape,
                        level: DriftLevel::Critical,
                        mismatch_class: "shape_critical".to_owned(),
                        location: "series.len".to_owned(),
                        message: "forced hardened hard failure".to_owned(),
                    }],
                    evidence_records: 0,
                },
            ],
            drift_summary: fp_conformance::DriftSummary {
                total_drift_records: 3,
                critical_count: 2,
                non_critical_count: 1,
                informational_count: 0,
                categories: Vec::new(),
            },
        };

        let summary = summarize_differential_validation(&report);
        let entries = differential_validation_log_entries(&report);

        assert_eq!(summary.total_cases, 3);
        assert_eq!(summary.strict_critical_violations, 1);
        assert_eq!(summary.hardened_allowlisted_divergence, 1);
        assert_eq!(summary.hardened_critical_violations, 1);
        assert_eq!(entries.len(), 3);
        for entry in &entries {
            emit_differential_validation_log(entry);
        }

        let strict_entry = entries
            .iter()
            .find(|entry| entry.case_id == "strict_critical")
            .expect("strict entry");
        assert_eq!(strict_entry.trace_id, "FP-P2C-001:strict_critical:strict");
        assert_eq!(strict_entry.replay_key, "FP-P2C-001/strict_critical/strict");
        assert_eq!(strict_entry.mismatch_class, "value_critical");
    }

    #[test]
    fn differential_validation_workflow_is_deterministic_for_packet() {
        let cfg = HarnessConfig::default_paths();
        let first =
            run_differential_validation_by_packet(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
                .expect("first differential validation");
        let second =
            run_differential_validation_by_packet(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
                .expect("second differential validation");

        assert_eq!(first.logs, second.logs);
        assert_eq!(first.summary, second.summary);
        assert!(!first.logs.is_empty());
        for entry in &first.logs {
            assert!(!entry.trace_id.is_empty());
            assert!(!entry.replay_key.is_empty());
            assert!(!entry.mismatch_class.is_empty());
            emit_differential_validation_log(entry);
        }
    }

    #[test]
    fn e2e_scenario_matrix_covers_golden_regression_and_failure_injection() {
        let scenarios = default_frankentui_e2e_scenarios();
        assert_eq!(scenarios.len(), 3);
        assert!(
            scenarios
                .iter()
                .any(|scenario| matches!(scenario.kind, super::FrankentuiE2eScenarioKind::Golden))
        );
        assert!(
            scenarios.iter().any(|scenario| matches!(
                scenario.kind,
                super::FrankentuiE2eScenarioKind::Regression
            ))
        );
        assert!(scenarios.iter().any(|scenario| matches!(
            scenario.kind,
            super::FrankentuiE2eScenarioKind::FailureInjection
        )));
    }

    #[test]
    fn e2e_regression_scenario_emits_replay_bundle_fields() {
        let cfg = HarnessConfig::default_paths();
        let scenario = super::FrankentuiE2eScenario {
            scenario_id: "frankentui_e2e_regression".to_owned(),
            kind: super::FrankentuiE2eScenarioKind::Regression,
            packet_filter: Some("FP-P2C-001".to_owned()),
        };
        let report = run_frankentui_e2e_scenario(&cfg, &scenario).expect("run e2e scenario");

        assert!(!report.replay_bundles.is_empty());
        for entry in &report.replay_bundles {
            assert!(!entry.scenario_id.is_empty());
            assert!(!entry.packet_id.is_empty());
            assert!(!entry.trace_id.is_empty());
            assert!(!entry.step_id.is_empty());
            assert!(!entry.decision_action.is_empty());
            assert!(!entry.replay_key.is_empty());
            assert!(!entry.replay_cmd.is_empty());
            assert!(entry.latency_ms >= 1);
            emit_e2e_replay_log(entry);
        }
    }

    #[test]
    fn e2e_failure_injection_scenario_produces_repair_forensics_bundle() {
        let cfg = HarnessConfig::default_paths();
        let scenario = super::FrankentuiE2eScenario {
            scenario_id: "frankentui_e2e_failure_injection".to_owned(),
            kind: super::FrankentuiE2eScenarioKind::FailureInjection,
            packet_filter: Some("FP-P2C-001".to_owned()),
        };
        let report = run_frankentui_e2e_scenario(&cfg, &scenario).expect("run e2e scenario");

        assert!(report.report.total_failed >= 1);
        assert!(!report.report.gates_pass);
        assert!(!report.forensics.failures.is_empty());
        let injected = report
            .replay_bundles
            .iter()
            .find(|entry| entry.mismatch_class.as_deref() == Some("synthetic_failure_injection"))
            .expect("synthetic failure replay bundle");
        assert_eq!(injected.decision_action, "repair");
        assert!(
            injected
                .failure_diagnostics
                .contains("synthetic failure injection"),
            "expected synthetic failure diagnostic"
        );
        emit_e2e_replay_log(injected);
    }

    #[test]
    fn e2e_replay_bundle_optimized_path_is_isomorphic_to_baseline() {
        let cfg = HarnessConfig::default_paths();
        let scenario = super::FrankentuiE2eScenario {
            scenario_id: "frankentui_e2e_profile_probe".to_owned(),
            kind: super::FrankentuiE2eScenarioKind::Regression,
            packet_filter: Some("FP-P2C-001".to_owned()),
        };
        let mut scenario_report =
            run_frankentui_e2e_scenario(&cfg, &scenario).expect("run e2e profile probe");
        force_replay_bundle_mode_fallback(&mut scenario_report.report);
        amplify_replay_bundle_mode_lookups(&mut scenario_report.report, 512);

        let (baseline, baseline_stats) = build_frankentui_e2e_replay_bundles_baseline_with_stats(
            &scenario_report.report,
            &scenario_report.forensics,
        );
        let (optimized, optimized_stats) = build_frankentui_e2e_replay_bundles_optimized_with_stats(
            &scenario_report.report,
            &scenario_report.forensics,
        );

        assert_eq!(optimized, baseline);
        assert!(
            baseline_stats.mode_lookup_steps > optimized_stats.mode_lookup_steps,
            "expected indexed lookup to reduce fallback steps"
        );
    }

    #[test]
    fn e2e_replay_bundle_profile_snapshot_reports_lookup_delta() {
        const ITERATIONS: usize = 64;
        let cfg = HarnessConfig::default_paths();
        let scenario = super::FrankentuiE2eScenario {
            scenario_id: "frankentui_e2e_profile_snapshot".to_owned(),
            kind: super::FrankentuiE2eScenarioKind::Regression,
            packet_filter: Some("FP-P2C-001".to_owned()),
        };
        let mut scenario_report =
            run_frankentui_e2e_scenario(&cfg, &scenario).expect("run e2e profile snapshot");
        force_replay_bundle_mode_fallback(&mut scenario_report.report);
        amplify_replay_bundle_mode_lookups(&mut scenario_report.report, 512);

        let mut baseline_ns = Vec::with_capacity(ITERATIONS);
        let mut optimized_ns = Vec::with_capacity(ITERATIONS);
        let mut baseline_mode_lookup_steps_total = 0_usize;
        let mut optimized_mode_lookup_steps_total = 0_usize;

        for _ in 0..ITERATIONS {
            let baseline_start = Instant::now();
            let (baseline, baseline_stats) =
                build_frankentui_e2e_replay_bundles_baseline_with_stats(
                    &scenario_report.report,
                    &scenario_report.forensics,
                );
            baseline_ns.push(baseline_start.elapsed().as_nanos());
            baseline_mode_lookup_steps_total += baseline_stats.mode_lookup_steps;

            let optimized_start = Instant::now();
            let (optimized, optimized_stats) =
                build_frankentui_e2e_replay_bundles_optimized_with_stats(
                    &scenario_report.report,
                    &scenario_report.forensics,
                );
            optimized_ns.push(optimized_start.elapsed().as_nanos());
            optimized_mode_lookup_steps_total += optimized_stats.mode_lookup_steps;

            assert_eq!(optimized, baseline);
            black_box(optimized.len());
        }

        let (baseline_p50_ns, baseline_p95_ns, baseline_p99_ns) = latency_quantiles(baseline_ns);
        let (optimized_p50_ns, optimized_p95_ns, optimized_p99_ns) =
            latency_quantiles(optimized_ns);
        assert!(baseline_mode_lookup_steps_total > optimized_mode_lookup_steps_total);

        println!(
            "frankentui_e2e_replay_bundle_profile_snapshot baseline_ns[p50={baseline_p50_ns},p95={baseline_p95_ns},p99={baseline_p99_ns}] optimized_ns[p50={optimized_p50_ns},p95={optimized_p95_ns},p99={optimized_p99_ns}] mode_lookup_steps_baseline={baseline_mode_lookup_steps_total} mode_lookup_steps_optimized={optimized_mode_lookup_steps_total}"
        );
    }

    #[test]
    fn decision_summary_counts_modes_and_caps_evidence() {
        let mut ledger = EvidenceLedger::new();
        let strict = RuntimePolicy::strict();
        let hardened = RuntimePolicy::hardened(Some(8));
        strict.decide_unknown_feature("feature_x", "unsupported", &mut ledger);
        hardened.decide_join_admission(42, &mut ledger);

        let summary = summarize_decision_dashboard(&ledger, &hardened, 1);
        emit_test_log(
            "FRANKENTUI-E",
            "decision_summary_counts_modes_and_caps_evidence",
            "mixed",
            8,
            "summary mode counters and evidence cap",
            "pass",
        );
        assert_eq!(summary.total_records, 2);
        assert_eq!(summary.strict_records, 1);
        assert_eq!(summary.hardened_records, 1);
        assert!(
            summary
                .cards
                .iter()
                .all(|card| card.evidence_terms_shown == 1)
        );
        assert_eq!(summary.policy.hardened_join_row_cap, Some(8));
    }

    #[test]
    fn conformance_dashboard_aggregates_packet_and_trend_state() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("artifacts/phase2c");
        let packet_a = root.join("FP-P2C-001");
        let packet_b = root.join("FP-P2C-002");
        fs::create_dir_all(&packet_a).expect("packet a");
        fs::create_dir_all(&packet_b).expect("packet b");

        fs::write(
            packet_a.join("parity_report.json"),
            json!({
                "suite": "phase2c_packets",
                "packet_id": "FP-P2C-001",
                "oracle_present": true,
                "fixture_count": 2,
                "passed": 2,
                "failed": 0,
                "results": []
            })
            .to_string(),
        )
        .expect("parity a");
        fs::write(
            packet_a.join("parity_gate_result.json"),
            json!({
                "packet_id": "FP-P2C-001",
                "pass": true,
                "fixture_count": 2,
                "strict_total": 2,
                "strict_failed": 0,
                "hardened_total": 0,
                "hardened_failed": 0,
                "reasons": []
            })
            .to_string(),
        )
        .expect("gate a");

        fs::write(
            packet_b.join("parity_report.json"),
            json!({
                "suite": "phase2c_packets",
                "packet_id": "FP-P2C-002",
                "oracle_present": true,
                "fixture_count": 3,
                "passed": 2,
                "failed": 1,
                "results": []
            })
            .to_string(),
        )
        .expect("parity b");
        fs::write(
            packet_b.join("parity_gate_result.json"),
            json!({
                "packet_id": "FP-P2C-002",
                "pass": false,
                "fixture_count": 3,
                "strict_total": 2,
                "strict_failed": 1,
                "hardened_total": 1,
                "hardened_failed": 0,
                "reasons": ["synthetic gate failure"]
            })
            .to_string(),
        )
        .expect("gate b");

        fs::write(
            root.join("drift_history.jsonl"),
            format!(
                "{}\n{}\n",
                json!({
                    "ts_unix_ms": 1,
                    "packet_id": "FP-P2C-001",
                    "suite": "phase2c_packets",
                    "fixture_count": 2,
                    "passed": 2,
                    "failed": 0,
                    "strict_failed": 0,
                    "hardened_failed": 0,
                    "gate_pass": true,
                    "report_hash": "sha256:a"
                }),
                json!({
                    "ts_unix_ms": 2,
                    "packet_id": "FP-P2C-002",
                    "suite": "phase2c_packets",
                    "fixture_count": 3,
                    "passed": 2,
                    "failed": 1,
                    "strict_failed": 1,
                    "hardened_failed": 0,
                    "gate_pass": false,
                    "report_hash": "sha256:b"
                })
            ),
        )
        .expect("drift history");

        let source = FsFtuiDataSource::from_repo_root(dir.path());
        let dashboard = source
            .load_conformance_dashboard()
            .expect("load conformance dashboard");

        emit_test_log(
            "FP-P2C-002",
            "conformance_dashboard_aggregates_packet_and_trend_state",
            "mixed",
            2,
            "dashboard totals and trend aggregation",
            "pass",
        );
        assert_eq!(dashboard.total_packets, 2);
        assert_eq!(dashboard.green_packets, 1);
        assert_eq!(dashboard.failing_packets, 1);
        assert_eq!(dashboard.packet_trends.len(), 2);
        assert!(dashboard.render_plain().contains("Conformance packets=2"));
    }

    #[test]
    fn forensic_log_loader_skips_malformed_lines() {
        let dir = tempdir().expect("tempdir");
        let forensic_path = dir.path().join("forensic.jsonl");
        fs::write(
            &forensic_path,
            format!(
                "{}\nnot-json\n",
                json!({
                    "ts_unix_ms": 10,
                    "event": {
                        "kind": "suite_start",
                        "suite": "phase2c_packets",
                        "packet_filter": null
                    }
                })
            ),
        )
        .expect("write forensic log");

        let source = FsFtuiDataSource::from_repo_root(dir.path());
        let forensic = source
            .load_forensic_log(&forensic_path)
            .expect("load forensic log");

        assert!(!forensic.missing);
        assert_eq!(forensic.events.len(), 1);
        assert_eq!(forensic.malformed_lines, 1);
    }

    #[test]
    fn governance_gate_snapshot_reads_known_report() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("artifacts/phase2c");
        fs::create_dir_all(&root).expect("phase2c root");
        let ci = dir.path().join("artifacts/ci");
        fs::create_dir_all(&ci).expect("ci root");
        fs::write(
            ci.join("governance_gate_report.json"),
            json!({
                "generated_unix_ms": 123,
                "all_passed": false,
                "violation_count": 3
            })
            .to_string(),
        )
        .expect("governance report");

        let source = FsFtuiDataSource::from_repo_root(dir.path());
        let report = source
            .load_governance_gate_snapshot()
            .expect("load report")
            .expect("report present");
        assert!(!report.all_passed);
        assert_eq!(report.violation_count, 3);
        assert_eq!(report.generated_unix_ms, 123);
    }

    #[test]
    fn app_state_cycles_views_and_packet_selection() {
        let mut app = FtuiAppState::new(vec![
            "FP-P2C-002".to_owned(),
            "FP-P2C-001".to_owned(),
            "FP-P2C-003".to_owned(),
        ]);
        assert_eq!(app.selected_packet(), Some("FP-P2C-001"));
        app.select_next_packet();
        assert_eq!(app.selected_packet(), Some("FP-P2C-002"));
        app.cycle_next_view();
        app.cycle_next_view();
        app.cycle_next_view();
        app.cycle_next_view();
        assert_eq!(app.active_view, DashboardView::Conformance);
    }

    proptest! {
        #[test]
        fn prop_packet_id_accepts_three_digit_suffix(suffix in 0u16..1000u16) {
            let packet_id = format!("FP-P2C-{suffix:03}");
            emit_test_log(
                &packet_id,
                "prop_packet_id_accepts_three_digit_suffix",
                "n/a",
                u64::from(suffix),
                "is_valid_packet_id generated packet id",
                "pass",
            );
            prop_assert!(is_valid_packet_id(&packet_id));
        }

        #[test]
        fn prop_decision_summary_respects_cap_and_mode_counts(
            mode_plan in proptest::collection::vec(any::<bool>(), 1..32),
            cap in 0usize..8usize
        ) {
            let mut ledger = EvidenceLedger::new();
            let strict = RuntimePolicy::strict();
            let hardened = RuntimePolicy::hardened(Some(16));

            for (idx, strict_mode) in mode_plan.iter().copied().enumerate() {
                if strict_mode {
                    strict.decide_unknown_feature(
                        format!("feature_{idx}"),
                        "unsupported",
                        &mut ledger,
                    );
                } else {
                    hardened.decide_join_admission(idx + 1, &mut ledger);
                }
            }

            let summary = summarize_decision_dashboard(&ledger, &hardened, cap);
            let strict_expected = mode_plan.iter().filter(|&&is_strict| is_strict).count();
            let hardened_expected = mode_plan.len() - strict_expected;
            emit_test_log(
                "FRANKENTUI-E",
                "prop_decision_summary_respects_cap_and_mode_counts",
                "mixed",
                mode_plan.len() as u64,
                "summary totals and evidence cap invariants",
                "pass",
            );

            prop_assert_eq!(summary.total_records, mode_plan.len());
            prop_assert_eq!(summary.strict_records, strict_expected);
            prop_assert_eq!(summary.hardened_records, hardened_expected);
            prop_assert!(summary.cards.iter().all(|card| card.evidence_terms_shown <= cap));
            prop_assert!(
                summary
                    .cards
                    .iter()
                    .all(|card| card.evidence_terms_shown <= card.evidence_terms_total)
            );
        }
    }
}
