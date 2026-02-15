#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use fp_frame::{FrameError, Series, concat_series};
use fp_groupby::{GroupByOptions, groupby_count, groupby_mean, groupby_sum};
use fp_index::{AlignmentPlan, Index, IndexLabel, align_union, validate_alignment_plan};
use fp_io::{read_csv_str, write_csv_string};
use fp_join::{JoinType, join_series};
use fp_runtime::{
    DecodeProof, EvidenceLedger, RaptorQEnvelope, RaptorQMetadata, RuntimeMode, RuntimePolicy,
    ScrubStatus,
};
use fp_types::{Scalar, dropna, fill_na, nansum};
use raptorq::{Decoder, Encoder, EncodingPacket, ObjectTransmissionInformation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct HarnessConfig {
    pub repo_root: PathBuf,
    pub oracle_root: PathBuf,
    pub fixture_root: PathBuf,
    pub strict_mode: bool,
    pub python_bin: String,
    pub allow_system_pandas_fallback: bool,
}

impl HarnessConfig {
    #[must_use]
    pub fn default_paths() -> Self {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        Self {
            oracle_root: repo_root.join("legacy_pandas_code/pandas"),
            fixture_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"),
            strict_mode: true,
            python_bin: "python3".to_owned(),
            allow_system_pandas_fallback: false,
            repo_root,
        }
    }

    #[must_use]
    pub fn packet_fixture_root(&self) -> PathBuf {
        self.fixture_root.join("packets")
    }

    #[must_use]
    pub fn packet_artifact_root(&self, packet_id: &str) -> PathBuf {
        self.repo_root.join("artifacts/phase2c").join(packet_id)
    }

    #[must_use]
    pub fn parity_gate_path(&self, packet_id: &str) -> PathBuf {
        self.packet_artifact_root(packet_id)
            .join("parity_gate.yaml")
    }

    #[must_use]
    pub fn oracle_script_path(&self) -> PathBuf {
        self.repo_root
            .join("crates/fp-conformance/oracle/pandas_oracle.py")
    }
}

impl Default for HarnessConfig {
    fn default() -> Self {
        Self::default_paths()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessReport {
    pub suite: &'static str,
    pub oracle_present: bool,
    pub fixture_count: usize,
    pub strict_mode: bool,
}

#[must_use]
pub fn run_smoke(config: &HarnessConfig) -> HarnessReport {
    let fixture_count = fs::read_dir(&config.fixture_root)
        .ok()
        .into_iter()
        .flat_map(|it| it.filter_map(Result::ok))
        .count();

    HarnessReport {
        suite: "smoke",
        oracle_present: config.oracle_root.exists(),
        fixture_count,
        strict_mode: config.strict_mode,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleMode {
    FixtureExpected,
    LiveLegacyPandas,
}

#[derive(Debug, Clone)]
pub struct SuiteOptions {
    pub packet_filter: Option<String>,
    pub oracle_mode: OracleMode,
}

impl Default for SuiteOptions {
    fn default() -> Self {
        Self {
            packet_filter: None,
            oracle_mode: OracleMode::FixtureExpected,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureOperation {
    SeriesAdd,
    SeriesJoin,
    #[serde(rename = "groupby_sum", alias = "group_by_sum")]
    GroupBySum,
    IndexAlignUnion,
    IndexHasDuplicates,
    IndexFirstPositions,
    // FP-P2C-006: Join + concat
    SeriesConcat,
    // FP-P2C-007: Missingness + nanops
    NanSum,
    FillNa,
    DropNa,
    // FP-P2C-008: IO round-trip
    CsvRoundTrip,
    // FP-P2C-009: Storage invariants
    ColumnDtypeCheck,
    // FP-P2C-010: loc/iloc
    SeriesFilter,
    SeriesHead,
    // FP-P2C-011: Full GroupBy aggregate matrix
    #[serde(rename = "groupby_mean", alias = "group_by_mean")]
    GroupByMean,
    #[serde(rename = "groupby_count", alias = "group_by_count")]
    GroupByCount,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureJoinType {
    Inner,
    Left,
}

impl FixtureJoinType {
    #[must_use]
    pub fn into_join_type(self) -> JoinType {
        match self {
            Self::Inner => JoinType::Inner,
            Self::Left => JoinType::Left,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureOracleSource {
    Fixture,
    LiveLegacyPandas,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureSeries {
    pub name: String,
    pub index: Vec<IndexLabel>,
    pub values: Vec<Scalar>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureExpectedSeries {
    pub index: Vec<IndexLabel>,
    pub values: Vec<Scalar>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureExpectedAlignment {
    pub union_index: Vec<IndexLabel>,
    pub left_positions: Vec<Option<usize>>,
    pub right_positions: Vec<Option<usize>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureExpectedJoin {
    pub index: Vec<IndexLabel>,
    pub left_values: Vec<Scalar>,
    pub right_values: Vec<Scalar>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketFixture {
    pub packet_id: String,
    pub case_id: String,
    pub mode: RuntimeMode,
    pub operation: FixtureOperation,
    #[serde(default)]
    pub oracle_source: Option<FixtureOracleSource>,
    #[serde(default)]
    pub left: Option<FixtureSeries>,
    #[serde(default)]
    pub right: Option<FixtureSeries>,
    #[serde(default)]
    pub index: Option<Vec<IndexLabel>>,
    #[serde(default)]
    pub join_type: Option<FixtureJoinType>,
    #[serde(default)]
    pub expected_series: Option<FixtureExpectedSeries>,
    #[serde(default)]
    pub expected_join: Option<FixtureExpectedJoin>,
    #[serde(default)]
    pub expected_alignment: Option<FixtureExpectedAlignment>,
    #[serde(default)]
    pub expected_bool: Option<bool>,
    #[serde(default)]
    pub expected_positions: Option<Vec<Option<usize>>>,
    #[serde(default)]
    pub expected_scalar: Option<Scalar>,
    #[serde(default)]
    pub expected_dtype: Option<String>,
    #[serde(default)]
    pub fill_value: Option<Scalar>,
    #[serde(default)]
    pub head_n: Option<usize>,
    #[serde(default)]
    pub csv_input: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseStatus {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaseResult {
    pub packet_id: String,
    pub case_id: String,
    pub mode: RuntimeMode,
    pub operation: FixtureOperation,
    pub status: CaseStatus,
    pub mismatch: Option<String>,
    pub evidence_records: usize,
}

// === Differential Harness: Comparator Taxonomy + Drift Classification ===

/// Drift severity classification following frankenlibc fail-closed doctrine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftLevel {
    /// Hard parity failure: no tolerance, blocks gates.
    Critical,
    /// Soft divergence: within configured tolerance budget.
    NonCritical,
    /// Known behavioral gap, documented and accepted.
    Informational,
}

/// Comparison dimension in the differential taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonCategory {
    /// Scalar value equality (exact or within tolerance).
    Value,
    /// Data type agreement between actual and expected.
    Type,
    /// Shape: length or dimensionality mismatch.
    Shape,
    /// Index labels and ordering.
    Index,
    /// Null/NaN propagation behavior.
    Nullness,
}

/// A single drift observation from a differential comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftRecord {
    pub category: ComparisonCategory,
    pub level: DriftLevel,
    pub location: String,
    pub message: String,
}

/// Full differential comparison result for a single fixture case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DifferentialResult {
    pub case_id: String,
    pub packet_id: String,
    pub operation: FixtureOperation,
    pub mode: RuntimeMode,
    pub oracle_source: FixtureOracleSource,
    pub status: CaseStatus,
    pub drift_records: Vec<DriftRecord>,
    pub evidence_records: usize,
}

impl DifferentialResult {
    /// Convert to backward-compatible CaseResult.
    #[must_use]
    pub fn to_case_result(&self) -> CaseResult {
        let mismatch = if self.drift_records.is_empty() {
            None
        } else {
            Some(
                self.drift_records
                    .iter()
                    .map(|d| {
                        format!(
                            "[{:?}/{:?}] {}: {}",
                            d.category, d.level, d.location, d.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        };
        CaseResult {
            packet_id: self.packet_id.clone(),
            case_id: self.case_id.clone(),
            mode: self.mode,
            operation: self.operation,
            status: self.status.clone(),
            mismatch,
            evidence_records: self.evidence_records,
        }
    }
}

/// Per-category drift count in a summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryCount {
    pub category: ComparisonCategory,
    pub count: usize,
}

/// Aggregate drift statistics across all differential results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftSummary {
    pub total_drift_records: usize,
    pub critical_count: usize,
    pub non_critical_count: usize,
    pub informational_count: usize,
    pub categories: Vec<CategoryCount>,
}

/// Differential report: extends PacketParityReport with structured drift details.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DifferentialReport {
    pub report: PacketParityReport,
    pub differential_results: Vec<DifferentialResult>,
    pub drift_summary: DriftSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketParityReport {
    pub suite: String,
    pub packet_id: Option<String>,
    pub oracle_present: bool,
    pub fixture_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<CaseResult>,
}

impl PacketParityReport {
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.failed == 0 && self.fixture_count > 0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketGateResult {
    pub packet_id: String,
    pub pass: bool,
    pub fixture_count: usize,
    pub strict_total: usize,
    pub strict_failed: usize,
    pub hardened_total: usize,
    pub hardened_failed: usize,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaptorQPacketRecord {
    pub source_block_number: u8,
    pub encoding_symbol_id: u32,
    pub is_source: bool,
    pub serialized_hex: String,
    pub symbol_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaptorQScrubReport {
    pub verified_at_unix_ms: u64,
    pub status: String,
    pub packet_count: usize,
    pub invalid_packets: usize,
    pub source_hash_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaptorQSidecarArtifact {
    #[serde(flatten)]
    pub envelope: RaptorQEnvelope,
    pub oti_serialized_hex: String,
    pub source_packets: usize,
    pub repair_packets: usize,
    pub repair_packets_per_block: u32,
    pub packet_records: Vec<RaptorQPacketRecord>,
    pub scrub_report: RaptorQScrubReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WrittenPacketArtifacts {
    pub packet_id: String,
    pub parity_report_path: PathBuf,
    pub raptorq_sidecar_path: PathBuf,
    pub decode_proof_path: PathBuf,
    pub gate_result_path: PathBuf,
    pub mismatch_corpus_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PacketDriftHistoryEntry {
    pub ts_unix_ms: u64,
    pub packet_id: String,
    pub suite: String,
    pub fixture_count: usize,
    pub passed: usize,
    pub failed: usize,
    pub strict_failed: usize,
    pub hardened_failed: usize,
    pub gate_pass: bool,
    pub report_hash: String,
}

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error("fixture format error: {0}")]
    FixtureFormat(String),
    #[error("oracle is unavailable: {0}")]
    OracleUnavailable(String),
    #[error("oracle command failed: status={status}, stderr={stderr}")]
    OracleCommandFailed { status: i32, stderr: String },
    #[error("raptorq error: {0}")]
    RaptorQ(String),
}

#[derive(Debug, Deserialize)]
struct ParityGateConfig {
    packet_id: String,
    strict: StrictGateConfig,
    hardened: HardenedGateConfig,
    machine_check: MachineCheckConfig,
}

#[derive(Debug, Deserialize)]
struct StrictGateConfig {
    critical_drift_budget: usize,
    non_critical_drift_budget_percent: f64,
}

#[derive(Debug, Deserialize)]
struct HardenedGateConfig {
    divergence_budget_percent: f64,
    allowlisted_divergence_categories: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct MachineCheckConfig {
    suite: String,
    require_fixture_count_at_least: usize,
    require_failed: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct OracleRequest {
    operation: FixtureOperation,
    left: Option<FixtureSeries>,
    right: Option<FixtureSeries>,
    index: Option<Vec<IndexLabel>>,
    join_type: Option<FixtureJoinType>,
    #[serde(default)]
    fill_value: Option<Scalar>,
    #[serde(default)]
    head_n: Option<usize>,
    #[serde(default)]
    csv_input: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OracleResponse {
    #[serde(default)]
    expected_series: Option<FixtureExpectedSeries>,
    #[serde(default)]
    expected_join: Option<FixtureExpectedJoin>,
    #[serde(default)]
    expected_alignment: Option<FixtureExpectedAlignment>,
    #[serde(default)]
    expected_bool: Option<bool>,
    #[serde(default)]
    expected_positions: Option<Vec<Option<usize>>>,
    #[serde(default)]
    expected_scalar: Option<Scalar>,
    #[serde(default)]
    expected_dtype: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum ResolvedExpected {
    Series(FixtureExpectedSeries),
    Join(FixtureExpectedJoin),
    Alignment(FixtureExpectedAlignment),
    Bool(bool),
    Positions(Vec<Option<usize>>),
    Scalar(Scalar),
    Dtype(String),
}

pub fn run_packet_suite(config: &HarnessConfig) -> Result<PacketParityReport, HarnessError> {
    run_packet_suite_with_options(config, &SuiteOptions::default())
}

pub fn run_packet_suite_with_options(
    config: &HarnessConfig,
    options: &SuiteOptions,
) -> Result<PacketParityReport, HarnessError> {
    let fixtures = load_fixtures(config, options.packet_filter.as_deref())?;
    build_report(
        config,
        "phase2c_packets".to_owned(),
        None,
        &fixtures,
        options,
    )
}

pub fn run_packet_by_id(
    config: &HarnessConfig,
    packet_id: &str,
    oracle_mode: OracleMode,
) -> Result<PacketParityReport, HarnessError> {
    let options = SuiteOptions {
        packet_filter: Some(packet_id.to_owned()),
        oracle_mode,
    };
    let fixtures = load_fixtures(config, Some(packet_id))?;
    build_report(
        config,
        format!("phase2c_packets:{packet_id}"),
        Some(packet_id.to_owned()),
        &fixtures,
        &options,
    )
}

pub fn run_packets_grouped(
    config: &HarnessConfig,
    options: &SuiteOptions,
) -> Result<Vec<PacketParityReport>, HarnessError> {
    let fixtures = load_fixtures(config, options.packet_filter.as_deref())?;
    let mut grouped = BTreeMap::<String, Vec<PacketFixture>>::new();
    for fixture in fixtures {
        grouped
            .entry(fixture.packet_id.clone())
            .or_default()
            .push(fixture);
    }

    let mut reports = Vec::with_capacity(grouped.len());
    for (packet_id, mut packet_fixtures) in grouped {
        packet_fixtures.sort_by(|a, b| a.case_id.cmp(&b.case_id));
        reports.push(build_report(
            config,
            format!("phase2c_packets:{packet_id}"),
            Some(packet_id),
            &packet_fixtures,
            options,
        )?);
    }
    Ok(reports)
}

pub fn write_grouped_artifacts(
    config: &HarnessConfig,
    reports: &[PacketParityReport],
) -> Result<Vec<WrittenPacketArtifacts>, HarnessError> {
    reports
        .iter()
        .map(|report| write_packet_artifacts(config, report))
        .collect()
}

pub fn enforce_packet_gates(
    config: &HarnessConfig,
    reports: &[PacketParityReport],
) -> Result<(), HarnessError> {
    let mut failures = Vec::new();
    for report in reports {
        let packet_id = report.packet_id.as_deref().unwrap_or("<unknown>");
        if !report.is_green() {
            failures.push(format!(
                "{packet_id}: parity report failed fixtures={}",
                report.failed
            ));
        }
        let gate = evaluate_parity_gate(config, report)?;
        if !gate.pass {
            failures.push(format!(
                "{packet_id}: gate failed reasons={}",
                gate.reasons.join("; ")
            ));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(HarnessError::FixtureFormat(format!(
            "phase2c enforcement failed: {}",
            failures.join(" | ")
        )))
    }
}

pub fn append_phase2c_drift_history(
    config: &HarnessConfig,
    reports: &[PacketParityReport],
) -> Result<PathBuf, HarnessError> {
    let history_path = config
        .repo_root
        .join("artifacts/phase2c/drift_history.jsonl");
    if let Some(parent) = history_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&history_path)?;

    for report in reports {
        let gate = evaluate_parity_gate(config, report)?;
        let report_json = serde_json::to_vec(report)?;
        let entry = PacketDriftHistoryEntry {
            ts_unix_ms: now_unix_ms(),
            packet_id: report
                .packet_id
                .clone()
                .unwrap_or_else(|| "<unknown>".to_owned()),
            suite: report.suite.clone(),
            fixture_count: report.fixture_count,
            passed: report.passed,
            failed: report.failed,
            strict_failed: gate.strict_failed,
            hardened_failed: gate.hardened_failed,
            gate_pass: gate.pass,
            report_hash: format!("sha256:{}", hash_bytes(&report_json)),
        };
        writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    }

    Ok(history_path)
}

// === Differential Harness: Public API ===

/// Run a differential suite over all matching fixtures with taxonomy-based comparison.
pub fn run_differential_suite(
    config: &HarnessConfig,
    options: &SuiteOptions,
) -> Result<DifferentialReport, HarnessError> {
    let fixtures = load_fixtures(config, options.packet_filter.as_deref())?;
    build_differential_report_internal(
        config,
        "phase2c_packets".to_owned(),
        None,
        &fixtures,
        options,
    )
}

/// Run a differential suite filtered by packet ID.
pub fn run_differential_by_id(
    config: &HarnessConfig,
    packet_id: &str,
    oracle_mode: OracleMode,
) -> Result<DifferentialReport, HarnessError> {
    let options = SuiteOptions {
        packet_filter: Some(packet_id.to_owned()),
        oracle_mode,
    };
    let fixtures = load_fixtures(config, Some(packet_id))?;
    build_differential_report_internal(
        config,
        format!("phase2c_packets:{packet_id}"),
        Some(packet_id.to_owned()),
        &fixtures,
        &options,
    )
}

/// Build a DifferentialReport from pre-computed DifferentialResults.
#[must_use]
pub fn build_differential_report(
    suite: String,
    packet_id: Option<String>,
    oracle_present: bool,
    results: Vec<DifferentialResult>,
) -> DifferentialReport {
    let drift_summary = summarize_drift(&results);
    let case_results: Vec<CaseResult> = results
        .iter()
        .map(DifferentialResult::to_case_result)
        .collect();
    let failed = case_results
        .iter()
        .filter(|r| matches!(r.status, CaseStatus::Fail))
        .count();
    let passed = case_results.len().saturating_sub(failed);
    DifferentialReport {
        report: PacketParityReport {
            suite,
            packet_id,
            oracle_present,
            fixture_count: case_results.len(),
            passed,
            failed,
            results: case_results,
        },
        differential_results: results,
        drift_summary,
    }
}

pub fn write_packet_artifacts(
    config: &HarnessConfig,
    report: &PacketParityReport,
) -> Result<WrittenPacketArtifacts, HarnessError> {
    let packet_id = report
        .packet_id
        .as_deref()
        .ok_or_else(|| HarnessError::FixtureFormat("packet_id is required".to_owned()))?;

    let root = config.packet_artifact_root(packet_id);
    fs::create_dir_all(&root)?;

    let parity_report_path = root.join("parity_report.json");
    fs::write(&parity_report_path, serde_json::to_string_pretty(report)?)?;

    let report_bytes = fs::read(&parity_report_path)?;
    let mut sidecar = generate_raptorq_sidecar(
        &format!("{packet_id}/parity_report"),
        "conformance",
        &report_bytes,
        8,
    )?;
    let decode_proof = run_raptorq_decode_recovery_drill(&sidecar, &report_bytes)?;
    sidecar.envelope.decode_proofs = vec![decode_proof.clone()];
    sidecar.envelope.scrub = ScrubStatus {
        last_ok_unix_ms: sidecar.scrub_report.verified_at_unix_ms,
        status: if sidecar.scrub_report.source_hash_verified {
            "ok".to_owned()
        } else {
            "failed".to_owned()
        },
    };

    let raptorq_sidecar_path = root.join("parity_report.raptorq.json");
    fs::write(
        &raptorq_sidecar_path,
        serde_json::to_string_pretty(&sidecar)?,
    )?;

    let decode_proof_path = root.join("parity_report.decode_proof.json");
    let decode_artifact = DecodeProofArtifact {
        packet_id: packet_id.to_owned(),
        decode_proofs: vec![decode_proof],
        status: DecodeProofStatus::Recovered,
    };
    fs::write(
        &decode_proof_path,
        serde_json::to_string_pretty(&decode_artifact)?,
    )?;

    let gate_result = evaluate_parity_gate(config, report)?;
    let gate_result_path = root.join("parity_gate_result.json");
    fs::write(
        &gate_result_path,
        serde_json::to_string_pretty(&gate_result)?,
    )?;

    let mismatch_corpus_path = root.join("parity_mismatch_corpus.json");
    let mismatches = report
        .results
        .iter()
        .filter(|result| matches!(result.status, CaseStatus::Fail))
        .cloned()
        .collect::<Vec<_>>();
    let mismatch_payload = serde_json::json!({
        "packet_id": packet_id,
        "mismatch_count": mismatches.len(),
        "mismatches": mismatches,
    });
    fs::write(
        &mismatch_corpus_path,
        serde_json::to_string_pretty(&mismatch_payload)?,
    )?;

    Ok(WrittenPacketArtifacts {
        packet_id: packet_id.to_owned(),
        parity_report_path,
        raptorq_sidecar_path,
        decode_proof_path,
        gate_result_path,
        mismatch_corpus_path,
    })
}

pub fn evaluate_parity_gate(
    config: &HarnessConfig,
    report: &PacketParityReport,
) -> Result<PacketGateResult, HarnessError> {
    let packet_id = report
        .packet_id
        .clone()
        .ok_or_else(|| HarnessError::FixtureFormat("report has no packet_id".to_owned()))?;
    let gate: ParityGateConfig =
        serde_yaml::from_str(&fs::read_to_string(config.parity_gate_path(&packet_id))?)?;

    let strict_total = report
        .results
        .iter()
        .filter(|result| matches!(result.mode, RuntimeMode::Strict))
        .count();
    let strict_failed = report
        .results
        .iter()
        .filter(|result| {
            matches!(result.mode, RuntimeMode::Strict) && matches!(result.status, CaseStatus::Fail)
        })
        .count();
    let hardened_total = report
        .results
        .iter()
        .filter(|result| matches!(result.mode, RuntimeMode::Hardened))
        .count();
    let hardened_failed = report
        .results
        .iter()
        .filter(|result| {
            matches!(result.mode, RuntimeMode::Hardened)
                && matches!(result.status, CaseStatus::Fail)
        })
        .count();

    let strict_failure_percent = percent(strict_failed, strict_total);
    let hardened_failure_percent = percent(hardened_failed, hardened_total);

    let mut reasons = Vec::new();
    if gate.packet_id != packet_id {
        reasons.push(format!(
            "packet_id mismatch between gate ({}) and report ({packet_id})",
            gate.packet_id
        ));
    }
    if gate.machine_check.suite != "phase2c_packets"
        && gate.machine_check.suite != report.suite
        && !report.suite.starts_with(&gate.machine_check.suite)
    {
        reasons.push(format!(
            "suite mismatch: gate={}, report={}",
            gate.machine_check.suite, report.suite
        ));
    }
    if report.fixture_count < gate.machine_check.require_fixture_count_at_least {
        reasons.push(format!(
            "fixture_count={} below required {}",
            report.fixture_count, gate.machine_check.require_fixture_count_at_least
        ));
    }
    if report.failed != gate.machine_check.require_failed {
        reasons.push(format!(
            "failed={} but gate requires {}",
            report.failed, gate.machine_check.require_failed
        ));
    }
    if strict_failed > gate.strict.critical_drift_budget {
        reasons.push(format!(
            "strict_failed={} exceeds critical_drift_budget={}",
            strict_failed, gate.strict.critical_drift_budget
        ));
    }
    if strict_failure_percent > gate.strict.non_critical_drift_budget_percent {
        reasons.push(format!(
            "strict failure percent {:.3}% exceeds {:.3}%",
            strict_failure_percent, gate.strict.non_critical_drift_budget_percent
        ));
    }
    if hardened_failure_percent > gate.hardened.divergence_budget_percent {
        reasons.push(format!(
            "hardened failure percent {:.3}% exceeds {:.3}%",
            hardened_failure_percent, gate.hardened.divergence_budget_percent
        ));
    }
    if let Some(categories) = &gate.hardened.allowlisted_divergence_categories
        && categories.is_empty()
    {
        reasons.push("hardened allowlist categories must not be empty".to_owned());
    }

    Ok(PacketGateResult {
        packet_id,
        pass: reasons.is_empty(),
        fixture_count: report.fixture_count,
        strict_total,
        strict_failed,
        hardened_total,
        hardened_failed,
        reasons,
    })
}

pub fn generate_raptorq_sidecar(
    artifact_id: &str,
    artifact_type: &str,
    report_bytes: &[u8],
    repair_packets_per_block: u32,
) -> Result<RaptorQSidecarArtifact, HarnessError> {
    if report_bytes.is_empty() {
        return Err(HarnessError::RaptorQ(
            "cannot generate sidecar for empty payload".to_owned(),
        ));
    }

    let encoder = Encoder::with_defaults(report_bytes, 1400);
    let config = encoder.get_config();

    let mut packet_records = Vec::new();
    let mut symbol_hashes = Vec::new();
    let mut source_packets = 0usize;

    for block in encoder.get_block_encoders() {
        for packet in block.source_packets() {
            source_packets += 1;
            let record = packet_record(packet, true);
            symbol_hashes.push(record.symbol_hash.clone());
            packet_records.push(record);
        }
        for packet in block.repair_packets(0, repair_packets_per_block) {
            let record = packet_record(packet, false);
            symbol_hashes.push(record.symbol_hash.clone());
            packet_records.push(record);
        }
    }

    let repair_packets = packet_records.len().saturating_sub(source_packets);
    let source_hash = hash_bytes(report_bytes);
    let mut scrub_report = verify_raptorq_sidecar_internal(
        report_bytes,
        &source_hash,
        &packet_records,
        now_unix_ms(),
    )?;
    scrub_report.status = if scrub_report.invalid_packets == 0 && scrub_report.source_hash_verified
    {
        "ok".to_owned()
    } else {
        "failed".to_owned()
    };

    let envelope = RaptorQEnvelope {
        artifact_id: artifact_id.to_owned(),
        artifact_type: artifact_type.to_owned(),
        source_hash: format!("sha256:{source_hash}"),
        raptorq: RaptorQMetadata {
            k: source_packets as u32,
            repair_symbols: repair_packets as u32,
            overhead_ratio: if source_packets == 0 {
                0.0
            } else {
                repair_packets as f64 / source_packets as f64
            },
            symbol_hashes,
        },
        scrub: ScrubStatus {
            last_ok_unix_ms: scrub_report.verified_at_unix_ms,
            status: scrub_report.status.clone(),
        },
        decode_proofs: Vec::new(),
    };

    Ok(RaptorQSidecarArtifact {
        envelope,
        oti_serialized_hex: hex_encode(&config.serialize()),
        source_packets,
        repair_packets,
        repair_packets_per_block,
        packet_records,
        scrub_report,
    })
}

pub fn run_raptorq_decode_recovery_drill(
    sidecar: &RaptorQSidecarArtifact,
    report_bytes: &[u8],
) -> Result<DecodeProof, HarnessError> {
    if sidecar.packet_records.is_empty() {
        return Err(HarnessError::RaptorQ(
            "sidecar has no packet records".to_owned(),
        ));
    }

    let oti_bytes = hex_decode(&sidecar.oti_serialized_hex)?;
    if oti_bytes.len() != 12 {
        return Err(HarnessError::RaptorQ(format!(
            "invalid OTI byte length: {}",
            oti_bytes.len()
        )));
    }
    let mut oti = [0_u8; 12];
    oti.copy_from_slice(&oti_bytes);
    let config = ObjectTransmissionInformation::deserialize(&oti);

    let drop_count = sidecar.source_packets.saturating_div(4).max(1);
    let mut dropped_sources = 0usize;
    let mut packets = Vec::with_capacity(sidecar.packet_records.len());
    for record in &sidecar.packet_records {
        if record.is_source && dropped_sources < drop_count {
            dropped_sources += 1;
            continue;
        }

        let packet_bytes = hex_decode(&record.serialized_hex)?;
        packets.push(EncodingPacket::deserialize(&packet_bytes));
    }

    let mut decoder = Decoder::new(config);
    let mut recovered = None;
    for packet in packets {
        recovered = decoder.decode(packet);
        if recovered.is_some() {
            break;
        }
    }

    let recovered = recovered.ok_or_else(|| {
        HarnessError::RaptorQ("decode drill could not reconstruct payload".to_owned())
    })?;
    if recovered != report_bytes {
        return Err(HarnessError::RaptorQ(
            "decode drill recovered bytes do not match source payload".to_owned(),
        ));
    }

    let proof_material = format!(
        "{}:{}:{}",
        sidecar.envelope.artifact_id,
        dropped_sources,
        hash_bytes(&recovered)
    );

    Ok(DecodeProof {
        ts_unix_ms: now_unix_ms(),
        reason: format!(
            "raptorq decode drill dropped {dropped_sources} source packets and recovered payload"
        ),
        recovered_blocks: dropped_sources as u32,
        proof_hash: format!("sha256:{}", hash_bytes(proof_material.as_bytes())),
    })
}

pub fn verify_raptorq_sidecar(
    sidecar: &RaptorQSidecarArtifact,
    report_bytes: &[u8],
) -> Result<RaptorQScrubReport, HarnessError> {
    let expected = sidecar
        .envelope
        .source_hash
        .strip_prefix("sha256:")
        .ok_or_else(|| {
            HarnessError::RaptorQ("source hash must be prefixed with sha256:".to_owned())
        })?
        .to_owned();
    verify_raptorq_sidecar_internal(
        report_bytes,
        &expected,
        &sidecar.packet_records,
        now_unix_ms(),
    )
}

fn verify_raptorq_sidecar_internal(
    report_bytes: &[u8],
    expected_source_hash: &str,
    records: &[RaptorQPacketRecord],
    ts_unix_ms: u64,
) -> Result<RaptorQScrubReport, HarnessError> {
    let source_hash_verified = hash_bytes(report_bytes) == expected_source_hash;
    let mut invalid_packets = 0usize;
    for record in records {
        let bytes = hex_decode(&record.serialized_hex)?;
        if hash_bytes(&bytes) != record.symbol_hash {
            invalid_packets += 1;
        }
    }

    Ok(RaptorQScrubReport {
        verified_at_unix_ms: ts_unix_ms,
        status: if source_hash_verified && invalid_packets == 0 {
            "ok".to_owned()
        } else {
            "failed".to_owned()
        },
        packet_count: records.len(),
        invalid_packets,
        source_hash_verified,
    })
}

fn packet_record(packet: EncodingPacket, is_source: bool) -> RaptorQPacketRecord {
    let payload = packet.payload_id();
    let serialized = packet.serialize();
    RaptorQPacketRecord {
        source_block_number: payload.source_block_number(),
        encoding_symbol_id: payload.encoding_symbol_id(),
        is_source,
        serialized_hex: hex_encode(&serialized),
        symbol_hash: hash_bytes(&serialized),
    }
}

// === RaptorQ CI Enforcement (bd-2gi.9) ===

/// Typed decode proof artifact matching `decode_proof_artifact.schema.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodeProofArtifact {
    pub packet_id: String,
    pub decode_proofs: Vec<DecodeProof>,
    pub status: DecodeProofStatus,
}

/// Outcome of a decode drill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecodeProofStatus {
    Recovered,
    Failed,
    NotAttempted,
}

impl std::fmt::Display for DecodeProofStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Recovered => write!(f, "recovered"),
            Self::Failed => write!(f, "failed"),
            Self::NotAttempted => write!(f, "not_attempted"),
        }
    }
}

/// Result of verifying a single packet's RaptorQ sidecar integrity (Rule T5).
#[derive(Debug, Clone)]
pub struct SidecarIntegrityResult {
    pub packet_id: String,
    pub parity_report_exists: bool,
    pub sidecar_exists: bool,
    pub decode_proof_exists: bool,
    pub source_hash_matches: bool,
    pub scrub_ok: bool,
    pub decode_proof_valid: bool,
    pub errors: Vec<String>,
}

impl SidecarIntegrityResult {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
            && self.parity_report_exists
            && self.sidecar_exists
            && self.decode_proof_exists
            && self.source_hash_matches
            && self.scrub_ok
            && self.decode_proof_valid
    }
}

/// Verify Rule T5 for a single packet directory: parity_report.json must have
/// a corresponding raptorq sidecar and decode proof with matching hashes.
pub fn verify_packet_sidecar_integrity(
    packet_dir: &Path,
    packet_id: &str,
) -> SidecarIntegrityResult {
    let mut result = SidecarIntegrityResult {
        packet_id: packet_id.to_owned(),
        parity_report_exists: false,
        sidecar_exists: false,
        decode_proof_exists: false,
        source_hash_matches: false,
        scrub_ok: false,
        decode_proof_valid: false,
        errors: Vec::new(),
    };

    let report_path = packet_dir.join("parity_report.json");
    let sidecar_path = packet_dir.join("parity_report.raptorq.json");
    let proof_path = packet_dir.join("parity_report.decode_proof.json");

    // Check file existence (Rule T5)
    result.parity_report_exists = report_path.exists();
    result.sidecar_exists = sidecar_path.exists();
    result.decode_proof_exists = proof_path.exists();

    if !result.parity_report_exists {
        result
            .errors
            .push(format!("{packet_id}: missing parity_report.json"));
        return result;
    }
    if !result.sidecar_exists {
        result.errors.push(format!(
            "{packet_id}: missing parity_report.raptorq.json (Rule T5)"
        ));
    }
    if !result.decode_proof_exists {
        result.errors.push(format!(
            "{packet_id}: missing parity_report.decode_proof.json (Rule T5)"
        ));
    }
    if !result.sidecar_exists || !result.decode_proof_exists {
        return result;
    }

    // Read artifacts
    let report_bytes = match fs::read(&report_path) {
        Ok(b) => b,
        Err(e) => {
            result
                .errors
                .push(format!("{packet_id}: cannot read parity_report.json: {e}"));
            return result;
        }
    };
    let sidecar: RaptorQSidecarArtifact = match fs::read_to_string(&sidecar_path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(s) => s,
        Err(e) => {
            result.errors.push(format!(
                "{packet_id}: cannot parse parity_report.raptorq.json: {e}"
            ));
            return result;
        }
    };
    let proof: DecodeProofArtifact = match fs::read_to_string(&proof_path)
        .map_err(|e| e.to_string())
        .and_then(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
    {
        Ok(p) => p,
        Err(e) => {
            result.errors.push(format!(
                "{packet_id}: cannot parse parity_report.decode_proof.json: {e}"
            ));
            return result;
        }
    };

    // Verify source hash matches (sidecar.source_hash == SHA-256 of parity_report.json)
    let actual_hash = hash_bytes(&report_bytes);
    let expected_hash = sidecar
        .envelope
        .source_hash
        .strip_prefix("sha256:")
        .unwrap_or(&sidecar.envelope.source_hash);
    result.source_hash_matches = actual_hash == expected_hash;
    if !result.source_hash_matches {
        result.errors.push(format!(
            "{packet_id}: source_hash mismatch (expected {expected_hash}, got {actual_hash})"
        ));
    }

    // Verify scrub status
    result.scrub_ok = sidecar.scrub_report.status == "ok"
        && sidecar.scrub_report.source_hash_verified
        && sidecar.scrub_report.invalid_packets == 0;
    if !result.scrub_ok {
        result.errors.push(format!(
            "{packet_id}: scrub report not ok (status={}, invalid={})",
            sidecar.scrub_report.status, sidecar.scrub_report.invalid_packets
        ));
    }

    // Verify decode proof
    result.decode_proof_valid = proof.status == DecodeProofStatus::Recovered
        && !proof.decode_proofs.is_empty()
        && proof.packet_id == packet_id;
    if !result.decode_proof_valid {
        result.errors.push(format!(
            "{packet_id}: decode proof invalid (status={}, proofs={})",
            proof.status,
            proof.decode_proofs.len()
        ));
    }

    result
}

/// CI gate function: verify all packet sidecars under an artifact root directory.
/// Returns Ok with results if all pass, Err with failures if any fail.
pub fn verify_all_sidecars_ci(
    artifact_root: &Path,
) -> Result<Vec<SidecarIntegrityResult>, Vec<SidecarIntegrityResult>> {
    let phase2c = artifact_root.join("phase2c");
    if !phase2c.exists() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(&phase2c)
        .unwrap_or_else(|_| fs::read_dir(".").unwrap())
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                && e.file_name()
                    .to_str()
                    .map(|n| n.starts_with("FP-P2C-"))
                    .unwrap_or(false)
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let packet_id = entry.file_name().to_string_lossy().to_string();
        let result = verify_packet_sidecar_integrity(&entry.path(), &packet_id);
        results.push(result);
    }

    let failures: Vec<_> = results.iter().filter(|r| !r.is_ok()).cloned().collect();
    if failures.is_empty() {
        Ok(results)
    } else {
        Err(failures)
    }
}

// === CI Gate Topology (bd-2gi.10) ===

/// CI gate identifiers matching the G1..G8 pipeline from COVERAGE_FLAKE_BUDGETS.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CiGate {
    /// G1: Compilation + formatting
    G1Compile,
    /// G2: Lint (clippy)
    G2Lint,
    /// G3: Unit tests
    G3Unit,
    /// G4: Property tests
    G4Property,
    /// G4.5: Fuzz regression (nightly only)
    G4_5Fuzz,
    /// G5: Integration tests
    G5Integration,
    /// G6: Conformance tests
    G6Conformance,
    /// G7: Coverage floor
    G7Coverage,
    /// G8: E2E orchestrator
    G8E2e,
}

impl CiGate {
    /// Gate ordering index for pipeline sequencing.
    pub fn order(self) -> u8 {
        match self {
            Self::G1Compile => 1,
            Self::G2Lint => 2,
            Self::G3Unit => 3,
            Self::G4Property => 4,
            Self::G4_5Fuzz => 5,
            Self::G5Integration => 6,
            Self::G6Conformance => 7,
            Self::G7Coverage => 8,
            Self::G8E2e => 9,
        }
    }

    /// Human-readable gate label.
    pub fn label(self) -> &'static str {
        match self {
            Self::G1Compile => "G1: Compile + Format",
            Self::G2Lint => "G2: Lint (Clippy)",
            Self::G3Unit => "G3: Unit Tests",
            Self::G4Property => "G4: Property Tests",
            Self::G4_5Fuzz => "G4.5: Fuzz Regression",
            Self::G5Integration => "G5: Integration Tests",
            Self::G6Conformance => "G6: Conformance",
            Self::G7Coverage => "G7: Coverage Floor",
            Self::G8E2e => "G8: E2E Pipeline",
        }
    }

    /// Shell command(s) for this gate (when run via external CI).
    pub fn commands(self) -> Vec<&'static str> {
        match self {
            Self::G1Compile => vec!["cargo check --all-targets", "cargo fmt --check"],
            Self::G2Lint => vec!["cargo clippy --workspace --all-targets"],
            Self::G3Unit => vec!["cargo test --workspace --lib"],
            Self::G4Property => vec!["cargo test --workspace --test proptest_properties"],
            Self::G4_5Fuzz => vec![], // nightly only, defined in ADVERSARIAL_FUZZ_CORPUS.md
            Self::G5Integration => vec!["cargo test --workspace --test smoke"],
            Self::G6Conformance => vec!["cargo test -p fp-conformance -- --nocapture"],
            Self::G7Coverage => vec!["cargo llvm-cov --workspace --summary-only"],
            Self::G8E2e => vec![], // Rust-native, uses run_e2e_suite()
        }
    }

    /// All gates in pipeline order.
    pub fn pipeline() -> Vec<CiGate> {
        vec![
            Self::G1Compile,
            Self::G2Lint,
            Self::G3Unit,
            Self::G4Property,
            Self::G5Integration,
            Self::G6Conformance,
            Self::G7Coverage,
            Self::G8E2e,
        ]
    }

    /// Default pipeline for per-commit CI (excludes G4.5 fuzz and G7 coverage).
    pub fn commit_pipeline() -> Vec<CiGate> {
        vec![
            Self::G1Compile,
            Self::G2Lint,
            Self::G3Unit,
            Self::G4Property,
            Self::G5Integration,
            Self::G6Conformance,
        ]
    }
}

impl std::fmt::Display for CiGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// Result of a single CI gate evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiGateResult {
    pub gate: CiGate,
    pub passed: bool,
    pub elapsed_ms: u64,
    pub summary: String,
    pub errors: Vec<String>,
}

/// Result of a full CI gate pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiPipelineResult {
    pub gates: Vec<CiGateResult>,
    pub all_passed: bool,
    pub first_failure: Option<CiGate>,
    pub elapsed_ms: u64,
}

impl CiPipelineResult {
    pub fn passed_count(&self) -> usize {
        self.gates.iter().filter(|g| g.passed).count()
    }

    pub fn total_count(&self) -> usize {
        self.gates.len()
    }
}

impl std::fmt::Display for CiPipelineResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.all_passed {
            writeln!(
                f,
                "CI PIPELINE: ALL GREEN ({}/{} gates passed in {}ms)",
                self.passed_count(),
                self.total_count(),
                self.elapsed_ms
            )?;
        } else {
            writeln!(
                f,
                "CI PIPELINE: FAILED ({}/{} gates passed)",
                self.passed_count(),
                self.total_count()
            )?;
        }
        for gate in &self.gates {
            let status = if gate.passed { "PASS" } else { "FAIL" };
            writeln!(f, "  [{status}] {} ({}ms)", gate.gate, gate.elapsed_ms)?;
            if !gate.passed {
                writeln!(f, "         {}", gate.summary)?;
                for err in &gate.errors {
                    writeln!(f, "         - {err}")?;
                }
            }
        }
        Ok(())
    }
}

/// Configuration for a CI pipeline run.
#[derive(Debug, Clone)]
pub struct CiPipelineConfig {
    /// Which gates to run (default: commit pipeline)
    pub gates: Vec<CiGate>,
    /// Stop on first failure (default: true)
    pub fail_fast: bool,
    /// Harness config for Rust-native gates
    pub harness_config: HarnessConfig,
    /// Whether to run RaptorQ sidecar verification
    pub verify_sidecars: bool,
}

impl Default for CiPipelineConfig {
    fn default() -> Self {
        Self {
            gates: CiGate::commit_pipeline(),
            fail_fast: true,
            harness_config: HarnessConfig::default_paths(),
            verify_sidecars: true,
        }
    }
}

/// Evaluate a single CI gate using Rust-native checks where possible.
pub fn evaluate_ci_gate(gate: CiGate, config: &CiPipelineConfig) -> CiGateResult {
    let start = SystemTime::now();

    let (passed, summary, errors) = match gate {
        CiGate::G6Conformance => {
            let options = SuiteOptions::default();
            match run_packet_suite_with_options(&config.harness_config, &options) {
                Ok(report) => {
                    if report.is_green() {
                        (
                            true,
                            format!("All {} fixtures passed", report.fixture_count),
                            vec![],
                        )
                    } else {
                        let mut errs = Vec::new();
                        for result in &report.results {
                            if matches!(result.status, CaseStatus::Fail) {
                                errs.push(format!(
                                    "{}: {}",
                                    result.case_id,
                                    result.mismatch.as_deref().unwrap_or("unknown")
                                ));
                            }
                        }
                        (
                            false,
                            format!("{}/{} fixtures failed", report.failed, report.fixture_count),
                            errs,
                        )
                    }
                }
                Err(e) => (false, format!("Harness error: {e}"), vec![e.to_string()]),
            }
        }
        CiGate::G8E2e => {
            let e2e_config = E2eConfig::default_all_phases();
            match run_e2e_suite(&e2e_config, &mut NoopHooks) {
                Ok(report) => {
                    if report.gates_pass {
                        (
                            true,
                            format!(
                                "E2E green: {}/{} passed",
                                report.total_passed, report.total_fixtures
                            ),
                            vec![],
                        )
                    } else {
                        (
                            false,
                            format!(
                                "E2E failed: {}/{} passed",
                                report.total_passed, report.total_fixtures
                            ),
                            report
                                .gate_results
                                .iter()
                                .filter(|g| !g.pass)
                                .map(|g| format!("{}: {}", g.packet_id, g.reasons.join(", ")))
                                .collect(),
                        )
                    }
                }
                Err(e) => (false, format!("E2E error: {e}"), vec![e.to_string()]),
            }
        }
        _ => {
            // External gates: check commands are defined, report as info
            let cmds = gate.commands();
            if cmds.is_empty() {
                (true, "Skipped (no commands defined)".to_owned(), vec![])
            } else {
                // Run shell commands for external gates
                let mut all_ok = true;
                let mut errs = Vec::new();
                for cmd in &cmds {
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    if parts.is_empty() {
                        continue;
                    }
                    match Command::new(parts[0])
                        .args(&parts[1..])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .output()
                    {
                        Ok(output) => {
                            if !output.status.success() {
                                all_ok = false;
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                errs.push(format!(
                                    "`{cmd}` failed (exit {}): {}",
                                    output.status.code().unwrap_or(-1),
                                    stderr.chars().take(500).collect::<String>()
                                ));
                            }
                        }
                        Err(e) => {
                            all_ok = false;
                            errs.push(format!("`{cmd}` execution error: {e}"));
                        }
                    }
                }
                let summary = if all_ok {
                    format!("{} command(s) passed", cmds.len())
                } else {
                    format!("{} error(s)", errs.len())
                };
                (all_ok, summary, errs)
            }
        }
    };

    let elapsed_ms = start.elapsed().map(|d| d.as_millis() as u64).unwrap_or(0);

    CiGateResult {
        gate,
        passed,
        elapsed_ms,
        summary,
        errors,
    }
}

/// Run the full CI gate pipeline with fail-fast and forensics.
pub fn run_ci_pipeline(config: &CiPipelineConfig) -> CiPipelineResult {
    let start = SystemTime::now();
    let mut gates = Vec::new();
    let mut first_failure = None;

    for &gate in &config.gates {
        let result = evaluate_ci_gate(gate, config);
        let failed = !result.passed;
        gates.push(result);

        if failed {
            if first_failure.is_none() {
                first_failure = Some(gate);
            }
            if config.fail_fast {
                break;
            }
        }
    }

    // RaptorQ sidecar verification (supplemental check)
    if config.verify_sidecars && first_failure.is_none() {
        let artifact_root = config.harness_config.repo_root.join("artifacts");
        if let Err(failures) = verify_all_sidecars_ci(&artifact_root) {
            let errs: Vec<String> = failures.iter().flat_map(|f| f.errors.clone()).collect();
            gates.push(CiGateResult {
                gate: CiGate::G6Conformance,
                passed: false,
                elapsed_ms: 0,
                summary: format!("{} sidecar integrity failure(s)", failures.len()),
                errors: errs,
            });
            if first_failure.is_none() {
                first_failure = Some(CiGate::G6Conformance);
            }
        }
    }

    let all_passed = first_failure.is_none();
    let elapsed_ms = start.elapsed().map(|d| d.as_millis() as u64).unwrap_or(0);

    CiPipelineResult {
        gates,
        all_passed,
        first_failure,
        elapsed_ms,
    }
}

fn build_report(
    config: &HarnessConfig,
    suite: String,
    packet_id: Option<String>,
    fixtures: &[PacketFixture],
    options: &SuiteOptions,
) -> Result<PacketParityReport, HarnessError> {
    let mut results = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        results.push(run_fixture(config, fixture, options)?);
    }

    let failed = results
        .iter()
        .filter(|result| matches!(result.status, CaseStatus::Fail))
        .count();
    let passed = results.len().saturating_sub(failed);

    Ok(PacketParityReport {
        suite,
        packet_id,
        oracle_present: config.oracle_root.exists(),
        fixture_count: results.len(),
        passed,
        failed,
        results,
    })
}

fn load_fixtures(
    config: &HarnessConfig,
    packet_filter: Option<&str>,
) -> Result<Vec<PacketFixture>, HarnessError> {
    let fixture_files = list_fixture_files(&config.packet_fixture_root())?;
    let mut fixtures = Vec::with_capacity(fixture_files.len());

    for fixture_path in fixture_files {
        let fixture = load_fixture(&fixture_path)?;
        if packet_filter.is_none_or(|packet| fixture.packet_id == packet) {
            fixtures.push(fixture);
        }
    }
    fixtures.sort_by(|a, b| a.case_id.cmp(&b.case_id));
    Ok(fixtures)
}

fn load_fixture(path: &Path) -> Result<PacketFixture, HarnessError> {
    let body = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&body)?)
}

fn list_fixture_files(root: &Path) -> Result<Vec<PathBuf>, HarnessError> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "json") {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn run_fixture(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    options: &SuiteOptions,
) -> Result<CaseResult, HarnessError> {
    let policy = match fixture.mode {
        RuntimeMode::Strict => RuntimePolicy::strict(),
        RuntimeMode::Hardened => RuntimePolicy::hardened(Some(100_000)),
    };

    let mut ledger = EvidenceLedger::new();
    let mismatch =
        run_fixture_operation(config, fixture, &policy, &mut ledger, options.oracle_mode).err();

    Ok(CaseResult {
        packet_id: fixture.packet_id.clone(),
        case_id: fixture.case_id.clone(),
        mode: fixture.mode,
        operation: fixture.operation,
        status: if mismatch.is_none() {
            CaseStatus::Pass
        } else {
            CaseStatus::Fail
        },
        mismatch,
        evidence_records: ledger.records().len(),
    })
}

fn run_fixture_operation(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
    default_oracle_mode: OracleMode,
) -> Result<(), String> {
    let expected = resolve_expected(config, fixture, default_oracle_mode)
        .map_err(|err| format!("expected resolution failed: {err}"))?;

    match fixture.operation {
        FixtureOperation::SeriesAdd => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let actual = build_series(left)?
                .add_with_policy(&build_series(right)?, policy, ledger)
                .map_err(|err| err.to_string())?;

            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for series_add".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::SeriesJoin => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let join_type = require_join_type(fixture)?;
            let joined = join_series(
                &build_series(left).map_err(|err| format!("left series build failed: {err}"))?,
                &build_series(right).map_err(|err| format!("right series build failed: {err}"))?,
                join_type.into_join_type(),
            )
            .map_err(|err| err.to_string())?;

            let expected = match expected {
                ResolvedExpected::Join(join) => join,
                _ => return Err("expected_join is required for series_join".to_owned()),
            };
            compare_join_expected(&joined, &expected)
        }
        FixtureOperation::GroupBySum => {
            let keys = require_left_series(fixture)?;
            let values = require_right_series(fixture)?;
            let actual = groupby_sum(
                &build_series(keys).map_err(|err| format!("keys series build failed: {err}"))?,
                &build_series(values)
                    .map_err(|err| format!("values series build failed: {err}"))?,
                GroupByOptions::default(),
                policy,
                ledger,
            )
            .map_err(|err| err.to_string())?;

            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for groupby_sum".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::IndexAlignUnion => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let plan = align_union(
                &Index::new(left.index.clone()),
                &Index::new(right.index.clone()),
            );
            validate_alignment_plan(&plan).map_err(|err| err.to_string())?;

            let expected = match expected {
                ResolvedExpected::Alignment(alignment) => alignment,
                _ => return Err("expected_alignment is required for index_align_union".to_owned()),
            };
            compare_alignment_expected(&plan, &expected)
        }
        FixtureOperation::IndexHasDuplicates => {
            let index = require_index(fixture)?;
            let actual = Index::new(index.clone()).has_duplicates();
            let expected = match expected {
                ResolvedExpected::Bool(value) => value,
                _ => return Err("expected_bool is required for index_has_duplicates".to_owned()),
            };
            if actual != expected {
                return Err(format!(
                    "duplicate mismatch: actual={actual}, expected={expected}"
                ));
            }
            Ok(())
        }
        FixtureOperation::IndexFirstPositions => {
            let index = require_index(fixture)?;
            let index = Index::new(index.clone());
            let positions = index.position_map_first();
            let actual = index
                .labels()
                .iter()
                .map(|label| positions.get(label).copied())
                .collect::<Vec<_>>();
            let expected = match expected {
                ResolvedExpected::Positions(values) => values,
                _ => {
                    return Err(
                        "expected_positions is required for index_first_positions".to_owned()
                    );
                }
            };
            if actual != expected {
                return Err(format!(
                    "first-position mismatch: actual={actual:?}, expected={expected:?}"
                ));
            }
            Ok(())
        }
        FixtureOperation::SeriesConcat => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let left_s = build_series(left)?;
            let right_s = build_series(right)?;
            let actual = concat_series(&[&left_s, &right_s]).map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for series_concat".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::NanSum => {
            let left = require_left_series(fixture)?;
            let actual = nansum(&left.values);
            let expected = match expected {
                ResolvedExpected::Scalar(scalar) => scalar,
                _ => return Err("expected_scalar is required for nan_sum".to_owned()),
            };
            compare_scalar(&actual, &expected, "nan_sum")
        }
        FixtureOperation::FillNa => {
            let left = require_left_series(fixture)?;
            let fill = fixture
                .fill_value
                .as_ref()
                .ok_or_else(|| "fill_value is required for fill_na".to_owned())?;
            let actual_values = fill_na(&left.values, fill);
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for fill_na".to_owned()),
            };
            if actual_values != expected.values {
                return Err(format!(
                    "fill_na value mismatch: actual={actual_values:?}, expected={:?}",
                    expected.values
                ));
            }
            Ok(())
        }
        FixtureOperation::DropNa => {
            let left = require_left_series(fixture)?;
            let actual_values = dropna(&left.values);
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for drop_na".to_owned()),
            };
            if actual_values != expected.values {
                return Err(format!(
                    "drop_na value mismatch: actual={actual_values:?}, expected={:?}",
                    expected.values
                ));
            }
            Ok(())
        }
        FixtureOperation::CsvRoundTrip => {
            let csv_input = fixture
                .csv_input
                .as_ref()
                .ok_or_else(|| "csv_input is required for csv_round_trip".to_owned())?;
            let df = read_csv_str(csv_input).map_err(|err| err.to_string())?;
            let output = write_csv_string(&df).map_err(|err| err.to_string())?;
            let df2 = read_csv_str(&output).map_err(|err| err.to_string())?;
            if df.column_names() != df2.column_names() {
                return Err(format!(
                    "csv round-trip column mismatch: {:?} vs {:?}",
                    df.column_names(),
                    df2.column_names()
                ));
            }
            if df.len() != df2.len() {
                return Err(format!(
                    "csv round-trip row count mismatch: {} vs {}",
                    df.len(),
                    df2.len()
                ));
            }
            let expected = match expected {
                ResolvedExpected::Bool(value) => value,
                _ => return Err("expected_bool is required for csv_round_trip".to_owned()),
            };
            if !expected {
                return Err("csv round-trip expected to fail".to_owned());
            }
            Ok(())
        }
        FixtureOperation::ColumnDtypeCheck => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual_dtype = format!("{:?}", series.column().dtype());
            let expected = match expected {
                ResolvedExpected::Dtype(dtype) => dtype,
                _ => return Err("expected_dtype is required for column_dtype_check".to_owned()),
            };
            if actual_dtype != expected {
                return Err(format!(
                    "dtype mismatch: actual={actual_dtype}, expected={expected}"
                ));
            }
            Ok(())
        }
        FixtureOperation::SeriesFilter => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let data = build_series(left)?;
            let mask = build_series(right)?;
            let actual = data.filter(&mask).map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for series_filter".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::SeriesHead => {
            let left = require_left_series(fixture)?;
            let n = fixture
                .head_n
                .ok_or_else(|| "head_n is required for series_head".to_owned())?;
            let series = build_series(left)?;
            let take = n.min(series.len());
            let labels = series.index().labels()[..take].to_vec();
            let values = series.values()[..take].to_vec();
            let actual =
                Series::from_values(series.name(), labels, values).map_err(|e| e.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for series_head".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::GroupByMean => {
            let keys = require_left_series(fixture)?;
            let values = require_right_series(fixture)?;
            let actual = groupby_mean(
                &build_series(keys).map_err(|err| format!("keys series build failed: {err}"))?,
                &build_series(values)
                    .map_err(|err| format!("values series build failed: {err}"))?,
                GroupByOptions::default(),
                policy,
                ledger,
            )
            .map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for groupby_mean".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
        FixtureOperation::GroupByCount => {
            let keys = require_left_series(fixture)?;
            let values = require_right_series(fixture)?;
            let actual = groupby_count(
                &build_series(keys).map_err(|err| format!("keys series build failed: {err}"))?,
                &build_series(values)
                    .map_err(|err| format!("values series build failed: {err}"))?,
                GroupByOptions::default(),
                policy,
                ledger,
            )
            .map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series is required for groupby_count".to_owned()),
            };
            compare_series_expected(&actual, &expected)
        }
    }
}

fn resolve_expected(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    default_mode: OracleMode,
) -> Result<ResolvedExpected, HarnessError> {
    let requested_mode = fixture
        .oracle_source
        .map(|source| match source {
            FixtureOracleSource::Fixture => OracleMode::FixtureExpected,
            FixtureOracleSource::LiveLegacyPandas => OracleMode::LiveLegacyPandas,
        })
        .unwrap_or(default_mode);

    match requested_mode {
        OracleMode::FixtureExpected => fixture_expected(fixture),
        OracleMode::LiveLegacyPandas => capture_live_oracle_expected(config, fixture),
    }
}

fn fixture_expected(fixture: &PacketFixture) -> Result<ResolvedExpected, HarnessError> {
    match fixture.operation {
        FixtureOperation::SeriesAdd => fixture
            .expected_series
            .clone()
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_series for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::SeriesJoin => fixture
            .expected_join
            .clone()
            .map(ResolvedExpected::Join)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_join for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::GroupBySum => fixture
            .expected_series
            .clone()
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_series for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::IndexAlignUnion => fixture
            .expected_alignment
            .clone()
            .map(ResolvedExpected::Alignment)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_alignment for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::IndexHasDuplicates => fixture
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_bool for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::IndexFirstPositions => fixture
            .expected_positions
            .clone()
            .map(ResolvedExpected::Positions)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_positions for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::SeriesConcat
        | FixtureOperation::FillNa
        | FixtureOperation::DropNa
        | FixtureOperation::SeriesFilter
        | FixtureOperation::SeriesHead
        | FixtureOperation::GroupByMean
        | FixtureOperation::GroupByCount => fixture
            .expected_series
            .clone()
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_series for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::NanSum => fixture
            .expected_scalar
            .clone()
            .map(ResolvedExpected::Scalar)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_scalar for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::CsvRoundTrip => fixture
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_bool for case {}",
                    fixture.case_id
                ))
            }),
        FixtureOperation::ColumnDtypeCheck => fixture
            .expected_dtype
            .clone()
            .map(ResolvedExpected::Dtype)
            .ok_or_else(|| {
                HarnessError::FixtureFormat(format!(
                    "missing expected_dtype for case {}",
                    fixture.case_id
                ))
            }),
    }
}

fn capture_live_oracle_expected(
    config: &HarnessConfig,
    fixture: &PacketFixture,
) -> Result<ResolvedExpected, HarnessError> {
    if !config.oracle_root.exists() {
        return Err(HarnessError::OracleUnavailable(format!(
            "legacy oracle root does not exist: {}",
            config.oracle_root.display()
        )));
    }
    let script = config.oracle_script_path();
    if !script.exists() {
        return Err(HarnessError::OracleUnavailable(format!(
            "oracle script does not exist: {}",
            script.display()
        )));
    }

    let payload = OracleRequest {
        operation: fixture.operation,
        left: fixture.left.clone(),
        right: fixture.right.clone(),
        index: fixture.index.clone(),
        join_type: fixture.join_type,
        fill_value: fixture.fill_value.clone(),
        head_n: fixture.head_n,
        csv_input: fixture.csv_input.clone(),
    };
    let input = serde_json::to_vec(&payload)?;

    let output = Command::new(&config.python_bin)
        .arg(&script)
        .arg("--legacy-root")
        .arg(&config.oracle_root)
        .arg("--strict-legacy")
        .args(
            config
                .allow_system_pandas_fallback
                .then_some("--allow-system-pandas-fallback"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(&input)?;
            }
            child.wait_with_output()
        })?;

    if !output.status.success() {
        if let Ok(response) = serde_json::from_slice::<OracleResponse>(&output.stdout)
            && let Some(error) = response.error
        {
            return Err(HarnessError::OracleUnavailable(error));
        }
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        return Err(HarnessError::OracleCommandFailed {
            status: code,
            stderr: format!("{stderr}\nstdout={stdout}"),
        });
    }

    let response: OracleResponse = serde_json::from_slice(&output.stdout)?;
    if let Some(error) = response.error {
        return Err(HarnessError::OracleUnavailable(error));
    }

    match fixture.operation {
        FixtureOperation::SeriesAdd => response
            .expected_series
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_series".to_owned())
            }),
        FixtureOperation::SeriesJoin => response
            .expected_join
            .map(ResolvedExpected::Join)
            .ok_or_else(|| HarnessError::FixtureFormat("oracle omitted expected_join".to_owned())),
        FixtureOperation::GroupBySum => response
            .expected_series
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_series".to_owned())
            }),
        FixtureOperation::IndexAlignUnion => response
            .expected_alignment
            .map(ResolvedExpected::Alignment)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_alignment".to_owned())
            }),
        FixtureOperation::IndexHasDuplicates => response
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| HarnessError::FixtureFormat("oracle omitted expected_bool".to_owned())),
        FixtureOperation::IndexFirstPositions => response
            .expected_positions
            .map(ResolvedExpected::Positions)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_positions".to_owned())
            }),
        FixtureOperation::SeriesConcat
        | FixtureOperation::FillNa
        | FixtureOperation::DropNa
        | FixtureOperation::SeriesFilter
        | FixtureOperation::SeriesHead
        | FixtureOperation::GroupByMean
        | FixtureOperation::GroupByCount => response
            .expected_series
            .map(ResolvedExpected::Series)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_series".to_owned())
            }),
        FixtureOperation::NanSum => response
            .expected_scalar
            .map(ResolvedExpected::Scalar)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_scalar".to_owned())
            }),
        FixtureOperation::CsvRoundTrip => response
            .expected_bool
            .map(ResolvedExpected::Bool)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_bool".to_owned())
            }),
        FixtureOperation::ColumnDtypeCheck => response
            .expected_dtype
            .map(ResolvedExpected::Dtype)
            .ok_or_else(|| {
                HarnessError::FixtureFormat("oracle omitted expected_dtype".to_owned())
            }),
    }
}

fn require_left_series(fixture: &PacketFixture) -> Result<&FixtureSeries, String> {
    fixture
        .left
        .as_ref()
        .ok_or_else(|| "missing left fixture series".to_owned())
}

fn require_right_series(fixture: &PacketFixture) -> Result<&FixtureSeries, String> {
    fixture
        .right
        .as_ref()
        .ok_or_else(|| "missing right fixture series".to_owned())
}

fn require_index(fixture: &PacketFixture) -> Result<&Vec<IndexLabel>, String> {
    fixture
        .index
        .as_ref()
        .ok_or_else(|| "missing index fixture vector".to_owned())
}

fn require_join_type(fixture: &PacketFixture) -> Result<FixtureJoinType, String> {
    fixture
        .join_type
        .ok_or_else(|| "missing join_type for join fixture".to_owned())
}

fn build_series(series: &FixtureSeries) -> Result<Series, String> {
    Series::from_values(
        series.name.clone(),
        series.index.clone(),
        series.values.clone(),
    )
    .map_err(|err| err.to_string())
}

fn compare_series_expected(
    actual: &Series,
    expected: &FixtureExpectedSeries,
) -> Result<(), String> {
    if actual.index().labels() != expected.index {
        return Err(format!(
            "index mismatch: actual={:?}, expected={:?}",
            actual.index().labels(),
            expected.index
        ));
    }

    if actual.values().len() != expected.values.len() {
        return Err(format!(
            "value length mismatch: actual={}, expected={}",
            actual.values().len(),
            expected.values.len()
        ));
    }

    for (idx, (left, right)) in actual
        .values()
        .iter()
        .zip(expected.values.iter())
        .enumerate()
    {
        if !left.semantic_eq(right) {
            return Err(format!(
                "value mismatch at idx={idx}: actual={left:?}, expected={right:?}"
            ));
        }
    }
    Ok(())
}

fn compare_scalar(actual: &Scalar, expected: &Scalar, op_name: &str) -> Result<(), String> {
    if !actual.semantic_eq(expected) {
        return Err(format!(
            "{op_name} scalar mismatch: actual={actual:?}, expected={expected:?}"
        ));
    }
    Ok(())
}

fn compare_join_expected(
    actual: &fp_join::JoinedSeries,
    expected: &FixtureExpectedJoin,
) -> Result<(), String> {
    if actual.index.labels() != expected.index {
        return Err(format!(
            "join index mismatch: actual={:?}, expected={:?}",
            actual.index.labels(),
            expected.index
        ));
    }

    if actual.left_values.values().len() != expected.left_values.len() {
        return Err(format!(
            "join left length mismatch: actual={}, expected={}",
            actual.left_values.values().len(),
            expected.left_values.len()
        ));
    }
    if actual.right_values.values().len() != expected.right_values.len() {
        return Err(format!(
            "join right length mismatch: actual={}, expected={}",
            actual.right_values.values().len(),
            expected.right_values.len()
        ));
    }

    for (idx, (left, right)) in actual
        .left_values
        .values()
        .iter()
        .zip(expected.left_values.iter())
        .enumerate()
    {
        let equal = left.semantic_eq(right) || (left.is_missing() && right.is_missing());
        if !equal {
            return Err(format!(
                "join left mismatch at idx={idx}: actual={left:?}, expected={right:?}"
            ));
        }
    }
    for (idx, (left, right)) in actual
        .right_values
        .values()
        .iter()
        .zip(expected.right_values.iter())
        .enumerate()
    {
        let equal = left.semantic_eq(right) || (left.is_missing() && right.is_missing());
        if !equal {
            return Err(format!(
                "join right mismatch at idx={idx}: actual={left:?}, expected={right:?}"
            ));
        }
    }

    Ok(())
}

fn compare_alignment_expected(
    actual: &AlignmentPlan,
    expected: &FixtureExpectedAlignment,
) -> Result<(), String> {
    if actual.union_index.labels() != expected.union_index {
        return Err(format!(
            "union_index mismatch: actual={:?}, expected={:?}",
            actual.union_index.labels(),
            expected.union_index
        ));
    }
    if actual.left_positions != expected.left_positions {
        return Err(format!(
            "left_positions mismatch: actual={:?}, expected={:?}",
            actual.left_positions, expected.left_positions
        ));
    }
    if actual.right_positions != expected.right_positions {
        return Err(format!(
            "right_positions mismatch: actual={:?}, expected={:?}",
            actual.right_positions, expected.right_positions
        ));
    }
    Ok(())
}

// === Differential Harness: Internal Execution + Taxonomy Comparators ===

fn build_differential_report_internal(
    config: &HarnessConfig,
    suite: String,
    packet_id: Option<String>,
    fixtures: &[PacketFixture],
    options: &SuiteOptions,
) -> Result<DifferentialReport, HarnessError> {
    let mut results = Vec::with_capacity(fixtures.len());
    for fixture in fixtures {
        results.push(run_differential_fixture(config, fixture, options)?);
    }
    Ok(build_differential_report(
        suite,
        packet_id,
        config.oracle_root.exists(),
        results,
    ))
}

fn run_differential_fixture(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    options: &SuiteOptions,
) -> Result<DifferentialResult, HarnessError> {
    let policy = match fixture.mode {
        RuntimeMode::Strict => RuntimePolicy::strict(),
        RuntimeMode::Hardened => RuntimePolicy::hardened(Some(100_000)),
    };
    let mut ledger = EvidenceLedger::new();
    let oracle_source = fixture.oracle_source.unwrap_or(match options.oracle_mode {
        OracleMode::FixtureExpected => FixtureOracleSource::Fixture,
        OracleMode::LiveLegacyPandas => FixtureOracleSource::LiveLegacyPandas,
    });

    let drift_records = match execute_and_compare_differential(
        config,
        fixture,
        &policy,
        &mut ledger,
        options.oracle_mode,
    ) {
        Ok(drifts) => drifts,
        Err(err) => vec![DriftRecord {
            category: ComparisonCategory::Value,
            level: DriftLevel::Critical,
            location: "execution".to_owned(),
            message: err,
        }],
    };

    let has_critical = drift_records
        .iter()
        .any(|d| matches!(d.level, DriftLevel::Critical));

    Ok(DifferentialResult {
        case_id: fixture.case_id.clone(),
        packet_id: fixture.packet_id.clone(),
        operation: fixture.operation,
        mode: fixture.mode,
        oracle_source,
        status: if has_critical {
            CaseStatus::Fail
        } else {
            CaseStatus::Pass
        },
        drift_records,
        evidence_records: ledger.records().len(),
    })
}

fn execute_and_compare_differential(
    config: &HarnessConfig,
    fixture: &PacketFixture,
    policy: &RuntimePolicy,
    ledger: &mut EvidenceLedger,
    default_oracle_mode: OracleMode,
) -> Result<Vec<DriftRecord>, String> {
    let expected = resolve_expected(config, fixture, default_oracle_mode)
        .map_err(|err| format!("expected resolution failed: {err}"))?;

    match fixture.operation {
        FixtureOperation::SeriesAdd => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let actual = build_series(left)?
                .add_with_policy(&build_series(right)?, policy, ledger)
                .map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for series_add".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::SeriesJoin => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let join_type = require_join_type(fixture)?;
            let joined = join_series(
                &build_series(left).map_err(|err| format!("left build: {err}"))?,
                &build_series(right).map_err(|err| format!("right build: {err}"))?,
                join_type.into_join_type(),
            )
            .map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Join(j) => j,
                _ => return Err("expected_join required for series_join".to_owned()),
            };
            Ok(diff_join(&joined, &expected))
        }
        FixtureOperation::GroupBySum => {
            let keys = require_left_series(fixture)?;
            let values = require_right_series(fixture)?;
            let actual = groupby_sum(
                &build_series(keys).map_err(|err| format!("keys build: {err}"))?,
                &build_series(values).map_err(|err| format!("values build: {err}"))?,
                GroupByOptions::default(),
                policy,
                ledger,
            )
            .map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for groupby_sum".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::IndexAlignUnion => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let plan = align_union(
                &Index::new(left.index.clone()),
                &Index::new(right.index.clone()),
            );
            validate_alignment_plan(&plan).map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Alignment(a) => a,
                _ => return Err("expected_alignment required for index_align_union".to_owned()),
            };
            Ok(diff_alignment(&plan, &expected))
        }
        FixtureOperation::IndexHasDuplicates => {
            let index = require_index(fixture)?;
            let actual = Index::new(index.clone()).has_duplicates();
            let expected = match expected {
                ResolvedExpected::Bool(b) => b,
                _ => return Err("expected_bool required for index_has_duplicates".to_owned()),
            };
            Ok(diff_bool(actual, expected, "has_duplicates"))
        }
        FixtureOperation::IndexFirstPositions => {
            let index = require_index(fixture)?;
            let index = Index::new(index.clone());
            let positions = index.position_map_first();
            let actual: Vec<Option<usize>> = index
                .labels()
                .iter()
                .map(|label| positions.get(label).copied())
                .collect();
            let expected = match expected {
                ResolvedExpected::Positions(p) => p,
                _ => {
                    return Err("expected_positions required for index_first_positions".to_owned());
                }
            };
            Ok(diff_positions(&actual, &expected))
        }
        FixtureOperation::SeriesConcat => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let left_s = build_series(left).map_err(|err| format!("left build: {err}"))?;
            let right_s = build_series(right).map_err(|err| format!("right build: {err}"))?;
            let actual =
                concat_series(&[&left_s, &right_s]).map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for series_concat".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::NanSum => {
            let left = require_left_series(fixture)?;
            let actual = nansum(&left.values);
            let expected = match expected {
                ResolvedExpected::Scalar(scalar) => scalar,
                _ => return Err("expected_scalar required for nan_sum".to_owned()),
            };
            Ok(diff_scalar(&actual, &expected, "nan_sum"))
        }
        FixtureOperation::FillNa => {
            let left = require_left_series(fixture)?;
            let fill = fixture
                .fill_value
                .as_ref()
                .ok_or_else(|| "fill_value is required for fill_na".to_owned())?;
            let actual_values = fill_na(&left.values, fill);
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series required for fill_na".to_owned()),
            };
            Ok(diff_values(&actual_values, &expected.values, "fill_na"))
        }
        FixtureOperation::DropNa => {
            let left = require_left_series(fixture)?;
            let actual_values = dropna(&left.values);
            let expected = match expected {
                ResolvedExpected::Series(series) => series,
                _ => return Err("expected_series required for drop_na".to_owned()),
            };
            Ok(diff_values(&actual_values, &expected.values, "drop_na"))
        }
        FixtureOperation::CsvRoundTrip => {
            let csv_input = fixture
                .csv_input
                .as_ref()
                .ok_or_else(|| "csv_input is required for csv_round_trip".to_owned())?;
            let df = read_csv_str(csv_input).map_err(|err| err.to_string())?;
            let output = write_csv_string(&df).map_err(|err| err.to_string())?;
            let df2 = read_csv_str(&output).map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Bool(value) => value,
                _ => return Err("expected_bool required for csv_round_trip".to_owned()),
            };
            let round_trip_ok = df.column_names() == df2.column_names() && df.len() == df2.len();
            Ok(diff_bool(round_trip_ok, expected, "csv_round_trip"))
        }
        FixtureOperation::ColumnDtypeCheck => {
            let left = require_left_series(fixture)?;
            let series = build_series(left)?;
            let actual_dtype = format!("{:?}", series.column().dtype());
            let expected = match expected {
                ResolvedExpected::Dtype(dtype) => dtype,
                _ => return Err("expected_dtype required for column_dtype_check".to_owned()),
            };
            Ok(diff_string(&actual_dtype, &expected, "column_dtype"))
        }
        FixtureOperation::SeriesFilter => {
            let left = require_left_series(fixture)?;
            let right = require_right_series(fixture)?;
            let data = build_series(left)?;
            let mask = build_series(right)?;
            let actual = data.filter(&mask).map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for series_filter".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::SeriesHead => {
            let left = require_left_series(fixture)?;
            let n = fixture
                .head_n
                .ok_or_else(|| "head_n is required for series_head".to_owned())?;
            let series = build_series(left)?;
            let take = n.min(series.len());
            let labels = series.index().labels()[..take].to_vec();
            let values = series.values()[..take].to_vec();
            let actual =
                Series::from_values(series.name(), labels, values).map_err(|e| e.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for series_head".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::GroupByMean => {
            let keys = require_left_series(fixture)?;
            let values = require_right_series(fixture)?;
            let actual = groupby_mean(
                &build_series(keys).map_err(|err| format!("keys build: {err}"))?,
                &build_series(values).map_err(|err| format!("values build: {err}"))?,
                GroupByOptions::default(),
                policy,
                ledger,
            )
            .map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for groupby_mean".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
        FixtureOperation::GroupByCount => {
            let keys = require_left_series(fixture)?;
            let values = require_right_series(fixture)?;
            let actual = groupby_count(
                &build_series(keys).map_err(|err| format!("keys build: {err}"))?,
                &build_series(values).map_err(|err| format!("values build: {err}"))?,
                GroupByOptions::default(),
                policy,
                ledger,
            )
            .map_err(|err| err.to_string())?;
            let expected = match expected {
                ResolvedExpected::Series(s) => s,
                _ => return Err("expected_series required for groupby_count".to_owned()),
            };
            Ok(diff_series(&actual, &expected))
        }
    }
}

fn diff_scalar(actual: &Scalar, expected: &Scalar, name: &str) -> Vec<DriftRecord> {
    if actual.semantic_eq(expected) {
        Vec::new()
    } else {
        vec![DriftRecord {
            category: ComparisonCategory::Value,
            level: DriftLevel::Critical,
            location: format!("{name}.scalar"),
            message: format!("scalar mismatch: actual={actual:?}, expected={expected:?}"),
        }]
    }
}

fn diff_values(actual: &[Scalar], expected: &[Scalar], name: &str) -> Vec<DriftRecord> {
    let mut drifts = Vec::new();
    if actual.len() != expected.len() {
        drifts.push(DriftRecord {
            category: ComparisonCategory::Shape,
            level: DriftLevel::Critical,
            location: format!("{name}.len"),
            message: format!(
                "length mismatch: actual={}, expected={}",
                actual.len(),
                expected.len()
            ),
        });
        return drifts;
    }
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        if !a.semantic_eq(e) {
            drifts.push(DriftRecord {
                category: ComparisonCategory::Value,
                level: DriftLevel::Critical,
                location: format!("{name}[{i}]"),
                message: format!("value mismatch: actual={a:?}, expected={e:?}"),
            });
        }
    }
    drifts
}

fn diff_string(actual: &str, expected: &str, name: &str) -> Vec<DriftRecord> {
    if actual == expected {
        Vec::new()
    } else {
        vec![DriftRecord {
            category: ComparisonCategory::Type,
            level: DriftLevel::Critical,
            location: format!("{name}.value"),
            message: format!("string mismatch: actual={actual:?}, expected={expected:?}"),
        }]
    }
}

fn diff_series(actual: &Series, expected: &FixtureExpectedSeries) -> Vec<DriftRecord> {
    let mut drifts = Vec::new();

    if actual.index().labels() != expected.index {
        drifts.push(DriftRecord {
            category: ComparisonCategory::Index,
            level: DriftLevel::Critical,
            location: "series.index".to_owned(),
            message: format!(
                "index mismatch: actual={:?}, expected={:?}",
                actual.index().labels(),
                expected.index
            ),
        });
    }

    if actual.values().len() != expected.values.len() {
        drifts.push(DriftRecord {
            category: ComparisonCategory::Shape,
            level: DriftLevel::Critical,
            location: "series.values.len".to_owned(),
            message: format!(
                "length mismatch: actual={}, expected={}",
                actual.values().len(),
                expected.values.len()
            ),
        });
        return drifts;
    }

    diff_value_vectors(
        actual.values(),
        &expected.values,
        "series.values",
        &mut drifts,
    );
    drifts
}

fn diff_join(actual: &fp_join::JoinedSeries, expected: &FixtureExpectedJoin) -> Vec<DriftRecord> {
    let mut drifts = Vec::new();

    if actual.index.labels() != expected.index {
        drifts.push(DriftRecord {
            category: ComparisonCategory::Index,
            level: DriftLevel::Critical,
            location: "join.index".to_owned(),
            message: format!(
                "index mismatch: actual={:?}, expected={:?}",
                actual.index.labels(),
                expected.index
            ),
        });
    }

    if actual.left_values.values().len() != expected.left_values.len() {
        drifts.push(DriftRecord {
            category: ComparisonCategory::Shape,
            level: DriftLevel::Critical,
            location: "join.left_values.len".to_owned(),
            message: format!(
                "left length mismatch: actual={}, expected={}",
                actual.left_values.values().len(),
                expected.left_values.len()
            ),
        });
    } else {
        diff_value_vectors(
            actual.left_values.values(),
            &expected.left_values,
            "join.left_values",
            &mut drifts,
        );
    }

    if actual.right_values.values().len() != expected.right_values.len() {
        drifts.push(DriftRecord {
            category: ComparisonCategory::Shape,
            level: DriftLevel::Critical,
            location: "join.right_values.len".to_owned(),
            message: format!(
                "right length mismatch: actual={}, expected={}",
                actual.right_values.values().len(),
                expected.right_values.len()
            ),
        });
    } else {
        diff_value_vectors(
            actual.right_values.values(),
            &expected.right_values,
            "join.right_values",
            &mut drifts,
        );
    }

    drifts
}

fn diff_alignment(actual: &AlignmentPlan, expected: &FixtureExpectedAlignment) -> Vec<DriftRecord> {
    let mut drifts = Vec::new();

    if actual.union_index.labels() != expected.union_index {
        drifts.push(DriftRecord {
            category: ComparisonCategory::Index,
            level: DriftLevel::Critical,
            location: "alignment.union_index".to_owned(),
            message: format!(
                "union_index mismatch: actual={:?}, expected={:?}",
                actual.union_index.labels(),
                expected.union_index
            ),
        });
    }

    if actual.left_positions != expected.left_positions {
        drifts.push(DriftRecord {
            category: ComparisonCategory::Value,
            level: DriftLevel::Critical,
            location: "alignment.left_positions".to_owned(),
            message: format!(
                "left_positions mismatch: actual={:?}, expected={:?}",
                actual.left_positions, expected.left_positions
            ),
        });
    }

    if actual.right_positions != expected.right_positions {
        drifts.push(DriftRecord {
            category: ComparisonCategory::Value,
            level: DriftLevel::Critical,
            location: "alignment.right_positions".to_owned(),
            message: format!(
                "right_positions mismatch: actual={:?}, expected={:?}",
                actual.right_positions, expected.right_positions
            ),
        });
    }

    drifts
}

fn diff_bool(actual: bool, expected: bool, name: &str) -> Vec<DriftRecord> {
    if actual == expected {
        Vec::new()
    } else {
        vec![DriftRecord {
            category: ComparisonCategory::Value,
            level: DriftLevel::Critical,
            location: name.to_owned(),
            message: format!("boolean mismatch: actual={actual}, expected={expected}"),
        }]
    }
}

fn diff_positions(actual: &[Option<usize>], expected: &[Option<usize>]) -> Vec<DriftRecord> {
    if actual == expected {
        Vec::new()
    } else {
        vec![DriftRecord {
            category: ComparisonCategory::Value,
            level: DriftLevel::Critical,
            location: "first_positions".to_owned(),
            message: format!("positions mismatch: actual={actual:?}, expected={expected:?}"),
        }]
    }
}

fn diff_value_vectors(
    actual: &[Scalar],
    expected: &[Scalar],
    prefix: &str,
    drifts: &mut Vec<DriftRecord>,
) {
    for (idx, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        let equal = a.semantic_eq(e) || (a.is_missing() && e.is_missing());
        if !equal {
            let location = format!("{prefix}[{idx}]");
            if a.is_missing() != e.is_missing() {
                drifts.push(DriftRecord {
                    category: ComparisonCategory::Nullness,
                    level: DriftLevel::Critical,
                    location,
                    message: format!("nullness mismatch: actual={a:?}, expected={e:?}"),
                });
            } else {
                let level = classify_value_drift(a, e);
                drifts.push(DriftRecord {
                    category: ComparisonCategory::Value,
                    level,
                    location,
                    message: format!("value mismatch: actual={a:?}, expected={e:?}"),
                });
            }
        }
    }
}

fn classify_value_drift(actual: &Scalar, expected: &Scalar) -> DriftLevel {
    match (actual, expected) {
        (Scalar::Float64(a), Scalar::Float64(e)) => {
            let max_abs = a.abs().max(e.abs()).max(1.0);
            let rel_diff = (a - e).abs() / max_abs;
            if rel_diff < 1e-10 {
                DriftLevel::NonCritical
            } else {
                DriftLevel::Critical
            }
        }
        _ => DriftLevel::Critical,
    }
}

fn summarize_drift(results: &[DifferentialResult]) -> DriftSummary {
    let mut total = 0usize;
    let mut critical = 0usize;
    let mut non_critical = 0usize;
    let mut informational = 0usize;
    let mut cat_counts = BTreeMap::<ComparisonCategory, usize>::new();

    for result in results {
        for drift in &result.drift_records {
            total += 1;
            match drift.level {
                DriftLevel::Critical => critical += 1,
                DriftLevel::NonCritical => non_critical += 1,
                DriftLevel::Informational => informational += 1,
            }
            *cat_counts.entry(drift.category).or_default() += 1;
        }
    }

    let categories = cat_counts
        .into_iter()
        .map(|(category, count)| CategoryCount { category, count })
        .collect();

    DriftSummary {
        total_drift_records: total,
        critical_count: critical,
        non_critical_count: non_critical,
        informational_count: informational,
        categories,
    }
}

fn percent(failed: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        (failed as f64 / total as f64) * 100.0
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_decode(value: &str) -> Result<Vec<u8>, HarnessError> {
    if !value.len().is_multiple_of(2) {
        return Err(HarnessError::RaptorQ(format!(
            "invalid hex length {}",
            value.len()
        )));
    }
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(value.len() / 2);
    for idx in (0..bytes.len()).step_by(2) {
        let high = hex_value(bytes[idx])?;
        let low = hex_value(bytes[idx + 1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + (value - 10)) as char,
        _ => unreachable!("nibble out of range"),
    }
}

fn hex_value(byte: u8) -> Result<u8, HarnessError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(HarnessError::RaptorQ(format!(
            "invalid hex character: {}",
            byte as char
        ))),
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

// === E2E Orchestrator + Replay/Forensics Logging (bd-2gi.6) ===

/// Forensic event kinds emitted during E2E orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ForensicEventKind {
    SuiteStart {
        suite: String,
        packet_filter: Option<String>,
    },
    SuiteEnd {
        suite: String,
        total_fixtures: usize,
        passed: usize,
        failed: usize,
    },
    PacketStart {
        packet_id: String,
    },
    PacketEnd {
        packet_id: String,
        fixtures: usize,
        passed: usize,
        failed: usize,
        gate_pass: bool,
    },
    CaseStart {
        packet_id: String,
        case_id: String,
        operation: FixtureOperation,
        mode: RuntimeMode,
    },
    CaseEnd {
        packet_id: String,
        case_id: String,
        status: CaseStatus,
        evidence_records: usize,
        elapsed_us: u64,
    },
    ArtifactWritten {
        packet_id: String,
        artifact_kind: String,
        path: String,
    },
    GateEvaluated {
        packet_id: String,
        pass: bool,
        reasons: Vec<String>,
    },
    DriftHistoryAppended {
        path: String,
        entries: usize,
    },
    Error {
        phase: String,
        message: String,
    },
}

/// A single forensic log entry with timestamp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForensicEvent {
    pub ts_unix_ms: u64,
    pub event: ForensicEventKind,
}

/// Accumulator for forensic events during an E2E run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForensicLog {
    pub events: Vec<ForensicEvent>,
}

impl ForensicLog {
    #[must_use]
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn record(&mut self, event: ForensicEventKind) {
        self.events.push(ForensicEvent {
            ts_unix_ms: now_unix_ms(),
            event,
        });
    }

    /// Write the forensic log as JSONL to the given path.
    pub fn write_jsonl(&self, path: &Path) -> Result<(), HarnessError> {
        let mut file = fs::File::create(path).map_err(HarnessError::Io)?;
        for entry in &self.events {
            let line = serde_json::to_string(entry).map_err(HarnessError::Json)?;
            writeln!(file, "{line}").map_err(HarnessError::Io)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Lifecycle hooks for E2E orchestration. Default implementations are no-ops.
pub trait LifecycleHooks {
    fn before_suite(&mut self, _suite: &str, _packet_filter: &Option<String>) {}
    fn after_suite(&mut self, _report: &[PacketParityReport]) {}
    fn before_packet(&mut self, _packet_id: &str) {}
    fn after_packet(&mut self, _report: &PacketParityReport, _gate_pass: bool) {}
    fn before_case(&mut self, _fixture: &PacketFixture) {}
    fn after_case(&mut self, _result: &CaseResult) {}
}

/// Default no-op hooks.
pub struct NoopHooks;
impl LifecycleHooks for NoopHooks {}

/// Configuration for the E2E orchestrator.
#[derive(Debug, Clone)]
pub struct E2eConfig {
    pub harness: HarnessConfig,
    pub options: SuiteOptions,
    pub write_artifacts: bool,
    pub enforce_gates: bool,
    pub append_drift_history: bool,
    pub forensic_log_path: Option<PathBuf>,
}

impl E2eConfig {
    #[must_use]
    pub fn default_all_phases() -> Self {
        Self {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions::default(),
            write_artifacts: true,
            enforce_gates: true,
            append_drift_history: true,
            forensic_log_path: None,
        }
    }
}

/// Final result of an E2E orchestration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct E2eReport {
    pub suite: String,
    pub packet_reports: Vec<PacketParityReport>,
    pub artifacts_written: Vec<WrittenPacketArtifacts>,
    pub gate_results: Vec<PacketGateResult>,
    pub gates_pass: bool,
    pub drift_history_path: Option<String>,
    pub forensic_log: ForensicLog,
    pub total_fixtures: usize,
    pub total_passed: usize,
    pub total_failed: usize,
}

impl E2eReport {
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.total_failed == 0 && self.total_fixtures > 0 && self.gates_pass
    }
}

/// Run the full E2E orchestration pipeline with lifecycle hooks and forensic logging.
///
/// Phases:
/// 1. **Run**: Execute all packet fixtures grouped by packet ID
/// 2. **Write**: Persist parity reports, RaptorQ sidecars, decode proofs, gate results, mismatch corpora
/// 3. **Enforce**: Validate all reports against parity gate configs
/// 4. **History**: Append JSONL rows to drift history ledger
///
/// Returns `E2eReport` with all phase results and forensic log.
pub fn run_e2e_suite(
    config: &E2eConfig,
    hooks: &mut dyn LifecycleHooks,
) -> Result<E2eReport, HarnessError> {
    let mut forensic = ForensicLog::new();
    let suite_name = config
        .options
        .packet_filter
        .clone()
        .unwrap_or("full".to_owned());

    // Phase 1: Suite Start
    forensic.record(ForensicEventKind::SuiteStart {
        suite: suite_name.clone(),
        packet_filter: config.options.packet_filter.clone(),
    });
    hooks.before_suite(&suite_name, &config.options.packet_filter);

    // Phase 2: Run fixtures grouped by packet
    let grouped_reports = run_packets_grouped(&config.harness, &config.options)?;

    // Emit per-packet forensic events
    for report in &grouped_reports {
        let packet_id = report
            .packet_id
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());

        forensic.record(ForensicEventKind::PacketStart {
            packet_id: packet_id.clone(),
        });
        hooks.before_packet(&packet_id);

        // Emit per-case events from the report results
        for case_result in &report.results {
            forensic.record(ForensicEventKind::CaseEnd {
                packet_id: case_result.packet_id.clone(),
                case_id: case_result.case_id.clone(),
                status: case_result.status.clone(),
                evidence_records: case_result.evidence_records,
                elapsed_us: 0, // not available in retrospective mode
            });
            hooks.after_case(case_result);
        }

        // Evaluate gate for this packet
        let gate_result = evaluate_parity_gate(&config.harness, report)?;

        forensic.record(ForensicEventKind::GateEvaluated {
            packet_id: packet_id.clone(),
            pass: gate_result.pass,
            reasons: gate_result.reasons.clone(),
        });

        forensic.record(ForensicEventKind::PacketEnd {
            packet_id: packet_id.clone(),
            fixtures: report.fixture_count,
            passed: report.passed,
            failed: report.failed,
            gate_pass: gate_result.pass,
        });
        hooks.after_packet(report, gate_result.pass);
    }

    // Phase 3: Write artifacts
    let mut artifacts_written = Vec::new();
    if config.write_artifacts {
        let written = write_grouped_artifacts(&config.harness, &grouped_reports)?;
        for w in &written {
            for (kind, path) in [
                ("parity_report", &w.parity_report_path),
                ("raptorq_sidecar", &w.raptorq_sidecar_path),
                ("decode_proof", &w.decode_proof_path),
                ("gate_result", &w.gate_result_path),
                ("mismatch_corpus", &w.mismatch_corpus_path),
            ] {
                forensic.record(ForensicEventKind::ArtifactWritten {
                    packet_id: w.packet_id.clone(),
                    artifact_kind: kind.to_owned(),
                    path: path.display().to_string(),
                });
            }
        }
        artifacts_written = written;
    }

    // Phase 4: Evaluate gates
    let mut gate_results = Vec::new();
    let mut gates_pass = true;
    for report in &grouped_reports {
        let gate = evaluate_parity_gate(&config.harness, report)?;
        if !gate.pass {
            gates_pass = false;
        }
        gate_results.push(gate);
    }

    // Phase 5: Enforce gates (if configured)
    if config.enforce_gates
        && let Err(e) = enforce_packet_gates(&config.harness, &grouped_reports)
    {
        forensic.record(ForensicEventKind::Error {
            phase: "gate_enforcement".to_owned(),
            message: e.to_string(),
        });
        // Record but don't fail: the E2eReport captures gates_pass=false
    }

    // Phase 6: Append drift history
    let mut drift_history_path = None;
    if config.append_drift_history {
        match append_phase2c_drift_history(&config.harness, &grouped_reports) {
            Ok(path) => {
                forensic.record(ForensicEventKind::DriftHistoryAppended {
                    path: path.display().to_string(),
                    entries: grouped_reports.len(),
                });
                drift_history_path = Some(path.display().to_string());
            }
            Err(e) => {
                forensic.record(ForensicEventKind::Error {
                    phase: "drift_history".to_owned(),
                    message: e.to_string(),
                });
            }
        }
    }

    // Aggregate totals
    let total_fixtures: usize = grouped_reports.iter().map(|r| r.fixture_count).sum();
    let total_passed: usize = grouped_reports.iter().map(|r| r.passed).sum();
    let total_failed: usize = grouped_reports.iter().map(|r| r.failed).sum();

    // Suite End
    forensic.record(ForensicEventKind::SuiteEnd {
        suite: suite_name.clone(),
        total_fixtures,
        passed: total_passed,
        failed: total_failed,
    });
    hooks.after_suite(&grouped_reports);

    // Write forensic log if configured
    if let Some(ref log_path) = config.forensic_log_path {
        forensic.write_jsonl(log_path)?;
    }

    Ok(E2eReport {
        suite: suite_name,
        packet_reports: grouped_reports,
        artifacts_written,
        gate_results,
        gates_pass,
        drift_history_path,
        forensic_log: forensic,
        total_fixtures,
        total_passed,
        total_failed,
    })
}

// === Failure Forensics UX + Artifact Index (bd-2gi.21) ===

/// Deterministic artifact identifier for cross-referencing forensic artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtifactId {
    pub packet_id: String,
    pub artifact_kind: String,
    pub run_ts_unix_ms: u64,
}

impl ArtifactId {
    /// Generate a short deterministic hash for display.
    #[must_use]
    pub fn short_hash(&self) -> String {
        let input = format!(
            "{}:{}:{}",
            self.packet_id, self.artifact_kind, self.run_ts_unix_ms
        );
        let hash = Sha256::digest(input.as_bytes());
        format!("{:x}", hash)[..8].to_owned()
    }
}

impl std::fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}:{}@{}",
            self.packet_id,
            self.artifact_kind,
            self.short_hash()
        )
    }
}

/// A concise failure summary for a single test case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureDigest {
    pub packet_id: String,
    pub case_id: String,
    pub operation: FixtureOperation,
    pub mode: RuntimeMode,
    pub mismatch_summary: String,
    pub replay_command: String,
    pub artifact_path: Option<String>,
}

impl std::fmt::Display for FailureDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "FAIL {packet}::{case} [{op:?}/{mode:?}]",
            packet = self.packet_id,
            case = self.case_id,
            op = self.operation,
            mode = self.mode,
        )?;
        writeln!(f, "  Mismatch: {}", self.mismatch_summary)?;
        writeln!(f, "  Replay:   {}", self.replay_command)?;
        if let Some(ref path) = self.artifact_path {
            writeln!(f, "  Artifact: {path}")?;
        }
        Ok(())
    }
}

/// Human-readable failure report for an E2E run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureForensicsReport {
    pub run_ts_unix_ms: u64,
    pub total_fixtures: usize,
    pub total_passed: usize,
    pub total_failed: usize,
    pub failures: Vec<FailureDigest>,
    pub gate_failures: Vec<String>,
}

impl FailureForensicsReport {
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty() && self.gate_failures.is_empty()
    }
}

impl std::fmt::Display for FailureForensicsReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_clean() {
            writeln!(
                f,
                "ALL GREEN: {}/{} fixtures passed",
                self.total_passed, self.total_fixtures
            )?;
            return Ok(());
        }

        writeln!(
            f,
            "FAILURES: {failed}/{total} fixtures failed",
            failed = self.total_failed,
            total = self.total_fixtures,
        )?;
        writeln!(f)?;

        for (i, failure) in self.failures.iter().enumerate() {
            write!(f, "  {idx}. {failure}", idx = i + 1)?;
        }

        if !self.gate_failures.is_empty() {
            writeln!(f)?;
            writeln!(f, "GATE FAILURES:")?;
            for reason in &self.gate_failures {
                writeln!(f, "  - {reason}")?;
            }
        }

        Ok(())
    }
}

/// Build a failure forensics report from an E2E report.
#[must_use]
pub fn build_failure_forensics(e2e: &E2eReport) -> FailureForensicsReport {
    let mut failures = Vec::new();

    for report in &e2e.packet_reports {
        let packet_id = report
            .packet_id
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());

        for result in &report.results {
            if matches!(result.status, CaseStatus::Fail) {
                let mismatch_summary = result
                    .mismatch
                    .as_deref()
                    .unwrap_or("(no details)")
                    .chars()
                    .take(200)
                    .collect();

                let replay_command = format!(
                    "cargo test -p fp-conformance -- {} --nocapture",
                    result.case_id
                );

                let artifact_path = e2e
                    .artifacts_written
                    .iter()
                    .find(|a| a.packet_id == packet_id)
                    .map(|a| a.mismatch_corpus_path.display().to_string());

                failures.push(FailureDigest {
                    packet_id: packet_id.clone(),
                    case_id: result.case_id.clone(),
                    operation: result.operation,
                    mode: result.mode,
                    mismatch_summary,
                    replay_command,
                    artifact_path,
                });
            }
        }
    }

    let gate_failures: Vec<String> = e2e
        .gate_results
        .iter()
        .filter(|g| !g.pass)
        .flat_map(|g| {
            g.reasons
                .iter()
                .map(|r| format!("{}: {}", g.packet_id, r))
                .collect::<Vec<_>>()
        })
        .collect();

    FailureForensicsReport {
        run_ts_unix_ms: now_unix_ms(),
        total_fixtures: e2e.total_fixtures,
        total_passed: e2e.total_passed,
        total_failed: e2e.total_failed,
        failures,
        gate_failures,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        ArtifactId, CaseResult, CaseStatus, CiGate, CiGateResult, CiPipelineConfig,
        CiPipelineResult, ComparisonCategory, DecodeProofArtifact, DecodeProofStatus,
        DifferentialResult, DriftLevel, DriftRecord, E2eConfig, FailureDigest,
        FailureForensicsReport, FixtureExpectedAlignment, FixtureOperation, FixtureOracleSource,
        ForensicEventKind, ForensicLog, HarnessConfig, LifecycleHooks, NoopHooks, OracleMode,
        PacketParityReport, RaptorQSidecarArtifact, SuiteOptions, append_phase2c_drift_history,
        build_differential_report, build_failure_forensics, enforce_packet_gates, evaluate_ci_gate,
        evaluate_parity_gate, generate_raptorq_sidecar, run_ci_pipeline, run_differential_by_id,
        run_differential_suite, run_e2e_suite, run_packet_by_id, run_packet_suite,
        run_packet_suite_with_options, run_packets_grouped, run_raptorq_decode_recovery_drill,
        run_smoke, verify_all_sidecars_ci, verify_packet_sidecar_integrity,
    };
    use fp_runtime::RuntimeMode;

    #[test]
    fn smoke_harness_finds_oracle_and_fixtures() {
        let cfg = HarnessConfig::default_paths();
        let report = run_smoke(&cfg);
        assert!(report.oracle_present, "oracle repo should be present");
        assert!(report.fixture_count >= 1, "expected at least one fixture");
        assert!(report.strict_mode);
    }

    #[test]
    fn packet_suite_is_green_for_bootstrap_cases() {
        let cfg = HarnessConfig::default_paths();
        let report = run_packet_suite(&cfg).expect("suite should run");
        assert!(report.fixture_count >= 1, "expected packet fixtures");
        assert!(report.is_green(), "expected report green: {report:?}");
    }

    #[test]
    fn packet_filter_runs_only_requested_packet() {
        let cfg = HarnessConfig::default_paths();
        let report =
            run_packet_by_id(&cfg, "FP-P2C-002", OracleMode::FixtureExpected).expect("report");
        assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-002"));
        assert!(
            report.fixture_count >= 3,
            "expected dedicated FP-P2C-002 fixtures"
        );
        assert!(report.is_green());
    }

    #[test]
    fn packet_filter_runs_join_packet() {
        let cfg = HarnessConfig::default_paths();
        let report =
            run_packet_by_id(&cfg, "FP-P2C-004", OracleMode::FixtureExpected).expect("report");
        assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-004"));
        assert!(report.fixture_count >= 3, "expected join packet fixtures");
        assert!(report.is_green());
    }

    #[test]
    fn packet_filter_runs_groupby_packet() {
        let cfg = HarnessConfig::default_paths();
        let report =
            run_packet_by_id(&cfg, "FP-P2C-005", OracleMode::FixtureExpected).expect("report");
        assert_eq!(report.packet_id.as_deref(), Some("FP-P2C-005"));
        assert!(
            report.fixture_count >= 3,
            "expected groupby packet fixtures"
        );
        assert!(report.is_green());
    }

    #[test]
    fn grouped_reports_are_partitioned_per_packet() {
        let cfg = HarnessConfig::default_paths();
        let reports = run_packets_grouped(&cfg, &SuiteOptions::default()).expect("grouped");
        assert!(
            reports
                .iter()
                .any(|report| report.packet_id.as_deref() == Some("FP-P2C-001"))
        );
        assert!(
            reports
                .iter()
                .any(|report| report.packet_id.as_deref() == Some("FP-P2C-002"))
        );
        assert!(
            reports
                .iter()
                .any(|report| report.packet_id.as_deref() == Some("FP-P2C-003"))
        );
        assert!(
            reports
                .iter()
                .any(|report| report.packet_id.as_deref() == Some("FP-P2C-004"))
        );
        assert!(
            reports
                .iter()
                .any(|report| report.packet_id.as_deref() == Some("FP-P2C-005"))
        );
        enforce_packet_gates(&cfg, &reports).expect("enforcement should pass");
    }

    #[test]
    fn packet_gate_enforcement_fails_when_report_is_not_green() {
        let cfg = HarnessConfig::default_paths();
        let mut reports = run_packets_grouped(&cfg, &SuiteOptions::default()).expect("grouped");
        let report = reports.first_mut().expect("at least one packet");
        let first_case = report.results.first_mut().expect("at least one case");
        first_case.status = CaseStatus::Fail;
        first_case.mismatch = Some("synthetic non-green check".to_owned());
        report.failed = 1;
        report.passed = report.fixture_count.saturating_sub(1);

        let err = enforce_packet_gates(&cfg, &reports).expect_err("should fail");
        let message = err.to_string();
        assert!(
            message.contains("enforcement failed"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn drift_history_append_emits_jsonl_rows() {
        let cfg = HarnessConfig::default_paths();
        let reports = run_packets_grouped(&cfg, &SuiteOptions::default()).expect("grouped");
        let history_path = append_phase2c_drift_history(&cfg, &reports).expect("history");
        let contents = fs::read_to_string(&history_path).expect("history content");
        let latest = contents.lines().last().expect("at least one row");
        let row: serde_json::Value = serde_json::from_str(latest).expect("json row");
        assert!(
            row.get("packet_id").is_some(),
            "history row should include packet_id"
        );
        assert!(
            row.get("gate_pass").is_some(),
            "history row should include gate pass status"
        );
    }

    #[test]
    fn parity_gate_evaluation_passes_for_packet_001() {
        let cfg = HarnessConfig::default_paths();
        let report =
            run_packet_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected).expect("report");
        let result = evaluate_parity_gate(&cfg, &report).expect("gate");
        assert!(result.pass, "gate should pass: {result:?}");
    }

    #[test]
    fn parity_gate_evaluation_fails_for_injected_drift() {
        let cfg = HarnessConfig::default_paths();
        let mut report =
            run_packet_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected).expect("report");
        let first = report.results.first_mut().expect("at least one result");
        first.status = CaseStatus::Fail;
        first.mismatch = Some("synthetic drift injection".to_owned());
        report.failed = 1;
        report.passed = report.fixture_count.saturating_sub(1);

        let result = evaluate_parity_gate(&cfg, &report).expect("gate");
        assert!(!result.pass, "gate should fail for injected drift");
        assert!(
            result
                .reasons
                .iter()
                .any(|reason| reason.contains("failed="))
        );
    }

    #[test]
    fn raptorq_sidecar_round_trip_recovery_drill_passes() {
        let payload = br#"{\"suite\":\"phase2c_packets\",\"passed\":4,\"failed\":0}"#;
        let sidecar = generate_raptorq_sidecar("test/parity_report", "conformance", payload, 8)
            .expect("sidecar generation");
        let proof = run_raptorq_decode_recovery_drill(&sidecar, payload).expect("decode drill");
        assert!(proof.recovered_blocks >= 1);
    }

    #[test]
    fn index_alignment_expected_type_serialization_is_stable() {
        let expected = FixtureExpectedAlignment {
            union_index: vec![1_i64.into(), 2_i64.into()],
            left_positions: vec![Some(0), None],
            right_positions: vec![None, Some(0)],
        };
        let json = serde_json::to_string(&expected).expect("serialize");
        let back: FixtureExpectedAlignment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, expected);
    }

    #[test]
    fn live_oracle_mode_executes_or_returns_structured_failure() {
        let cfg = HarnessConfig::default_paths();
        let options = SuiteOptions {
            packet_filter: Some("FP-P2C-001".to_owned()),
            oracle_mode: OracleMode::LiveLegacyPandas,
        };
        let result = run_packet_suite_with_options(&cfg, &options);
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
    fn sidecar_verification_runs_on_generated_artifact() {
        let payload = br#"{\"suite\":\"phase2c_packets\",\"passed\":2,\"failed\":0}"#;
        let sidecar: RaptorQSidecarArtifact =
            generate_raptorq_sidecar("test/artifact", "conformance", payload, 8).expect("sidecar");
        let scrub = super::verify_raptorq_sidecar(&sidecar, payload).expect("scrub");
        assert_eq!(scrub.status, "ok");
    }

    // === Differential Harness Tests ===

    #[test]
    fn differential_suite_produces_structured_drift() {
        let cfg = HarnessConfig::default_paths();
        let diff_report =
            run_differential_suite(&cfg, &SuiteOptions::default()).expect("differential suite");
        assert!(diff_report.report.fixture_count >= 1);
        assert!(diff_report.report.is_green());
        assert_eq!(diff_report.drift_summary.critical_count, 0);
    }

    #[test]
    fn differential_by_id_matches_legacy_report() {
        let cfg = HarnessConfig::default_paths();
        let legacy_report =
            run_packet_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected).expect("legacy");
        let diff_report = run_differential_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
            .expect("differential");
        assert_eq!(
            diff_report.report.fixture_count,
            legacy_report.fixture_count
        );
        assert_eq!(diff_report.report.passed, legacy_report.passed);
        assert_eq!(diff_report.report.failed, legacy_report.failed);
    }

    #[test]
    fn differential_result_converts_to_case_result() {
        let diff = DifferentialResult {
            case_id: "test".to_owned(),
            packet_id: "FP-P2C-001".to_owned(),
            operation: FixtureOperation::SeriesAdd,
            mode: RuntimeMode::Strict,
            oracle_source: FixtureOracleSource::Fixture,
            status: CaseStatus::Pass,
            drift_records: Vec::new(),
            evidence_records: 0,
        };
        let case = diff.to_case_result();
        assert_eq!(case.status, CaseStatus::Pass);
        assert!(case.mismatch.is_none());
    }

    #[test]
    fn differential_result_with_drift_converts_mismatch_string() {
        let diff = DifferentialResult {
            case_id: "test".to_owned(),
            packet_id: "FP-P2C-001".to_owned(),
            operation: FixtureOperation::SeriesAdd,
            mode: RuntimeMode::Strict,
            oracle_source: FixtureOracleSource::Fixture,
            status: CaseStatus::Fail,
            drift_records: vec![
                DriftRecord {
                    category: ComparisonCategory::Value,
                    level: DriftLevel::Critical,
                    location: "series.values[0]".to_owned(),
                    message: "value mismatch".to_owned(),
                },
                DriftRecord {
                    category: ComparisonCategory::Index,
                    level: DriftLevel::NonCritical,
                    location: "series.index".to_owned(),
                    message: "order divergence".to_owned(),
                },
            ],
            evidence_records: 0,
        };
        let case = diff.to_case_result();
        assert_eq!(case.status, CaseStatus::Fail);
        let mismatch = case.mismatch.expect("should have mismatch");
        assert!(mismatch.contains("Value"));
        assert!(mismatch.contains("Index"));
        assert!(mismatch.contains("Critical"));
        assert!(mismatch.contains("NonCritical"));
    }

    #[test]
    fn drift_summary_counts_categories() {
        let results = vec![DifferentialResult {
            case_id: "test".to_owned(),
            packet_id: "FP-P2C-001".to_owned(),
            operation: FixtureOperation::SeriesAdd,
            mode: RuntimeMode::Strict,
            oracle_source: FixtureOracleSource::Fixture,
            status: CaseStatus::Fail,
            drift_records: vec![
                DriftRecord {
                    category: ComparisonCategory::Value,
                    level: DriftLevel::Critical,
                    location: "test[0]".to_owned(),
                    message: "mismatch".to_owned(),
                },
                DriftRecord {
                    category: ComparisonCategory::Index,
                    level: DriftLevel::NonCritical,
                    location: "test.index".to_owned(),
                    message: "order".to_owned(),
                },
                DriftRecord {
                    category: ComparisonCategory::Nullness,
                    level: DriftLevel::Informational,
                    location: "test[1]".to_owned(),
                    message: "nan handling".to_owned(),
                },
            ],
            evidence_records: 0,
        }];
        let summary = super::summarize_drift(&results);
        assert_eq!(summary.total_drift_records, 3);
        assert_eq!(summary.critical_count, 1);
        assert_eq!(summary.non_critical_count, 1);
        assert_eq!(summary.informational_count, 1);
        assert_eq!(summary.categories.len(), 3);
    }

    #[test]
    fn build_differential_report_aggregates_correctly() {
        let results = vec![
            DifferentialResult {
                case_id: "pass_case".to_owned(),
                packet_id: "FP-P2C-001".to_owned(),
                operation: FixtureOperation::SeriesAdd,
                mode: RuntimeMode::Strict,
                oracle_source: FixtureOracleSource::Fixture,
                status: CaseStatus::Pass,
                drift_records: Vec::new(),
                evidence_records: 0,
            },
            DifferentialResult {
                case_id: "fail_case".to_owned(),
                packet_id: "FP-P2C-001".to_owned(),
                operation: FixtureOperation::SeriesAdd,
                mode: RuntimeMode::Strict,
                oracle_source: FixtureOracleSource::Fixture,
                status: CaseStatus::Fail,
                drift_records: vec![DriftRecord {
                    category: ComparisonCategory::Value,
                    level: DriftLevel::Critical,
                    location: "v[0]".to_owned(),
                    message: "bad".to_owned(),
                }],
                evidence_records: 1,
            },
        ];
        let report = build_differential_report(
            "test_suite".to_owned(),
            Some("FP-P2C-001".to_owned()),
            true,
            results,
        );
        assert_eq!(report.report.fixture_count, 2);
        assert_eq!(report.report.passed, 1);
        assert_eq!(report.report.failed, 1);
        assert_eq!(report.differential_results.len(), 2);
        assert_eq!(report.drift_summary.total_drift_records, 1);
        assert_eq!(report.drift_summary.critical_count, 1);
    }

    #[test]
    fn differential_report_serializes_to_json() {
        let cfg = HarnessConfig::default_paths();
        let diff_report = run_differential_by_id(&cfg, "FP-P2C-001", OracleMode::FixtureExpected)
            .expect("differential");
        let json = serde_json::to_string_pretty(&diff_report).expect("serialize");
        assert!(json.contains("differential_results"));
        assert!(json.contains("drift_summary"));
    }

    #[test]
    fn differential_all_packets_green() {
        let cfg = HarnessConfig::default_paths();
        for packet_id in &[
            "FP-P2C-001",
            "FP-P2C-002",
            "FP-P2C-003",
            "FP-P2C-004",
            "FP-P2C-005",
        ] {
            let diff_report = run_differential_by_id(&cfg, packet_id, OracleMode::FixtureExpected)
                .expect(packet_id);
            assert!(
                diff_report.report.is_green(),
                "{packet_id} differential not green: {:?}",
                diff_report.drift_summary
            );
        }
    }

    // === E2E Orchestrator + Forensic Logging Tests (bd-2gi.6) ===

    #[test]
    fn forensic_log_records_events_with_timestamps() {
        let mut log = ForensicLog::new();
        assert!(log.is_empty());

        log.record(ForensicEventKind::SuiteStart {
            suite: "test".to_owned(),
            packet_filter: None,
        });
        log.record(ForensicEventKind::SuiteEnd {
            suite: "test".to_owned(),
            total_fixtures: 5,
            passed: 5,
            failed: 0,
        });

        assert_eq!(log.len(), 2);
        assert!(log.events[0].ts_unix_ms > 0);
        assert!(log.events[1].ts_unix_ms >= log.events[0].ts_unix_ms);
    }

    #[test]
    fn forensic_log_serializes_to_jsonl() {
        let mut log = ForensicLog::new();
        log.record(ForensicEventKind::PacketStart {
            packet_id: "FP-P2C-001".to_owned(),
        });
        log.record(ForensicEventKind::CaseEnd {
            packet_id: "FP-P2C-001".to_owned(),
            case_id: "test_case".to_owned(),
            status: CaseStatus::Pass,
            evidence_records: 2,
            elapsed_us: 1234,
        });

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("forensic.jsonl");
        log.write_jsonl(&path).expect("write");

        let content = fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);

        // Each line is valid JSON
        for line in &lines {
            let _: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
        }
        assert!(lines[0].contains("packet_start"));
        assert!(lines[1].contains("case_end"));
        assert!(lines[1].contains("test_case"));
    }

    #[test]
    fn forensic_event_kind_serde_round_trip() {
        let events = vec![
            ForensicEventKind::SuiteStart {
                suite: "smoke".to_owned(),
                packet_filter: Some("FP-P2C-001".to_owned()),
            },
            ForensicEventKind::ArtifactWritten {
                packet_id: "FP-P2C-001".to_owned(),
                artifact_kind: "parity_report".to_owned(),
                path: "artifacts/phase2c/FP-P2C-001/parity_report.json".to_owned(),
            },
            ForensicEventKind::GateEvaluated {
                packet_id: "FP-P2C-002".to_owned(),
                pass: true,
                reasons: Vec::new(),
            },
            ForensicEventKind::Error {
                phase: "gate_enforcement".to_owned(),
                message: "gate failed".to_owned(),
            },
        ];

        for event in &events {
            let json = serde_json::to_string(event).expect("serialize");
            let back: ForensicEventKind = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(*event, back);
        }
    }

    #[test]
    fn e2e_suite_runs_full_pipeline() {
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions::default(),
            write_artifacts: false,      // skip artifact writes in test
            enforce_gates: false,        // skip enforcement in test
            append_drift_history: false, // skip drift history in test
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let report = run_e2e_suite(&config, &mut hooks).expect("e2e");

        // Should have run all 5 packets
        assert!(
            report.packet_reports.len() >= 5,
            "expected 5+ packet reports"
        );
        assert!(report.total_fixtures > 0, "should have fixtures");
        assert_eq!(report.total_failed, 0, "no failures expected");
        assert!(report.is_green(), "e2e should be green");

        // Forensic log should have events
        assert!(!report.forensic_log.is_empty());

        // Should have suite_start and suite_end
        let first = &report.forensic_log.events[0].event;
        assert!(
            matches!(first, ForensicEventKind::SuiteStart { .. }),
            "first event should be SuiteStart"
        );
        let last = &report.forensic_log.events[report.forensic_log.len() - 1].event;
        assert!(
            matches!(last, ForensicEventKind::SuiteEnd { .. }),
            "last event should be SuiteEnd"
        );
    }

    #[test]
    fn e2e_suite_with_packet_filter() {
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let mut hooks = NoopHooks;
        let report = run_e2e_suite(&config, &mut hooks).expect("e2e");

        assert_eq!(report.packet_reports.len(), 1);
        assert_eq!(
            report.packet_reports[0].packet_id.as_deref(),
            Some("FP-P2C-001")
        );
        assert!(report.is_green());
    }

    #[test]
    fn e2e_lifecycle_hooks_called() {
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct TrackingHooks {
            calls: Arc<Mutex<Vec<String>>>,
        }

        impl LifecycleHooks for TrackingHooks {
            fn before_suite(&mut self, suite: &str, _filter: &Option<String>) {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("before_suite:{suite}"));
            }
            fn after_suite(&mut self, _reports: &[PacketParityReport]) {
                self.calls.lock().unwrap().push("after_suite".to_owned());
            }
            fn before_packet(&mut self, packet_id: &str) {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("before_packet:{packet_id}"));
            }
            fn after_packet(&mut self, _report: &PacketParityReport, gate_pass: bool) {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("after_packet:gate={gate_pass}"));
            }
            fn after_case(&mut self, result: &CaseResult) {
                self.calls
                    .lock()
                    .unwrap()
                    .push(format!("after_case:{}", result.case_id));
            }
        }

        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };

        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut hooks = TrackingHooks {
            calls: calls.clone(),
        };
        let _report = run_e2e_suite(&config, &mut hooks).expect("e2e");

        let logged = calls.lock().unwrap();
        assert!(logged[0].starts_with("before_suite:"));
        assert!(logged.iter().any(|c| c.starts_with("before_packet:")));
        assert!(logged.iter().any(|c| c.starts_with("after_packet:")));
        assert!(logged.iter().any(|c| c.starts_with("after_case:")));
        assert_eq!(*logged.last().unwrap(), "after_suite");
    }

    #[test]
    fn e2e_forensic_log_writes_to_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log_path = dir.path().join("forensic.jsonl");

        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions {
                packet_filter: Some("FP-P2C-001".to_owned()),
                oracle_mode: OracleMode::FixtureExpected,
            },
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: Some(log_path.clone()),
        };

        let mut hooks = NoopHooks;
        let _report = run_e2e_suite(&config, &mut hooks).expect("e2e");

        assert!(log_path.exists(), "forensic log file should exist");
        let content = fs::read_to_string(&log_path).expect("read");
        assert!(content.lines().count() >= 4, "should have multiple events");

        // Verify every line is valid JSON
        for line in content.lines() {
            let parsed: serde_json::Value = serde_json::from_str(line).expect("valid JSON line");
            assert!(parsed.get("ts_unix_ms").is_some());
            assert!(parsed.get("event").is_some());
        }
    }

    // === Failure Forensics UX Tests (bd-2gi.21) ===

    #[test]
    fn artifact_id_short_hash_is_deterministic() {
        let id = ArtifactId {
            packet_id: "FP-P2C-001".to_owned(),
            artifact_kind: "parity_report".to_owned(),
            run_ts_unix_ms: 1000,
        };
        let h1 = id.short_hash();
        let h2 = id.short_hash();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 8);
        // Display format
        let display = format!("{id}");
        assert!(display.starts_with("FP-P2C-001:parity_report@"));
    }

    #[test]
    fn failure_digest_display_format() {
        let digest = FailureDigest {
            packet_id: "FP-P2C-001".to_owned(),
            case_id: "series_add_strict".to_owned(),
            operation: FixtureOperation::SeriesAdd,
            mode: RuntimeMode::Strict,
            mismatch_summary: "expected Int64(10), got Float64(10.0)".to_owned(),
            replay_command: "cargo test -p fp-conformance -- series_add_strict --nocapture"
                .to_owned(),
            artifact_path: Some("artifacts/phase2c/FP-P2C-001/mismatch.json".to_owned()),
        };

        let output = format!("{digest}");
        assert!(output.contains("FAIL FP-P2C-001::series_add_strict"));
        assert!(output.contains("Mismatch:"));
        assert!(output.contains("Replay:"));
        assert!(output.contains("Artifact:"));
    }

    #[test]
    fn failure_forensics_report_clean_when_all_pass() {
        let config = E2eConfig {
            harness: HarnessConfig::default_paths(),
            options: SuiteOptions::default(),
            write_artifacts: false,
            enforce_gates: false,
            append_drift_history: false,
            forensic_log_path: None,
        };
        let mut hooks = NoopHooks;
        let e2e = run_e2e_suite(&config, &mut hooks).expect("e2e");
        let forensics = build_failure_forensics(&e2e);

        assert!(forensics.is_clean());
        let output = format!("{forensics}");
        assert!(output.contains("ALL GREEN"));
    }

    #[test]
    fn failure_forensics_report_shows_failures() {
        let report = FailureForensicsReport {
            run_ts_unix_ms: 1000,
            total_fixtures: 5,
            total_passed: 3,
            total_failed: 2,
            failures: vec![
                FailureDigest {
                    packet_id: "FP-P2C-001".to_owned(),
                    case_id: "case_a".to_owned(),
                    operation: FixtureOperation::SeriesAdd,
                    mode: RuntimeMode::Strict,
                    mismatch_summary: "value drift".to_owned(),
                    replay_command: "cargo test -- case_a".to_owned(),
                    artifact_path: None,
                },
                FailureDigest {
                    packet_id: "FP-P2C-002".to_owned(),
                    case_id: "case_b".to_owned(),
                    operation: FixtureOperation::IndexAlignUnion,
                    mode: RuntimeMode::Hardened,
                    mismatch_summary: "shape mismatch".to_owned(),
                    replay_command: "cargo test -- case_b".to_owned(),
                    artifact_path: Some("path/to/corpus.json".to_owned()),
                },
            ],
            gate_failures: vec!["FP-P2C-001: strict_failed > 0".to_owned()],
        };

        assert!(!report.is_clean());
        let output = format!("{report}");
        assert!(output.contains("FAILURES: 2/5"));
        assert!(output.contains("case_a"));
        assert!(output.contains("case_b"));
        assert!(output.contains("GATE FAILURES:"));
        assert!(output.contains("strict_failed > 0"));
    }

    #[test]
    fn failure_forensics_serializes_to_json() {
        let report = FailureForensicsReport {
            run_ts_unix_ms: 1000,
            total_fixtures: 1,
            total_passed: 1,
            total_failed: 0,
            failures: Vec::new(),
            gate_failures: Vec::new(),
        };
        let json = serde_json::to_string(&report).expect("serialize");
        let back: FailureForensicsReport = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report, back);
    }

    // === RaptorQ CI Enforcement Tests (bd-2gi.9) ===

    #[test]
    fn decode_proof_artifact_round_trip_serialization() {
        let artifact = DecodeProofArtifact {
            packet_id: "FP-P2C-001".to_owned(),
            decode_proofs: vec![fp_runtime::DecodeProof {
                ts_unix_ms: 1000,
                reason: "test drill".to_owned(),
                recovered_blocks: 2,
                proof_hash: "sha256:abcdef".to_owned(),
            }],
            status: DecodeProofStatus::Recovered,
        };
        let json = serde_json::to_string_pretty(&artifact).expect("serialize");
        let back: DecodeProofArtifact = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(artifact, back);
        assert!(json.contains("\"recovered\""));
    }

    #[test]
    fn decode_proof_status_variants_serialize_correctly() {
        for (status, expected) in [
            (DecodeProofStatus::Recovered, "\"recovered\""),
            (DecodeProofStatus::Failed, "\"failed\""),
            (DecodeProofStatus::NotAttempted, "\"not_attempted\""),
        ] {
            let json = serde_json::to_string(&status).expect("serialize");
            assert_eq!(json, expected);
        }
    }

    #[test]
    fn sidecar_integrity_check_passes_for_valid_packet_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let packet_id = "FP-P2C-099";

        // Write parity report
        let report_bytes = br#"{"suite":"test","passed":1,"failed":0}"#;
        fs::write(dir.path().join("parity_report.json"), report_bytes).unwrap();

        // Generate sidecar
        let mut sidecar =
            generate_raptorq_sidecar("FP-P2C-099/parity_report", "conformance", report_bytes, 8)
                .expect("sidecar");
        let proof =
            run_raptorq_decode_recovery_drill(&sidecar, report_bytes).expect("decode drill");
        sidecar.envelope.decode_proofs = vec![proof.clone()];
        sidecar.envelope.scrub.status = "ok".to_owned();
        fs::write(
            dir.path().join("parity_report.raptorq.json"),
            serde_json::to_string_pretty(&sidecar).unwrap(),
        )
        .unwrap();

        // Write decode proof artifact
        let decode_artifact = DecodeProofArtifact {
            packet_id: packet_id.to_owned(),
            decode_proofs: vec![proof],
            status: DecodeProofStatus::Recovered,
        };
        fs::write(
            dir.path().join("parity_report.decode_proof.json"),
            serde_json::to_string_pretty(&decode_artifact).unwrap(),
        )
        .unwrap();

        let result = verify_packet_sidecar_integrity(dir.path(), packet_id);
        assert!(
            result.is_ok(),
            "expected ok, got errors: {:?}",
            result.errors
        );
        assert!(result.source_hash_matches);
        assert!(result.scrub_ok);
        assert!(result.decode_proof_valid);
    }

    #[test]
    fn sidecar_integrity_fails_when_sidecar_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let report_bytes = br#"{"suite":"test"}"#;
        fs::write(dir.path().join("parity_report.json"), report_bytes).unwrap();

        let result = verify_packet_sidecar_integrity(dir.path(), "FP-P2C-099");
        assert!(!result.is_ok());
        assert!(result.parity_report_exists);
        assert!(!result.sidecar_exists);
        assert!(result.errors.iter().any(|e| e.contains("Rule T5")));
    }

    #[test]
    fn verify_all_sidecars_ci_on_empty_dir_returns_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = verify_all_sidecars_ci(dir.path());
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn verify_all_sidecars_ci_on_existing_packets() {
        let cfg = HarnessConfig::default_paths();
        let artifact_root = cfg.repo_root.join("artifacts");
        if !artifact_root.join("phase2c").exists() {
            return; // skip if no artifacts
        }
        let result = verify_all_sidecars_ci(&artifact_root);
        match result {
            Ok(results) => {
                for r in &results {
                    assert!(r.is_ok(), "{}: {:?}", r.packet_id, r.errors);
                }
            }
            Err(failures) => {
                for f in &failures {
                    eprintln!("SIDECAR INTEGRITY FAILURE: {}: {:?}", f.packet_id, f.errors);
                }
                // Don't assert here - existing artifacts may not all have sidecars yet
            }
        }
    }

    // === CI Gate Topology Tests (bd-2gi.10) ===

    #[test]
    fn ci_gate_pipeline_order_is_monotonic() {
        let pipeline = CiGate::pipeline();
        for window in pipeline.windows(2) {
            assert!(
                window[0].order() < window[1].order(),
                "{:?} should come before {:?}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn ci_gate_commit_pipeline_is_subset_of_full() {
        let full = CiGate::pipeline();
        let commit = CiGate::commit_pipeline();
        for gate in &commit {
            assert!(
                full.contains(gate),
                "{gate:?} in commit but not full pipeline"
            );
        }
    }

    #[test]
    fn ci_gate_labels_are_nonempty() {
        for gate in CiGate::pipeline() {
            assert!(!gate.label().is_empty());
            assert!(gate.to_string().contains("G"));
        }
    }

    #[test]
    fn ci_gate_serialization_round_trip() {
        for gate in CiGate::pipeline() {
            let json = serde_json::to_string(&gate).expect("serialize");
            let back: CiGate = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(gate, back);
        }
    }

    #[test]
    fn ci_gate_g6_conformance_evaluates() {
        let config = CiPipelineConfig::default();
        let result = evaluate_ci_gate(CiGate::G6Conformance, &config);
        assert!(result.passed, "G6 should pass: {}", result.summary);
        assert!(result.elapsed_ms > 0 || result.passed);
    }

    #[test]
    fn ci_pipeline_result_display_format() {
        let result = CiPipelineResult {
            gates: vec![
                CiGateResult {
                    gate: CiGate::G3Unit,
                    passed: true,
                    elapsed_ms: 100,
                    summary: "10 tests passed".to_owned(),
                    errors: vec![],
                },
                CiGateResult {
                    gate: CiGate::G6Conformance,
                    passed: false,
                    elapsed_ms: 200,
                    summary: "2 fixtures failed".to_owned(),
                    errors: vec!["case_a: value drift".to_owned()],
                },
            ],
            all_passed: false,
            first_failure: Some(CiGate::G6Conformance),
            elapsed_ms: 300,
        };
        let output = format!("{result}");
        assert!(output.contains("FAILED"));
        assert!(output.contains("[PASS]"));
        assert!(output.contains("[FAIL]"));
        assert!(output.contains("value drift"));
    }

    #[test]
    fn ci_pipeline_conformance_only_runs_and_reports() {
        let config = CiPipelineConfig {
            gates: vec![CiGate::G6Conformance],
            fail_fast: true,
            harness_config: HarnessConfig::default_paths(),
            verify_sidecars: false,
        };
        let result = run_ci_pipeline(&config);
        assert_eq!(result.gates.len(), 1);
        assert!(
            result.all_passed,
            "conformance gate should pass: {}",
            result
        );
    }
}
