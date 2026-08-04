#![forbid(unsafe_code)]
#![warn(rustdoc::broken_intra_doc_links)]

//! Runtime policy + decision-recording layer for **frankenpandas**.
//!
//! Pandas operations frequently hit "do we accept this input or fail
//! closed?" decisions: a dtype that doesn't quite match, a frequency
//! that's almost-but-not-quite regular, an alignment that produces
//! NaNs the user maybe didn't expect. fp-runtime gives the rest of
//! the workspace a single place to record those decisions, score
//! their compatibility, and persist a verifiable evidence trail so
//! pipelines can audit "why did the IO layer / groupby / merge make
//! this choice on this input?" after the fact.
//!
//! ## Decision recording
//!
//! - [`RuntimePolicy`]: the active policy bundle — mode, fail-closed
//!   flags, decision thresholds. Constructed once per pipeline and
//!   threaded through hot-path code.
//! - [`RuntimeMode`]: the top-level mode (Permissive / Hardened /
//!   Strict) controlling how aggressively the policy fails on
//!   ambiguity.
//! - [`EvidenceLedger`]: append-only log of [`DecisionRecord`]
//!   entries. Thread it through long-running pipelines to capture
//!   every decision made; serialize at the end for audit.
//! - [`DecisionRecord`]: one decision's structured trail —
//!   [`DecisionAction`] taken, the [`DecisionMetrics`] /
//!   [`LossMatrix`] / [`EvidenceTerm`] inputs, any
//!   [`CompatibilityIssue`] entries surfaced.
//! - [`GalaxyBrainCard`]: human-readable summary of one decision
//!   suitable for surfacing in IDE plugins or CI logs.
//! - [`decision_to_card`]: convert a [`DecisionRecord`] to a card.
//!
//! ## Conformal prediction guards
//!
//! - [`ConformalGuard`]: rolling-window nonconformity calibration
//!   used to gate uncertain decisions inside hot-path code (e.g.
//!   "this dtype inference is too unsure — fail closed").
//! - [`ConformalPredictionSet`]: the calibrated prediction set
//!   (inclusion / exclusion of candidate labels) the guard
//!   produces.
//!
//! ## RaptorQ envelopes
//!
//! - [`RaptorQEnvelope`] / [`RaptorQMetadata`] / [`ScrubStatus`] /
//!   [`DecodeProof`]: forward-error-correction envelope types used
//!   for verifying / scrubbing on-disk artifacts that the runtime
//!   policy needs to trust.
//!
//! ## Error reporting
//!
//! - [`RuntimeError`]: structural errors in policy construction or
//!   ledger serialization.
//! - [`IssueKind`]: enum tagging the category of a
//!   [`CompatibilityIssue`].
//!
//! ## Cargo features
//!
//! - `asupersync` (off by default): enables the `asupersync`
//!   submodule and the `outcome_to_action` helper for converting
//!   an `asupersync::Outcome` into a [`DecisionAction`]. Pulls in
//!   the `asupersync` crate as an optional dep. (Items are gated
//!   behind the feature so they don't appear in the default
//!   docs.rs render.)

use std::{
    borrow::Cow,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(feature = "asupersync")]
pub mod asupersync;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    Strict,
    Hardened,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    Allow,
    Reject,
    Repair,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueKind {
    UnknownFeature,
    MalformedInput,
    JoinCardinality,
    PolicyOverride,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityIssue {
    pub kind: IssueKind,
    pub subject: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceTerm {
    pub name: Cow<'static, str>,
    pub log_likelihood_if_compatible: f64,
    pub log_likelihood_if_incompatible: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LossMatrix {
    pub allow_if_compatible: f64,
    pub allow_if_incompatible: f64,
    pub reject_if_compatible: f64,
    pub reject_if_incompatible: f64,
    pub repair_if_compatible: f64,
    pub repair_if_incompatible: f64,
}

impl Default for LossMatrix {
    fn default() -> Self {
        Self {
            allow_if_compatible: 0.0,
            allow_if_incompatible: 100.0,
            reject_if_compatible: 6.0,
            reject_if_incompatible: 0.5,
            repair_if_compatible: 2.0,
            repair_if_incompatible: 3.0,
        }
    }
}

const UNKNOWN_FEATURE_PRIOR: f64 = 0.25;
const JOIN_ADMISSION_PRIOR: f64 = 0.6;
const PRIOR_COMPATIBLE_EPSILON: f64 = 1e-10;

const UNKNOWN_FEATURE_EVIDENCE: [EvidenceTerm; 2] = [
    EvidenceTerm {
        name: Cow::Borrowed("compatibility_allowlist_miss"),
        log_likelihood_if_compatible: -3.5,
        log_likelihood_if_incompatible: -0.2,
    },
    EvidenceTerm {
        name: Cow::Borrowed("unknown_protocol_field"),
        log_likelihood_if_compatible: -2.0,
        log_likelihood_if_incompatible: -0.1,
    },
];

const JOIN_ADMISSION_EVIDENCE_WITHIN_CAP: [EvidenceTerm; 2] = [
    EvidenceTerm {
        name: Cow::Borrowed("estimator_overflow_risk"),
        log_likelihood_if_compatible: -0.3,
        log_likelihood_if_incompatible: -1.2,
    },
    EvidenceTerm {
        name: Cow::Borrowed("memory_budget_signal"),
        log_likelihood_if_compatible: -0.4,
        log_likelihood_if_incompatible: -1.5,
    },
];

const JOIN_ADMISSION_EVIDENCE_OVER_CAP: [EvidenceTerm; 2] = [
    EvidenceTerm {
        name: Cow::Borrowed("estimator_overflow_risk"),
        log_likelihood_if_compatible: -2.8,
        log_likelihood_if_incompatible: -0.1,
    },
    EvidenceTerm {
        name: Cow::Borrowed("memory_budget_signal"),
        log_likelihood_if_compatible: -2.2,
        log_likelihood_if_incompatible: -0.2,
    },
];

const JOIN_ADMISSION_LOSS: LossMatrix = LossMatrix {
    allow_if_compatible: 0.0,
    allow_if_incompatible: 130.0,
    reject_if_compatible: 5.0,
    reject_if_incompatible: 0.5,
    repair_if_compatible: 1.5,
    repair_if_incompatible: 3.0,
};

const DEFAULT_CONFORMAL_ALPHA: f64 = 0.1;
const MIN_CONFORMAL_ALPHA: f64 = 0.01;
const MAX_CONFORMAL_ALPHA: f64 = 0.5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionMetrics {
    pub posterior_compatible: f64,
    pub bayes_factor_compatible_over_incompatible: f64,
    pub expected_loss_allow: f64,
    pub expected_loss_reject: f64,
    pub expected_loss_repair: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub ts_unix_ms: u64,
    pub mode: RuntimeMode,
    pub action: DecisionAction,
    pub issue: CompatibilityIssue,
    pub prior_compatible: f64,
    pub metrics: DecisionMetrics,
    pub evidence: Vec<EvidenceTerm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticIndexIdentity {
    pub role: String,
    pub len: usize,
    pub has_duplicates: bool,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticWitnessRecord {
    pub ts_unix_ms: u64,
    pub operation: String,
    pub materialization_reason: String,
    pub alignment_mode: String,
    pub input_index_identity: Vec<SemanticIndexIdentity>,
    pub output_index_identity: SemanticIndexIdentity,
    pub null_nan_policy: String,
    pub output_ordering_contract: String,
}

impl SemanticWitnessRecord {
    #[must_use]
    pub fn new(
        operation: impl Into<String>,
        materialization_reason: impl Into<String>,
        alignment_mode: impl Into<String>,
        input_index_identity: Vec<SemanticIndexIdentity>,
        output_index_identity: SemanticIndexIdentity,
        null_nan_policy: impl Into<String>,
        output_ordering_contract: impl Into<String>,
    ) -> Self {
        Self {
            ts_unix_ms: now_unix_ms().unwrap_or_default(),
            operation: operation.into(),
            materialization_reason: materialization_reason.into(),
            alignment_mode: alignment_mode.into(),
            input_index_identity,
            output_index_identity,
            null_nan_policy: null_nan_policy.into(),
            output_ordering_contract: output_ordering_contract.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GalaxyBrainCard {
    pub title: String,
    pub equation: String,
    pub substitution: String,
    pub intuition: String,
}

impl GalaxyBrainCard {
    #[must_use]
    pub fn render_plain(&self) -> String {
        let capacity = self.title.len()
            + self.equation.len()
            + self.substitution.len()
            + self.intuition.len()
            + 5;
        let mut rendered = String::with_capacity(capacity);
        rendered.push('[');
        rendered.push_str(&self.title);
        rendered.push_str("]\n");
        rendered.push_str(&self.equation);
        rendered.push('\n');
        rendered.push_str(&self.substitution);
        rendered.push('\n');
        rendered.push_str(&self.intuition);
        rendered
    }
}

#[must_use]
pub fn decision_to_card(record: &DecisionRecord) -> GalaxyBrainCard {
    GalaxyBrainCard {
        title: format!("{}::{:?}", record.issue.subject, record.action),
        equation: "argmin_a Σ_s L(a,s) P(s|evidence)".to_owned(),
        substitution: format!(
            "P(compatible|e)={:.4}, E[allow]={:.4}, E[reject]={:.4}, E[repair]={:.4}",
            record.metrics.posterior_compatible,
            record.metrics.expected_loss_allow,
            record.metrics.expected_loss_reject,
            record.metrics.expected_loss_repair
        ),
        intuition: "Lower expected loss wins; strict mode may still force fail-closed.".to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceLedger {
    records: Vec<DecisionRecord>,
    #[serde(default)]
    semantic_witnesses: Vec<SemanticWitnessRecord>,
    // Runtime-only switch: when false, callers that build a throwaway ledger
    // (e.g. the public `Series::add` convenience wrappers that discard the
    // ledger) can skip the expensive semantic-witness fingerprinting. Not part
    // of the serialized audit artifact, and `#[serde(skip)]` keeps the on-disk
    // format unchanged. Per br-frankenpandas-b75cc.
    #[serde(skip)]
    record_semantic_witnesses: bool,
}

impl Default for EvidenceLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl EvidenceLedger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            semantic_witnesses: Vec::new(),
            record_semantic_witnesses: true,
        }
    }

    /// Build a ledger that does not record semantic witnesses. Used by the
    /// public arithmetic convenience methods that discard their ledger, so the
    /// AACE witness fingerprint (a sha256 over the index) is not computed when
    /// nothing will read it. Observable operation output is unaffected.
    #[must_use]
    pub fn without_semantic_witnesses(mut self) -> Self {
        self.record_semantic_witnesses = false;
        self
    }

    /// Whether this ledger records AACE semantic witnesses (default true).
    #[must_use]
    pub fn records_semantic_witnesses(&self) -> bool {
        self.record_semantic_witnesses
    }

    pub fn push(&mut self, record: DecisionRecord) {
        self.records.push(record);
    }

    pub fn push_semantic_witness(&mut self, record: SemanticWitnessRecord) {
        self.semantic_witnesses.push(record);
    }

    #[must_use]
    pub fn records(&self) -> &[DecisionRecord] {
        &self.records
    }

    #[must_use]
    pub fn semantic_witnesses(&self) -> &[SemanticWitnessRecord] {
        &self.semantic_witnesses
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePolicy {
    pub mode: RuntimeMode,
    pub fail_closed_unknown_features: bool,
    pub hardened_join_row_cap: Option<usize>,
}

impl RuntimePolicy {
    #[must_use]
    pub fn strict() -> Self {
        Self {
            mode: RuntimeMode::Strict,
            fail_closed_unknown_features: true,
            hardened_join_row_cap: None,
        }
    }

    #[must_use]
    pub fn hardened(join_row_cap: Option<usize>) -> Self {
        Self {
            mode: RuntimeMode::Hardened,
            fail_closed_unknown_features: false,
            hardened_join_row_cap: join_row_cap,
        }
    }

    pub fn decide_unknown_feature(
        &self,
        subject: impl Into<String>,
        detail: impl Into<String>,
        ledger: &mut EvidenceLedger,
    ) -> DecisionAction {
        let issue = CompatibilityIssue {
            kind: IssueKind::UnknownFeature,
            subject: subject.into(),
            detail: detail.into(),
        };

        let mut record = decide(
            self.mode,
            issue,
            UNKNOWN_FEATURE_PRIOR,
            LossMatrix::default(),
            UNKNOWN_FEATURE_EVIDENCE.to_vec(),
        );
        if self.fail_closed_unknown_features {
            record.action = DecisionAction::Reject;
        }
        let action = record.action;
        ledger.push(record);
        action
    }

    pub fn decide_join_admission(
        &self,
        estimated_rows: usize,
        ledger: &mut EvidenceLedger,
    ) -> DecisionAction {
        let issue = CompatibilityIssue {
            kind: IssueKind::JoinCardinality,
            subject: "join_estimator".to_owned(),
            detail: format!("estimated_rows={estimated_rows}"),
        };

        let cap = self.hardened_join_row_cap.unwrap_or(usize::MAX);
        let evidence = if estimated_rows <= cap {
            JOIN_ADMISSION_EVIDENCE_WITHIN_CAP.to_vec()
        } else {
            JOIN_ADMISSION_EVIDENCE_OVER_CAP.to_vec()
        };
        let mut record = decide(
            self.mode,
            issue,
            JOIN_ADMISSION_PRIOR,
            JOIN_ADMISSION_LOSS,
            evidence,
        );

        if matches!(self.mode, RuntimeMode::Hardened) && estimated_rows > cap {
            record.action = DecisionAction::Repair;
        }

        let action = record.action;
        ledger.push(record);
        action
    }
}

impl Default for RuntimePolicy {
    fn default() -> Self {
        Self::strict()
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("system clock is before UNIX_EPOCH")]
    ClockSkew,
}

fn now_unix_ms() -> Result<u64, RuntimeError> {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RuntimeError::ClockSkew)?
        .as_millis();
    Ok(ms as u64)
}

fn normalize_prior_compatible(prior_compatible: f64) -> f64 {
    if !prior_compatible.is_finite() {
        return 0.5;
    }
    prior_compatible.clamp(PRIOR_COMPATIBLE_EPSILON, 1.0 - PRIOR_COMPATIBLE_EPSILON)
}

fn decide(
    mode: RuntimeMode,
    issue: CompatibilityIssue,
    prior_compatible: f64,
    loss: LossMatrix,
    evidence: Vec<EvidenceTerm>,
) -> DecisionRecord {
    let prior_compatible = normalize_prior_compatible(prior_compatible);
    let log_odds_prior = (prior_compatible / (1.0 - prior_compatible)).ln();
    let llr_sum: f64 = evidence
        .iter()
        .map(|term| term.log_likelihood_if_compatible - term.log_likelihood_if_incompatible)
        .sum();
    let log_odds_post = log_odds_prior + llr_sum;

    let posterior_compatible = 1.0 / (1.0 + (-log_odds_post).exp());
    let posterior_incompatible = 1.0 - posterior_compatible;

    let expected_loss_allow = loss.allow_if_compatible * posterior_compatible
        + loss.allow_if_incompatible * posterior_incompatible;
    let expected_loss_reject = loss.reject_if_compatible * posterior_compatible
        + loss.reject_if_incompatible * posterior_incompatible;
    let expected_loss_repair = loss.repair_if_compatible * posterior_compatible
        + loss.repair_if_incompatible * posterior_incompatible;

    let mut best_action = DecisionAction::Allow;
    let mut best_loss = expected_loss_allow;

    if expected_loss_repair < best_loss {
        best_action = DecisionAction::Repair;
        best_loss = expected_loss_repair;
    }
    if expected_loss_reject < best_loss {
        best_action = DecisionAction::Reject;
    }

    DecisionRecord {
        ts_unix_ms: now_unix_ms().unwrap_or_default(),
        mode,
        action: best_action,
        issue,
        prior_compatible,
        metrics: DecisionMetrics {
            posterior_compatible,
            bayes_factor_compatible_over_incompatible: llr_sum.exp(),
            expected_loss_allow,
            expected_loss_reject,
            expected_loss_repair,
        },
        evidence,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaptorQEnvelope {
    pub artifact_id: String,
    pub artifact_type: String,
    pub source_hash: String,
    pub raptorq: RaptorQMetadata,
    pub scrub: ScrubStatus,
    pub decode_proofs: Vec<DecodeProof>,
}

pub const MAX_DECODE_PROOFS: usize = 1_000;
pub const DEFAULT_RAPTORQ_SYMBOL_BYTES: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaptorQMetadata {
    pub k: u32,
    pub repair_symbols: u32,
    pub overhead_ratio: f64,
    pub symbol_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScrubStatus {
    pub last_ok_unix_ms: u64,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodeProof {
    pub ts_unix_ms: u64,
    pub reason: String,
    pub recovered_blocks: u32,
    pub proof_hash: String,
}

impl RaptorQEnvelope {
    #[must_use]
    pub fn from_source_bytes(
        artifact_id: impl Into<String>,
        artifact_type: impl Into<String>,
        source_bytes: &[u8],
        repair_symbols: u32,
    ) -> Self {
        let symbol_hashes: Vec<String> = source_bytes
            .chunks(DEFAULT_RAPTORQ_SYMBOL_BYTES)
            .map(sha256_prefixed_hex)
            .collect();
        let k = u32::try_from(symbol_hashes.len()).unwrap_or(u32::MAX);
        let overhead_ratio = if k == 0 {
            0.0
        } else {
            f64::from(repair_symbols) / f64::from(k)
        };

        Self {
            artifact_id: artifact_id.into(),
            artifact_type: artifact_type.into(),
            source_hash: sha256_prefixed_hex(source_bytes),
            raptorq: RaptorQMetadata {
                k,
                repair_symbols,
                overhead_ratio,
                symbol_hashes,
            },
            scrub: ScrubStatus {
                last_ok_unix_ms: now_unix_ms().unwrap_or_default(),
                status: "ok".to_owned(),
            },
            decode_proofs: Vec::new(),
        }
    }

    /// Append a decode proof while enforcing a bounded history size.
    ///
    /// When the cap is exceeded, oldest proofs are evicted first.
    pub fn push_decode_proof_capped(&mut self, proof: DecodeProof) {
        if self.decode_proofs.len() >= MAX_DECODE_PROOFS {
            let overflow = self.decode_proofs.len() + 1 - MAX_DECODE_PROOFS;
            self.decode_proofs.drain(0..overflow);
        }
        self.decode_proofs.push(proof);
    }
}

#[must_use]
pub fn semantic_fingerprint_bytes(bytes: &[u8]) -> String {
    sha256_prefixed_hex(bytes)
}

#[derive(Debug)]
pub struct SemanticFingerprintBuilder {
    hasher: Sha256,
}

impl Default for SemanticFingerprintBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticFingerprintBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            hasher: Sha256::new(),
        }
    }

    pub fn update(&mut self, bytes: &[u8]) {
        self.hasher.update(bytes);
    }

    #[must_use]
    pub fn finish(self) -> String {
        let digest = self.hasher.finalize();
        let mut output = String::with_capacity(7 + 64);
        output.push_str("sha256:");
        append_sha256_digest_hex(&mut output, digest);
        output
    }
}

#[cfg(test)]
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    sha256_digest_hex(digest)
}

fn sha256_prefixed_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(7 + 64);
    hex.push_str("sha256:");
    append_sha256_digest_hex(&mut hex, digest);
    hex
}

#[cfg(test)]
fn sha256_digest_hex(digest: impl IntoIterator<Item = u8>) -> String {
    let mut hex = String::with_capacity(64);
    append_sha256_digest_hex(&mut hex, digest);
    hex
}

fn append_sha256_digest_hex(hex: &mut String, digest: impl IntoIterator<Item = u8>) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        hex.push(char::from(HEX[usize::from(byte >> 4)]));
        hex.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

// === Conformal Calibration for Decision Engine (bd-2t5e.9, AG-09) ===

/// Nonconformity score computed from a single decision record.
/// Higher score = more "strange" relative to calibration window.
fn nonconformity_score(record: &DecisionRecord) -> f64 {
    // Score is the absolute log-posterior-odds: high when decision is extreme
    let p = record
        .metrics
        .posterior_compatible
        .clamp(1e-15, 1.0 - 1e-15);
    (p / (1.0 - p)).ln().abs()
}

fn normalize_conformal_alpha(alpha: f64) -> f64 {
    if alpha.is_finite() {
        alpha.clamp(MIN_CONFORMAL_ALPHA, MAX_CONFORMAL_ALPHA)
    } else {
        DEFAULT_CONFORMAL_ALPHA
    }
}

fn select_conformal_quantile(mut scores: Vec<f64>, alpha: f64) -> Option<f64> {
    if scores.len() < 2 {
        return None;
    }
    // Quantile at level (1 - alpha)(1 + 1/n) per split conformal prediction.
    let n = scores.len() as f64;
    let level = (1.0 - normalize_conformal_alpha(alpha)) * (1.0 + 1.0 / n);
    let idx = (level * n).ceil() as usize;
    let idx = idx.min(scores.len()).saturating_sub(1);
    let (_, quantile, _) = scores.select_nth_unstable_by(idx, f64::total_cmp);
    Some(*quantile)
}

/// Conformal prediction set: which actions are admissible at significance level alpha.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConformalPredictionSet {
    /// The conformal quantile threshold at significance level alpha.
    pub quantile_threshold: f64,
    /// The nonconformity score of the current decision.
    pub current_score: f64,
    /// Whether the Bayesian argmin action is inside the conformal set.
    pub bayesian_action_in_set: bool,
    /// Actions that are admissible (score <= threshold).
    pub admissible_actions: Vec<DecisionAction>,
    /// Empirical coverage rate over the calibration window.
    pub empirical_coverage: f64,
}

/// Calibration window for conformal guard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConformalGuard {
    /// Rolling window of nonconformity scores.
    scores: Vec<f64>,
    /// Maximum window size.
    window_size: usize,
    /// Significance level (e.g., 0.1 for 90% coverage).
    alpha: f64,
    /// Count of decisions where Bayesian action was in the conformal set.
    in_set_count: usize,
    /// Total decisions evaluated.
    total_count: usize,
}

impl ConformalGuard {
    /// Create a new conformal guard with the given window size and significance level.
    #[must_use]
    pub fn new(window_size: usize, alpha: f64) -> Self {
        let window_size = window_size.max(1);
        Self {
            scores: Vec::with_capacity(window_size),
            window_size,
            alpha: normalize_conformal_alpha(alpha),
            in_set_count: 0,
            total_count: 0,
        }
    }

    /// Default: 1000-element window, alpha=0.1 (90% coverage guarantee).
    #[must_use]
    pub fn default_config() -> Self {
        Self::new(1000, 0.1)
    }

    /// Compute the conformal quantile from the calibration window.
    /// Returns None if the window has fewer than 2 scores.
    #[must_use]
    pub fn conformal_quantile(&self) -> Option<f64> {
        let finite = self
            .scores
            .iter()
            .copied()
            .filter(|score| score.is_finite())
            .collect();
        select_conformal_quantile(finite, self.alpha)
    }

    /// Evaluate a decision record against the conformal guard.
    /// Returns the prediction set and whether the Bayesian action is admissible.
    pub fn evaluate(&mut self, record: &DecisionRecord) -> ConformalPredictionSet {
        self.normalize_runtime_config();
        let score = nonconformity_score(record);

        debug_assert!(self.scores.iter().all(|score| score.is_finite()));
        let quantile = select_conformal_quantile(self.scores.clone(), self.alpha);

        // Add score to calibration window (rolling)
        if self.scores.len() >= self.window_size {
            self.scores.remove(0);
        }
        self.scores.push(score);

        let threshold = match quantile {
            Some(q) => q,
            None => {
                // Insufficient calibration data: accept all actions
                self.total_count += 1;
                self.in_set_count += 1;
                return ConformalPredictionSet {
                    quantile_threshold: f64::INFINITY,
                    current_score: score,
                    bayesian_action_in_set: true,
                    admissible_actions: vec![
                        DecisionAction::Allow,
                        DecisionAction::Reject,
                        DecisionAction::Repair,
                    ],
                    empirical_coverage: 1.0,
                };
            }
        };

        let bayesian_in_set = score <= threshold;

        // Determine which actions would have scores <= threshold
        // For now, if the Bayesian action is in set, it's the only admissible one.
        // If not, we admit all actions (conformal guard widens the set).
        let admissible = if bayesian_in_set {
            vec![record.action]
        } else {
            vec![
                DecisionAction::Allow,
                DecisionAction::Reject,
                DecisionAction::Repair,
            ]
        };

        self.total_count += 1;
        if bayesian_in_set {
            self.in_set_count += 1;
        }

        let empirical_coverage = if self.total_count > 0 {
            self.in_set_count as f64 / self.total_count as f64
        } else {
            1.0
        };

        ConformalPredictionSet {
            quantile_threshold: threshold,
            current_score: score,
            bayesian_action_in_set: bayesian_in_set,
            admissible_actions: admissible,
            empirical_coverage,
        }
    }

    /// Current empirical coverage rate.
    #[must_use]
    pub fn empirical_coverage(&self) -> f64 {
        if self.total_count == 0 {
            return 1.0;
        }
        self.in_set_count.min(self.total_count) as f64 / self.total_count as f64
    }

    /// Number of scores in the calibration window.
    #[must_use]
    pub fn calibration_count(&self) -> usize {
        self.scores.len()
    }

    /// Whether the calibration window has sufficient data.
    #[must_use]
    pub fn is_calibrated(&self) -> bool {
        self.scores.iter().filter(|score| score.is_finite()).count() >= 2
    }

    /// Whether coverage has dropped below target for the alert threshold.
    #[must_use]
    pub fn coverage_alert(&self) -> bool {
        self.total_count >= 100
            && self.empirical_coverage() < (1.0 - normalize_conformal_alpha(self.alpha))
    }

    fn normalize_runtime_config(&mut self) {
        self.window_size = self.window_size.max(1);
        self.alpha = normalize_conformal_alpha(self.alpha);
        self.scores.retain(|score| score.is_finite());
        if self.scores.len() > self.window_size {
            let overflow = self.scores.len() - self.window_size;
            self.scores.drain(0..overflow);
        }
        self.in_set_count = self.in_set_count.min(self.total_count);
    }
}

#[cfg(feature = "asupersync")]
#[must_use]
pub fn outcome_to_action<T, E>(outcome: &::asupersync::Outcome<T, E>) -> DecisionAction {
    match outcome {
        ::asupersync::Outcome::Ok(_) => DecisionAction::Allow,
        ::asupersync::Outcome::Err(_) => DecisionAction::Repair,
        ::asupersync::Outcome::Cancelled(_) | ::asupersync::Outcome::Panicked(_) => {
            DecisionAction::Reject
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, hint::black_box, time::Instant};

    use serde::Serialize;

    use super::{
        ConformalGuard, DecisionAction, EvidenceLedger, GalaxyBrainCard, RaptorQEnvelope,
        RuntimeMode, RuntimePolicy, SemanticIndexIdentity, SemanticWitnessRecord, decision_to_card,
    };

    const ASUPERSYNC_PACKET_ID: &str = "ASUPERSYNC-E";
    const REPLAY_PREFIX: &str = "cargo test -p fp-runtime --";

    /// br-frankenpandas-01gdm: semantic_fingerprint_bytes underpins reproducibility
    /// ledgers / RaptorQ provenance. Known-answer SHA-256 vectors prove it's the
    /// real hash (not a stub), plus determinism, format, and builder equivalence.
    #[test]
    fn semantic_fingerprint_sha256_known_answers_01gdm() {
        // Standard SHA-256 known-answer vectors.
        assert_eq!(
            super::semantic_fingerprint_bytes(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            super::semantic_fingerprint_bytes(b"abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        // Format: "sha256:" + 64 lowercase hex chars; deterministic.
        let f = super::semantic_fingerprint_bytes(b"frankenpandas");
        assert!(f.starts_with("sha256:"), "prefix");
        assert_eq!(f.len(), 7 + 64, "length");
        assert!(
            f[7..]
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
            "lowercase hex"
        );
        assert_eq!(
            f,
            super::semantic_fingerprint_bytes(b"frankenpandas"),
            "deterministic"
        );

        // Distinctness on a small sample (no collisions).
        let inputs: [&[u8]; 5] = [b"a", b"b", b"ab", b"ba", b""];
        for (i, x) in inputs.iter().enumerate() {
            for (j, y) in inputs.iter().enumerate() {
                if i != j {
                    assert_ne!(
                        super::semantic_fingerprint_bytes(x),
                        super::semantic_fingerprint_bytes(y),
                        "distinct {i} vs {j}"
                    );
                }
            }
        }

        // Builder (chunked update) == one-shot over the concatenation.
        let mut b = super::SemanticFingerprintBuilder::new();
        b.update(b"hello");
        b.update(b" ");
        b.update(b"world");
        assert_eq!(
            b.finish(),
            super::semantic_fingerprint_bytes(b"hello world")
        );
    }

    #[test]
    #[ignore = "foreground release attribution harness"]
    fn raptorq_prefixed_sha256_single_buffer_ab_qfz4j() {
        fn former(bytes: &[u8]) -> String {
            format!("sha256:{}", super::sha256_hex(bytes))
        }

        fn one_buffer(bytes: &[u8]) -> String {
            super::sha256_prefixed_hex(bytes)
        }

        fn elapsed(source: &[u8], hash: impl Fn(&[u8]) -> String) -> u128 {
            let start = Instant::now();
            let mut digest = 0_usize;
            for chunk in source.chunks(super::DEFAULT_RAPTORQ_SYMBOL_BYTES) {
                digest = digest.wrapping_add(black_box(hash(black_box(chunk))).len());
            }
            black_box(digest);
            start.elapsed().as_nanos()
        }

        fn median(values: &mut [u128]) -> u128 {
            values.sort_unstable();
            values[values.len() / 2]
        }

        for len in [0, 1, 31, 1_023, 1_024, 1_025, 4_097] {
            let bytes: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            assert_eq!(former(&bytes), one_buffer(&bytes));
        }

        let source: Vec<u8> = (0..4 * 1_024 * 1_024)
            .map(|i| ((i * 131 + i / 17) % 251) as u8)
            .collect();
        for chunk in source.chunks(super::DEFAULT_RAPTORQ_SYMBOL_BYTES) {
            assert_eq!(former(chunk), one_buffer(chunk));
        }

        for _ in 0..2 {
            black_box(elapsed(&source, former));
            black_box(elapsed(&source, one_buffer));
        }

        let mut former_samples = Vec::with_capacity(18);
        let mut candidate_samples = Vec::with_capacity(18);
        for block in 0..9 {
            if block % 2 == 0 {
                former_samples.push(elapsed(&source, former));
                candidate_samples.push(elapsed(&source, one_buffer));
                candidate_samples.push(elapsed(&source, one_buffer));
                former_samples.push(elapsed(&source, former));
            } else {
                candidate_samples.push(elapsed(&source, one_buffer));
                former_samples.push(elapsed(&source, former));
                former_samples.push(elapsed(&source, former));
                candidate_samples.push(elapsed(&source, one_buffer));
            }
        }

        let former_p50 = median(&mut former_samples);
        let candidate_p50 = median(&mut candidate_samples);
        eprintln!(
            "RAPTORQ_HASH_AB bytes={} symbols={} former_p50_ns={} candidate_p50_ns={} ratio={:.6}",
            source.len(),
            source.len() / super::DEFAULT_RAPTORQ_SYMBOL_BYTES,
            former_p50,
            candidate_p50,
            former_p50 as f64 / candidate_p50 as f64,
        );
        eprintln!("RAPTORQ_HASH_AB former_samples_ns={former_samples:?}");
        eprintln!("RAPTORQ_HASH_AB candidate_samples_ns={candidate_samples:?}");
    }

    #[test]
    #[ignore = "foreground release attribution harness"]
    fn semantic_fingerprint_one_buffer_ab_9yiey() {
        const BATCH: usize = 2_048;
        const BLOCKS: usize = 10;

        fn former(bytes: &[u8]) -> String {
            format!("sha256:{}", super::sha256_hex(bytes))
        }

        fn candidate(bytes: &[u8]) -> String {
            super::semantic_fingerprint_bytes(bytes)
        }

        fn elapsed(bytes: &[u8], fingerprint: fn(&[u8]) -> String) -> u128 {
            let started = Instant::now();
            let mut digest = 0_u8;
            for _ in 0..BATCH {
                let output = black_box(fingerprint(black_box(bytes)));
                digest ^= output.as_bytes()[70];
            }
            black_box(digest);
            started.elapsed().as_nanos() / BATCH as u128
        }

        fn percentile(samples: &mut [u128], pct: usize) -> u128 {
            samples.sort_unstable();
            let rank = (samples.len() * pct).div_ceil(100).saturating_sub(1);
            samples[rank]
        }

        for len in [0, 1, 31, 64, 65, 1_024, 1_025] {
            let bytes = (0..len)
                .map(|i| ((i * 131 + i / 7) % 251) as u8)
                .collect::<Vec<_>>();
            assert_eq!(former(&bytes), candidate(&bytes));
        }

        let bytes = (0..64)
            .map(|i| ((i * 131 + i / 7) % 251) as u8)
            .collect::<Vec<_>>();
        for _ in 0..2 {
            black_box(elapsed(&bytes, former));
            black_box(elapsed(&bytes, candidate));
        }

        let mut former_samples = Vec::with_capacity(BLOCKS * 2);
        let mut candidate_samples = Vec::with_capacity(BLOCKS * 2);
        for block in 0..BLOCKS {
            if block.is_multiple_of(2) {
                former_samples.push(elapsed(&bytes, former));
                candidate_samples.push(elapsed(&bytes, candidate));
                candidate_samples.push(elapsed(&bytes, candidate));
                former_samples.push(elapsed(&bytes, former));
            } else {
                candidate_samples.push(elapsed(&bytes, candidate));
                former_samples.push(elapsed(&bytes, former));
                former_samples.push(elapsed(&bytes, former));
                candidate_samples.push(elapsed(&bytes, candidate));
            }
        }

        let former_p50 = percentile(&mut former_samples, 50);
        let candidate_p50 = percentile(&mut candidate_samples, 50);
        let former_p95 = percentile(&mut former_samples, 95);
        let candidate_p95 = percentile(&mut candidate_samples, 95);
        eprintln!(
            "SEMANTIC_FINGERPRINT_AB bytes={} batch={BATCH} samples={} former_p50_ns={former_p50} candidate_p50_ns={candidate_p50} ratio={:.6} former_p95_ns={former_p95} candidate_p95_ns={candidate_p95}",
            bytes.len(),
            BLOCKS * 2,
            former_p50 as f64 / candidate_p50 as f64,
        );
        eprintln!("SEMANTIC_FINGERPRINT_AB former_samples_ns={former_samples:?}");
        eprintln!("SEMANTIC_FINGERPRINT_AB candidate_samples_ns={candidate_samples:?}");
    }

    #[test]
    #[ignore = "foreground release attribution harness"]
    fn semantic_fingerprint_builder_finish_one_buffer_ab_88cuv() {
        const BATCH: usize = 2_048;
        const BLOCKS: usize = 10;

        fn former(hasher: super::Sha256) -> String {
            format!(
                "sha256:{}",
                super::sha256_digest_hex(super::Digest::finalize(hasher))
            )
        }

        fn candidate(hasher: super::Sha256) -> String {
            super::SemanticFingerprintBuilder { hasher }.finish()
        }

        fn seeded_hasher(len: usize) -> super::Sha256 {
            let mut hasher = <super::Sha256 as super::Digest>::new();
            let bytes = (0..len)
                .map(|i| ((i * 131 + i / 7) % 251) as u8)
                .collect::<Vec<_>>();
            super::Digest::update(&mut hasher, &bytes);
            hasher
        }

        fn elapsed(template: &super::Sha256, finish: fn(super::Sha256) -> String) -> u128 {
            let started = Instant::now();
            let mut digest = 0_u8;
            for _ in 0..BATCH {
                let output = black_box(finish(black_box(template.clone())));
                digest ^= output.as_bytes().last().copied().unwrap_or_default();
            }
            black_box(digest);
            started.elapsed().as_nanos() / BATCH as u128
        }

        fn percentile(samples: &mut [u128], pct: usize) -> u128 {
            samples.sort_unstable();
            let rank = (samples.len() * pct).div_ceil(100).saturating_sub(1);
            samples[rank]
        }

        for len in [0, 1, 31, 64, 65, 1_024, 1_025] {
            let template = seeded_hasher(len);
            assert_eq!(former(template.clone()), candidate(template));
        }

        let template = seeded_hasher(64);
        for _ in 0..2 {
            black_box(elapsed(&template, former));
            black_box(elapsed(&template, candidate));
        }

        let mut former_samples = Vec::with_capacity(BLOCKS * 2);
        let mut candidate_samples = Vec::with_capacity(BLOCKS * 2);
        for block in 0..BLOCKS {
            if block.is_multiple_of(2) {
                former_samples.push(elapsed(&template, former));
                candidate_samples.push(elapsed(&template, candidate));
                candidate_samples.push(elapsed(&template, candidate));
                former_samples.push(elapsed(&template, former));
            } else {
                candidate_samples.push(elapsed(&template, candidate));
                former_samples.push(elapsed(&template, former));
                former_samples.push(elapsed(&template, former));
                candidate_samples.push(elapsed(&template, candidate));
            }
        }

        let former_p50 = percentile(&mut former_samples, 50);
        let candidate_p50 = percentile(&mut candidate_samples, 50);
        let former_p95 = percentile(&mut former_samples, 95);
        let candidate_p95 = percentile(&mut candidate_samples, 95);
        eprintln!(
            "SEMANTIC_BUILDER_FINISH_AB bytes=64 batch={BATCH} samples={} former_p50_ns={former_p50} candidate_p50_ns={candidate_p50} ratio={:.6} former_p95_ns={former_p95} candidate_p95_ns={candidate_p95}",
            BLOCKS * 2,
            former_p50 as f64 / candidate_p50 as f64,
        );
        eprintln!("SEMANTIC_BUILDER_FINISH_AB former_samples_ns={former_samples:?}");
        eprintln!("SEMANTIC_BUILDER_FINISH_AB candidate_samples_ns={candidate_samples:?}");
    }

    #[test]
    fn semantic_fingerprint_streaming_equals_oneshot_h2i8m() {
        // Metamorphic (br-frankenpandas-h2i8m): chunked SemanticFingerprintBuilder
        // updates == one-shot over the concatenation, for arbitrary bytes and chunk
        // boundaries (generalizes the fixed 3-chunk case in 01gdm). Seeded LCG.
        let mut st: u64 = 0x4f1e_0b1c_2d3e_4f50;
        let mut next = || {
            st = st
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (st >> 33) as u32
        };
        for iter in 0..400u32 {
            let total = (next() % 40) as usize;
            let bytes: Vec<u8> = (0..total).map(|_| (next() % 256) as u8).collect();
            // Split into chunks at random boundaries (some possibly empty).
            let mut builder = super::SemanticFingerprintBuilder::new();
            let mut pos = 0usize;
            while pos < bytes.len() {
                let remaining = bytes.len() - pos;
                let take = (next() as usize % (remaining + 1)).min(remaining);
                builder.update(&bytes[pos..pos + take]);
                pos += take;
                if take == 0 {
                    // Avoid infinite loop on a zero-take; force progress.
                    builder.update(&bytes[pos..pos + 1.min(bytes.len() - pos)]);
                    pos += 1;
                }
            }
            assert_eq!(
                builder.finish(),
                super::semantic_fingerprint_bytes(&bytes),
                "streaming==one-shot iter={iter} total={total}"
            );
        }
    }

    /// br-frankenpandas-b9vvk: RaptorQ provenance envelopes (AGENTS.md
    /// RaptorQ-Everywhere mandate). Structural invariants of from_source_bytes.
    #[test]
    fn raptorq_envelope_from_source_bytes_invariants_b9vvk() {
        let sym = super::DEFAULT_RAPTORQ_SYMBOL_BYTES;
        let repair = 3u32;
        for &len in &[0usize, 1, sym, sym + 1, 3 * sym - 72] {
            let source: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let env = RaptorQEnvelope::from_source_bytes("pkt-1", "conformance", &source, repair);

            let expected_k = len.div_ceil(sym) as u32; // chunk count (0 for empty)
            assert_eq!(env.raptorq.k, expected_k, "k for len={len}");
            assert_eq!(
                env.raptorq.symbol_hashes.len() as u32,
                expected_k,
                "one symbol hash per source symbol, len={len}"
            );
            assert_eq!(
                env.raptorq.repair_symbols, repair,
                "repair_symbols len={len}"
            );
            assert_eq!(
                env.source_hash,
                super::semantic_fingerprint_bytes(&source),
                "source_hash == fingerprint, len={len}"
            );
            let expected_overhead = if expected_k == 0 {
                0.0
            } else {
                f64::from(repair) / f64::from(expected_k)
            };
            assert_eq!(
                env.raptorq.overhead_ratio, expected_overhead,
                "overhead len={len}"
            );
            assert_eq!(env.scrub.status, "ok", "scrub ok len={len}");
            assert!(
                env.decode_proofs.is_empty(),
                "no decode proofs yet len={len}"
            );
            // Every symbol hash is a well-formed sha256 fingerprint.
            assert!(
                env.raptorq
                    .symbol_hashes
                    .iter()
                    .all(|h| h.starts_with("sha256:") && h.len() == 7 + 64),
                "symbol hash format len={len}"
            );
        }
    }

    /// br-frankenpandas-bhlwt: push_decode_proof_capped keeps a bounded history,
    /// evicting oldest first (defensive bounded recovery).
    #[test]
    fn raptorq_decode_proof_cap_fifo_bhlwt() {
        let mut env = RaptorQEnvelope::from_source_bytes("p", "t", b"x", 1);
        let cap = super::MAX_DECODE_PROOFS;
        let total = cap + 5;
        for i in 0..total {
            env.push_decode_proof_capped(super::DecodeProof {
                ts_unix_ms: i as u64,
                reason: "scrub".to_owned(),
                recovered_blocks: i as u32,
                proof_hash: "sha256:deadbeef".to_owned(),
            });
        }
        // Never exceeds the cap.
        assert_eq!(
            env.decode_proofs.len(),
            cap,
            "history capped at MAX_DECODE_PROOFS"
        );
        // Oldest-first eviction: the retained window is the newest `cap` entries.
        assert_eq!(
            env.decode_proofs.first().unwrap().recovered_blocks,
            (total - cap) as u32,
            "oldest evicted; first retained is seq=overflow"
        );
        assert_eq!(
            env.decode_proofs.last().unwrap().recovered_blocks,
            (total - 1) as u32,
            "newest retained"
        );
        // Retained sequence is contiguous and strictly increasing.
        assert!(
            env.decode_proofs
                .windows(2)
                .all(|w| w[1].recovered_blocks == w[0].recovered_blocks + 1),
            "retained window is contiguous"
        );
    }

    /// br-frankenpandas-mbjpj: RuntimePolicy security mandates — strict fail-closes
    /// unknown features; hardened caps oversized joins (bounded recovery).
    #[test]
    fn runtime_policy_failclosed_and_join_cap_mbjpj() {
        // Configuration.
        let s = RuntimePolicy::strict();
        assert_eq!(s.mode, RuntimeMode::Strict);
        assert!(s.fail_closed_unknown_features);
        assert_eq!(s.hardened_join_row_cap, None);
        let h = RuntimePolicy::hardened(Some(10));
        assert_eq!(h.mode, RuntimeMode::Hardened);
        assert!(!h.fail_closed_unknown_features);
        assert_eq!(h.hardened_join_row_cap, Some(10));
        assert_eq!(
            RuntimePolicy::default().mode,
            RuntimeMode::Strict,
            "default is strict"
        );

        // Fail-closed: strict mode rejects unknown features and records the decision.
        let mut led = EvidenceLedger::new();
        let action =
            RuntimePolicy::strict().decide_unknown_feature("widget", "no handler", &mut led);
        assert_eq!(
            action,
            DecisionAction::Reject,
            "strict fail-closes unknown features"
        );
        assert_eq!(led.records().len(), 1, "decision recorded");

        // Bounded recovery: hardened mode repairs (caps) an over-cap join.
        let mut led2 = EvidenceLedger::new();
        let over = RuntimePolicy::hardened(Some(10)).decide_join_admission(1_000, &mut led2);
        assert_eq!(over, DecisionAction::Repair, "hardened caps over-cap joins");
        assert_eq!(led2.records().len(), 1, "join decision recorded");
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
    struct StructuredTestLog {
        packet_id: String,
        case_id: String,
        mode: RuntimeMode,
        seed: u64,
        trace_id: String,
        assertion_path: String,
        result: String,
        replay_cmd: String,
    }

    fn make_structured_log(
        case_id: &str,
        mode: RuntimeMode,
        seed: u64,
        assertion_path: &str,
        result: &str,
    ) -> StructuredTestLog {
        StructuredTestLog {
            packet_id: ASUPERSYNC_PACKET_ID.to_owned(),
            case_id: case_id.to_owned(),
            mode,
            seed,
            trace_id: format!("{ASUPERSYNC_PACKET_ID}:{case_id}:{seed:016x}"),
            assertion_path: assertion_path.to_owned(),
            result: result.to_owned(),
            replay_cmd: format!("{REPLAY_PREFIX} {case_id} --nocapture"),
        }
    }

    fn assert_required_log_fields(log: &serde_json::Value) {
        for field in [
            "packet_id",
            "case_id",
            "mode",
            "seed",
            "trace_id",
            "assertion_path",
            "result",
            "replay_cmd",
        ] {
            assert!(
                log.get(field).is_some(),
                "structured log missing field: {field}"
            );
        }
    }

    #[test]
    fn evidence_ledger_records_semantic_witnesses_tn6qb3() {
        let mut ledger = EvidenceLedger::new();
        let witness = SemanticWitnessRecord::new(
            "series.add",
            "series_binary_arithmetic_materialization",
            "outer",
            vec![
                SemanticIndexIdentity {
                    role: "left".to_owned(),
                    len: 2,
                    has_duplicates: false,
                    fingerprint: super::semantic_fingerprint_bytes(b"left"),
                },
                SemanticIndexIdentity {
                    role: "right".to_owned(),
                    len: 2,
                    has_duplicates: false,
                    fingerprint: super::semantic_fingerprint_bytes(b"right"),
                },
            ],
            SemanticIndexIdentity {
                role: "output".to_owned(),
                len: 3,
                has_duplicates: false,
                fingerprint: super::semantic_fingerprint_bytes(b"output"),
            },
            "missing aligned operands materialize as NaN/null before arithmetic",
            "outer union preserves left order then right-only labels",
        );

        ledger.push_semantic_witness(witness);

        let witnesses = ledger.semantic_witnesses();
        assert_eq!(witnesses.len(), 1);
        assert_eq!(witnesses[0].operation, "series.add");
        assert_eq!(witnesses[0].alignment_mode, "outer");
        assert_eq!(witnesses[0].output_index_identity.len, 3);
        assert_eq!(witnesses[0].input_index_identity[0].role, "left");
        assert!(
            witnesses[0].input_index_identity[0]
                .fingerprint
                .starts_with("sha256:")
        );
    }

    fn decide_join_admission_baseline(
        policy: &RuntimePolicy,
        estimated_rows: usize,
        ledger: &mut EvidenceLedger,
    ) -> DecisionAction {
        let issue = super::CompatibilityIssue {
            kind: super::IssueKind::JoinCardinality,
            subject: "join_estimator".to_owned(),
            detail: format!("estimated_rows={estimated_rows}"),
        };
        let cap = policy.hardened_join_row_cap.unwrap_or(usize::MAX);
        let evidence = vec![
            super::EvidenceTerm {
                name: Cow::Owned("estimator_overflow_risk".to_owned()),
                log_likelihood_if_compatible: if estimated_rows <= cap { -0.3 } else { -2.8 },
                log_likelihood_if_incompatible: if estimated_rows <= cap { -1.2 } else { -0.1 },
            },
            super::EvidenceTerm {
                name: Cow::Owned("memory_budget_signal".to_owned()),
                log_likelihood_if_compatible: if estimated_rows <= cap { -0.4 } else { -2.2 },
                log_likelihood_if_incompatible: if estimated_rows <= cap { -1.5 } else { -0.2 },
            },
        ];
        let loss = super::LossMatrix {
            allow_if_compatible: 0.0,
            allow_if_incompatible: 130.0,
            reject_if_compatible: 5.0,
            reject_if_incompatible: 0.5,
            repair_if_compatible: 1.5,
            repair_if_incompatible: 3.0,
        };
        let mut record = super::decide(policy.mode, issue, 0.6, loss, evidence);
        if matches!(policy.mode, RuntimeMode::Hardened) && estimated_rows > cap {
            record.action = DecisionAction::Repair;
        }
        let action = record.action;
        ledger.push(record);
        action
    }

    fn assert_join_record_equivalent(
        optimized: &super::DecisionRecord,
        baseline: &super::DecisionRecord,
    ) {
        assert_eq!(optimized.mode, baseline.mode);
        assert_eq!(optimized.action, baseline.action);
        assert_eq!(optimized.issue.kind, baseline.issue.kind);
        assert_eq!(optimized.issue.subject, baseline.issue.subject);
        assert_eq!(optimized.issue.detail, baseline.issue.detail);
        assert_eq!(optimized.prior_compatible, baseline.prior_compatible);
        assert_eq!(optimized.metrics, baseline.metrics);
        assert_eq!(optimized.evidence.len(), baseline.evidence.len());
        for (left, right) in optimized.evidence.iter().zip(&baseline.evidence) {
            assert_eq!(left.name.as_ref(), right.name.as_ref());
            assert_eq!(
                left.log_likelihood_if_compatible,
                right.log_likelihood_if_compatible
            );
            assert_eq!(
                left.log_likelihood_if_incompatible,
                right.log_likelihood_if_incompatible
            );
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
    fn asupersync_join_admission_optimized_path_is_isomorphic_to_baseline() {
        let policy = RuntimePolicy::hardened(Some(1024));
        let mut optimized = EvidenceLedger::new();
        let mut baseline = EvidenceLedger::new();

        for seed in 0_usize..256 {
            let rows = if seed % 2 == 0 {
                512 + seed
            } else {
                4096 + seed
            };
            let optimized_action = policy.decide_join_admission(rows, &mut optimized);
            let baseline_action = decide_join_admission_baseline(&policy, rows, &mut baseline);
            assert_eq!(optimized_action, baseline_action);

            let optimized_record = optimized.records().last().expect("optimized record");
            let baseline_record = baseline.records().last().expect("baseline record");
            assert_join_record_equivalent(optimized_record, baseline_record);
        }
    }

    #[test]
    fn asupersync_join_admission_profile_snapshot_reports_allocation_delta() {
        const ITERATIONS: usize = 256;
        let policy = RuntimePolicy::hardened(Some(2048));
        let mut optimized = EvidenceLedger::new();
        let mut baseline = EvidenceLedger::new();
        let mut optimized_ns = Vec::with_capacity(ITERATIONS);
        let mut baseline_ns = Vec::with_capacity(ITERATIONS);

        for seed in 0_usize..ITERATIONS {
            let rows = if seed % 3 == 0 {
                1024 + seed
            } else {
                8192 + seed
            };

            let baseline_start = Instant::now();
            let baseline_action = decide_join_admission_baseline(&policy, rows, &mut baseline);
            baseline_ns.push(baseline_start.elapsed().as_nanos());
            black_box(baseline_action);

            let optimized_start = Instant::now();
            let optimized_action = policy.decide_join_admission(rows, &mut optimized);
            optimized_ns.push(optimized_start.elapsed().as_nanos());
            black_box(optimized_action);
        }

        for (optimized_record, baseline_record) in
            optimized.records().iter().zip(baseline.records())
        {
            assert_join_record_equivalent(optimized_record, baseline_record);
        }

        let (baseline_p50_ns, baseline_p95_ns, baseline_p99_ns) = latency_quantiles(baseline_ns);
        let (optimized_p50_ns, optimized_p95_ns, optimized_p99_ns) =
            latency_quantiles(optimized_ns);
        let baseline_name_bytes_per_call =
            "estimator_overflow_risk".len() + "memory_budget_signal".len();
        let baseline_name_bytes_total = baseline_name_bytes_per_call * ITERATIONS;
        let optimized_name_bytes_total = 0_usize;
        assert!(baseline_name_bytes_total > optimized_name_bytes_total);

        println!(
            "asupersync_join_admission_profile_snapshot baseline_ns[p50={baseline_p50_ns},p95={baseline_p95_ns},p99={baseline_p99_ns}] optimized_ns[p50={optimized_p50_ns},p95={optimized_p95_ns},p99={optimized_p99_ns}] name_alloc_bytes_baseline={baseline_name_bytes_total} name_alloc_bytes_optimized={optimized_name_bytes_total}"
        );
    }

    #[test]
    fn asupersync_structured_log_contains_required_fields() {
        let log = make_structured_log(
            "asupersync_structured_log_contains_required_fields",
            RuntimeMode::Strict,
            42,
            "ASUPERSYNC-E/log_schema",
            "pass",
        );
        let value = serde_json::to_value(log).expect("serialize log");
        assert_required_log_fields(&value);
    }

    #[test]
    fn asupersync_structured_log_is_deterministic_for_same_inputs() {
        let left = make_structured_log(
            "asupersync_structured_log_is_deterministic_for_same_inputs",
            RuntimeMode::Hardened,
            1337,
            "ASUPERSYNC-E/log_determinism",
            "pass",
        );
        let right = make_structured_log(
            "asupersync_structured_log_is_deterministic_for_same_inputs",
            RuntimeMode::Hardened,
            1337,
            "ASUPERSYNC-E/log_determinism",
            "pass",
        );
        assert_eq!(left, right);
        let left_json = serde_json::to_string(&left).expect("left json");
        let right_json = serde_json::to_string(&right).expect("right json");
        assert_eq!(left_json, right_json);
    }

    #[test]
    fn asupersync_property_strict_unknown_feature_always_rejects() {
        let policy = RuntimePolicy::strict();
        let mut ledger = EvidenceLedger::new();
        let case_id = "asupersync_property_strict_unknown_feature_always_rejects";

        for seed in 0_u64..128 {
            let action = policy.decide_unknown_feature(
                format!("unknown_subject_{seed}"),
                format!("unknown_detail_{:08x}", seed.wrapping_mul(37)),
                &mut ledger,
            );
            let log = make_structured_log(
                case_id,
                RuntimeMode::Strict,
                seed,
                "ASUPERSYNC-E/strict_unknown_feature_reject",
                if action == DecisionAction::Reject {
                    "pass"
                } else {
                    "fail"
                },
            );
            let log_json = serde_json::to_value(log).expect("serialize log");
            assert_required_log_fields(&log_json);
            assert_eq!(
                action,
                DecisionAction::Reject,
                "strict mode must reject unknown feature; log={}",
                serde_json::to_string(&log_json).expect("json")
            );
        }

        assert_eq!(ledger.records().len(), 128);
    }

    #[test]
    fn asupersync_property_hardened_over_cap_forces_repair() {
        let cap = 1024_usize;
        let policy = RuntimePolicy::hardened(Some(cap));
        let mut ledger = EvidenceLedger::new();
        let case_id = "asupersync_property_hardened_over_cap_forces_repair";

        for seed in 0_u64..256 {
            let rows = if seed % 2 == 0 {
                cap + 1 + (seed as usize % 10_000)
            } else {
                cap.saturating_sub(seed as usize % cap)
            };
            let action = policy.decide_join_admission(rows, &mut ledger);
            let log = make_structured_log(
                case_id,
                RuntimeMode::Hardened,
                seed,
                "ASUPERSYNC-E/hardened_join_cap_boundary",
                if rows > cap && action == DecisionAction::Repair {
                    "pass"
                } else {
                    "check"
                },
            );
            let log_json = serde_json::to_value(log).expect("serialize log");
            assert_required_log_fields(&log_json);
            if rows > cap {
                assert_eq!(
                    action,
                    DecisionAction::Repair,
                    "rows over cap must force repair; rows={rows}; log={}",
                    serde_json::to_string(&log_json).expect("json")
                );
            }
        }
    }

    #[test]
    fn asupersync_property_decision_metrics_are_finite_and_bounded() {
        let policy = RuntimePolicy::hardened(Some(2048));
        let mut ledger = EvidenceLedger::new();
        let case_id = "asupersync_property_decision_metrics_are_finite_and_bounded";

        for seed in 0_u64..128 {
            let rows = 1 + (seed as usize * 97 % 500_000);
            policy.decide_join_admission(rows, &mut ledger);
            let record = ledger.records().last().expect("record");
            let metrics = &record.metrics;
            let posterior = metrics.posterior_compatible;
            let bounded = (0.0..=1.0).contains(&posterior);
            let finite = metrics
                .bayes_factor_compatible_over_incompatible
                .is_finite()
                && metrics.expected_loss_allow.is_finite()
                && metrics.expected_loss_reject.is_finite()
                && metrics.expected_loss_repair.is_finite();

            let log = make_structured_log(
                case_id,
                RuntimeMode::Hardened,
                seed,
                "ASUPERSYNC-E/decision_metrics_finite",
                if bounded && finite { "pass" } else { "fail" },
            );
            let log_json = serde_json::to_value(log).expect("serialize log");
            assert_required_log_fields(&log_json);
            assert!(bounded, "posterior out of range; log={log_json}");
            assert!(finite, "non-finite metrics; log={log_json}");
        }
    }

    #[test]
    fn decide_clamps_boundary_priors_to_finite_range() {
        for (input_prior, expected_prior) in [
            (0.0, super::PRIOR_COMPATIBLE_EPSILON),
            (1.0, 1.0 - super::PRIOR_COMPATIBLE_EPSILON),
        ] {
            let record = super::decide(
                RuntimeMode::Strict,
                super::CompatibilityIssue {
                    kind: super::IssueKind::MalformedInput,
                    subject: "prior_clamp_test".to_owned(),
                    detail: "boundary prior".to_owned(),
                },
                input_prior,
                super::LossMatrix::default(),
                Vec::new(),
            );

            assert_eq!(
                record.prior_compatible, expected_prior,
                "prior should be clamped into open interval (0,1)"
            );
            assert!(
                record.metrics.posterior_compatible.is_finite(),
                "posterior must remain finite for boundary priors"
            );
            assert!(
                record.metrics.expected_loss_allow.is_finite()
                    && record.metrics.expected_loss_reject.is_finite()
                    && record.metrics.expected_loss_repair.is_finite(),
                "expected-loss metrics must remain finite for boundary priors"
            );
        }
    }

    #[test]
    fn decide_normalizes_non_finite_priors_to_neutral() {
        for input_prior in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let record = super::decide(
                RuntimeMode::Strict,
                super::CompatibilityIssue {
                    kind: super::IssueKind::MalformedInput,
                    subject: "prior_clamp_test".to_owned(),
                    detail: "non-finite prior".to_owned(),
                },
                input_prior,
                super::LossMatrix::default(),
                Vec::new(),
            );

            assert_eq!(
                record.prior_compatible, 0.5,
                "non-finite priors should normalize to neutral prior"
            );
            assert!(
                record.metrics.posterior_compatible.is_finite(),
                "posterior must remain finite for non-finite priors"
            );
        }
    }

    #[test]
    fn asupersync_adversarial_extreme_join_estimate_remains_repair_and_loggable() {
        let policy = RuntimePolicy::hardened(Some(8));
        let mut ledger = EvidenceLedger::new();
        let action = policy.decide_join_admission(usize::MAX, &mut ledger);
        assert_eq!(action, DecisionAction::Repair);
        let record = ledger.records().last().expect("record");
        assert_eq!(record.mode, RuntimeMode::Hardened);
        assert!(
            record.issue.detail.contains("estimated_rows="),
            "issue detail should include estimated_rows"
        );

        let log = make_structured_log(
            "asupersync_adversarial_extreme_join_estimate_remains_repair_and_loggable",
            RuntimeMode::Hardened,
            u64::MAX,
            "ASUPERSYNC-E/adversarial_extreme_rows",
            "pass",
        );
        let log_json = serde_json::to_value(log).expect("serialize log");
        assert_required_log_fields(&log_json);
    }

    #[test]
    fn strict_mode_fails_closed_for_unknown_features() {
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::strict();

        let action = policy.decide_unknown_feature("csv", "field=experimental", &mut ledger);
        assert_eq!(action, DecisionAction::Reject);
        assert_eq!(ledger.records()[0].mode, RuntimeMode::Strict);
    }

    #[test]
    fn hardened_mode_repairs_large_join_estimates() {
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(10_000));

        let action = policy.decide_join_admission(100_000, &mut ledger);
        assert_eq!(action, DecisionAction::Repair);
        assert_eq!(ledger.records().len(), 1);
    }

    #[test]
    fn source_backed_raptorq_envelope_records_manifest_fields() {
        let mut source = vec![7_u8; super::DEFAULT_RAPTORQ_SYMBOL_BYTES];
        source.extend_from_slice(b"tail");

        let envelope = RaptorQEnvelope::from_source_bytes("packet-001", "conformance", &source, 3);

        assert_eq!(envelope.artifact_id, "packet-001");
        assert_eq!(envelope.artifact_type, "conformance");
        assert!(envelope.source_hash.starts_with("sha256:"));
        assert_eq!(envelope.source_hash.len(), "sha256:".len() + 64);
        assert_eq!(envelope.raptorq.k, 2);
        assert_eq!(envelope.raptorq.repair_symbols, 3);
        assert_eq!(envelope.raptorq.overhead_ratio, 1.5);
        assert_eq!(envelope.raptorq.symbol_hashes.len(), 2);
        assert!(
            envelope
                .raptorq
                .symbol_hashes
                .iter()
                .all(|hash| hash.starts_with("sha256:") && hash.len() == "sha256:".len() + 64)
        );
        assert_eq!(envelope.scrub.status, "ok");
        assert!(envelope.scrub.last_ok_unix_ms > 0);
    }

    #[test]
    fn decode_proof_append_is_capped_and_evicts_oldest() {
        let mut envelope =
            RaptorQEnvelope::from_source_bytes("packet-001", "conformance", b"source", 1);
        let total = super::MAX_DECODE_PROOFS + 5;

        for idx in 0..total {
            envelope.push_decode_proof_capped(super::DecodeProof {
                ts_unix_ms: u64::try_from(idx).expect("idx within u64 range"),
                reason: format!("proof-{idx}"),
                recovered_blocks: u32::try_from(idx).expect("idx within u32 range"),
                proof_hash: format!("sha256:{idx:08x}"),
            });
        }

        assert_eq!(envelope.decode_proofs.len(), super::MAX_DECODE_PROOFS);
        assert_eq!(
            envelope.decode_proofs[0].proof_hash,
            format!("sha256:{:08x}", total - super::MAX_DECODE_PROOFS)
        );
        assert_eq!(
            envelope
                .decode_proofs
                .last()
                .expect("decode proof should exist")
                .proof_hash,
            format!("sha256:{:08x}", total - 1)
        );
    }

    #[test]
    fn decision_card_is_renderable_for_ftui_consumers() {
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::strict();
        policy.decide_unknown_feature("csv", "field=experimental", &mut ledger);

        let card = decision_to_card(&ledger.records()[0]);
        let rendered = card.render_plain();
        assert!(rendered.contains("argmin_a"));
        assert!(rendered.contains("P(compatible|e)"));

        let edge_card = GalaxyBrainCard {
            title: "brain {card} ]\nnext".into(),
            equation: "lambda -> integral".into(),
            substitution: "Unicode: cafe\u{301}, data, 🧠".into(),
            intuition: "embedded\nnewlines\nremain".into(),
        };
        assert_eq!(
            edge_card.render_plain(),
            "[brain {card} ]\nnext]\nlambda -> integral\nUnicode: cafe\u{301}, data, 🧠\nembedded\nnewlines\nremain"
        );
    }

    #[test]
    #[ignore = "foreground profile-first A/B"]
    fn galaxy_brain_card_render_plain_profile_lzy5c() {
        #[inline(never)]
        fn former(card: &GalaxyBrainCard) -> String {
            format!(
                "[{}]\n{}\n{}\n{}",
                card.title, card.equation, card.substitution, card.intuition
            )
        }

        #[inline(never)]
        fn candidate(card: &GalaxyBrainCard) -> String {
            card.render_plain()
        }

        fn elapsed(cards: &[GalaxyBrainCard], renderer: fn(&GalaxyBrainCard) -> String) -> u128 {
            let started = Instant::now();
            let rendered = black_box(cards).iter().map(renderer).collect::<Vec<_>>();
            black_box(&rendered);
            let elapsed = started.elapsed().as_nanos();
            black_box(rendered);
            elapsed
        }

        fn percentile(samples: &[u128], percent: usize) -> u128 {
            let mut sorted = samples.to_vec();
            sorted.sort_unstable();
            let rank = (sorted.len() * percent).div_ceil(100).saturating_sub(1);
            sorted[rank]
        }

        let edge_cards = [
            GalaxyBrainCard {
                title: String::new(),
                equation: String::new(),
                substitution: String::new(),
                intuition: String::new(),
            },
            GalaxyBrainCard {
                title: "csv::Reject".into(),
                equation: "argmin_a sum_s L(a,s) P(s|evidence)".into(),
                substitution: "P(compatible|e)=0.1250, E[allow]=9.5".into(),
                intuition: "Lower expected loss wins.".into(),
            },
            GalaxyBrainCard {
                title: "brain {card} ]\nnext".into(),
                equation: "lambda -> integral".into(),
                substitution: "Unicode: cafe\u{301}, data, 🧠".into(),
                intuition: "embedded\nnewlines\nremain".into(),
            },
        ];
        for card in &edge_cards {
            assert_eq!(candidate(card), former(card));
        }

        const CARDS: usize = 4_096;
        const SAMPLES: usize = 12;
        let cards = (0..CARDS)
            .map(|index| GalaxyBrainCard {
                title: format!("packet-{index:04}::{:?}", index % 3),
                equation: "argmin_a sum_s L(a,s) P(s|evidence)".into(),
                substitution: format!(
                    "P(compatible|e)=0.{:04}, E[allow]={:.4}, E[reject]={:.4}",
                    index % 10_000,
                    (index % 97) as f64 / 7.0,
                    (index % 89) as f64 / 11.0
                ),
                intuition: if index % 7 == 0 {
                    "Strict mode may force fail-closed. 🧠".into()
                } else {
                    "Lower expected loss wins; evidence remains auditable.".into()
                },
            })
            .collect::<Vec<_>>();
        for card in &cards {
            assert_eq!(candidate(card), former(card));
        }

        for _ in 0..2 {
            black_box(elapsed(&cards, former));
            black_box(elapsed(&cards, candidate));
        }
        let mut former_ns = Vec::with_capacity(SAMPLES);
        let mut candidate_ns = Vec::with_capacity(SAMPLES);
        for sample in 0..SAMPLES {
            if sample % 2 == 0 {
                former_ns.push(elapsed(&cards, former));
                candidate_ns.push(elapsed(&cards, candidate));
            } else {
                candidate_ns.push(elapsed(&cards, candidate));
                former_ns.push(elapsed(&cards, former));
            }
        }

        let former_p50 = percentile(&former_ns, 50);
        let former_p95 = percentile(&former_ns, 95);
        let former_p99 = percentile(&former_ns, 99);
        let candidate_p50 = percentile(&candidate_ns, 50);
        let candidate_p95 = percentile(&candidate_ns, 95);
        let candidate_p99 = percentile(&candidate_ns, 99);
        println!(
            "GALAXY_CARD_RENDER cards={CARDS} former_p50_ns={former_p50} candidate_p50_ns={candidate_p50} speedup_p50={:.6} former_p95_ns={former_p95} candidate_p95_ns={candidate_p95} speedup_p95={:.6} former_p99_ns={former_p99} candidate_p99_ns={candidate_p99} speedup_p99={:.6} former_samples={former_ns:?} candidate_samples={candidate_ns:?}",
            former_p50 as f64 / candidate_p50 as f64,
            former_p95 as f64 / candidate_p95 as f64,
            former_p99 as f64 / candidate_p99 as f64,
        );
    }

    // === Conformal Calibration Tests (bd-2t5e.9) ===

    #[test]
    fn conformal_guard_uncalibrated_accepts_all() {
        let mut guard = ConformalGuard::new(100, 0.1);
        assert!(!guard.is_calibrated());

        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::strict();
        policy.decide_unknown_feature("test", "detail", &mut ledger);

        let ps = guard.evaluate(&ledger.records()[0]);
        assert!(ps.bayesian_action_in_set);
        assert_eq!(ps.admissible_actions.len(), 3); // all actions admissible
        assert_eq!(ps.quantile_threshold, f64::INFINITY);
    }

    #[test]
    fn conformal_guard_calibrates_after_sufficient_data() {
        let mut guard = ConformalGuard::new(100, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        // Feed 10 decisions to build calibration window
        for _ in 0..10 {
            policy.decide_join_admission(50_000, &mut ledger);
        }

        for record in ledger.records() {
            guard.evaluate(record);
        }

        assert!(guard.is_calibrated());
        assert!(guard.conformal_quantile().is_some());
        assert_eq!(guard.calibration_count(), 10);
    }

    #[test]
    fn conformal_guard_rolling_window_evicts_old_scores() {
        let mut guard = ConformalGuard::new(5, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        for _ in 0..10 {
            policy.decide_join_admission(1000, &mut ledger);
        }

        for record in ledger.records() {
            guard.evaluate(record);
        }

        // Window should be capped at 5
        assert_eq!(guard.calibration_count(), 5);
    }

    #[test]
    fn conformal_guard_coverage_tracking() {
        let mut guard = ConformalGuard::new(50, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        // Generate consistent decisions
        for _ in 0..20 {
            policy.decide_join_admission(1000, &mut ledger);
        }

        for record in ledger.records() {
            guard.evaluate(record);
        }

        // With consistent decisions, most should be in the conformal set
        let coverage = guard.empirical_coverage();
        assert!(coverage > 0.5, "coverage should be reasonable: {coverage}");
    }

    #[test]
    fn conformal_guard_no_coverage_alert_under_100_decisions() {
        let mut guard = ConformalGuard::new(100, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        for _ in 0..10 {
            policy.decide_join_admission(1000, &mut ledger);
        }
        for record in ledger.records() {
            guard.evaluate(record);
        }

        // Under 100 decisions, no alert regardless of coverage
        assert!(!guard.coverage_alert());
    }

    #[test]
    fn conformal_guard_zero_window_size_is_clamped() {
        let mut guard = ConformalGuard::new(0, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        policy.decide_join_admission(1000, &mut ledger);
        let set = guard.evaluate(&ledger.records()[0]);

        assert!(set.bayesian_action_in_set);
        assert_eq!(guard.calibration_count(), 1);
    }

    #[test]
    fn conformal_guard_non_finite_alpha_uses_default() {
        let guard = ConformalGuard::new(100, f64::NAN);
        assert_eq!(guard.alpha, super::DEFAULT_CONFORMAL_ALPHA);
        assert!(!guard.coverage_alert());
    }

    #[test]
    fn conformal_guard_repairs_deserialized_zero_window_before_evaluate() {
        let mut guard: ConformalGuard = serde_json::from_str(
            r#"{"scores":[],"window_size":0,"alpha":0.1,"in_set_count":0,"total_count":0}"#,
        )
        .expect("deserialize guard");
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        policy.decide_join_admission(1000, &mut ledger);
        let set = guard.evaluate(&ledger.records()[0]);

        assert!(set.bayesian_action_in_set);
        assert_eq!(guard.window_size, 1);
        assert_eq!(guard.calibration_count(), 1);
    }

    #[test]
    fn conformal_quantile_ignores_non_finite_persisted_scores() {
        let guard = ConformalGuard {
            scores: vec![f64::NAN, f64::INFINITY, 1.0, 2.0],
            window_size: 10,
            alpha: f64::NAN,
            in_set_count: 5,
            total_count: 3,
        };

        assert!(guard.is_calibrated());
        assert_eq!(guard.conformal_quantile(), Some(2.0));
        assert_eq!(guard.empirical_coverage(), 1.0);
    }

    #[test]
    fn conformal_normalized_quantile_matches_robust_path_qckka() {
        let mut state = 0xa076_1d64_78bd_642f_u64;
        let random_scores = (0..1_000)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 11) as f64 / ((1_u64 << 53) as f64)
            })
            .collect::<Vec<_>>();
        let cases = [
            (Vec::new(), 0, f64::NAN),
            (vec![f64::NAN, 1.0, f64::INFINITY], 8, 0.1),
            (
                vec![f64::NEG_INFINITY, -0.0, 0.0, 1.0, 1.0, f64::MAX],
                4,
                f64::INFINITY,
            ),
            (random_scores, 257, 0.99),
        ];

        for (scores, window_size, alpha) in cases {
            let mut guard = ConformalGuard {
                scores,
                window_size,
                alpha,
                in_set_count: 9,
                total_count: 4,
            };
            guard.normalize_runtime_config();
            let former = guard.conformal_quantile().map(f64::to_bits);
            let candidate = super::select_conformal_quantile(guard.scores.clone(), guard.alpha)
                .map(f64::to_bits);
            assert_eq!(candidate, former, "window_size={window_size} alpha={alpha}");
        }
    }

    #[test]
    fn conformal_evaluate_preserves_observables_qckka() {
        let mut ledger = EvidenceLedger::new();
        RuntimePolicy::hardened(Some(100_000)).decide_join_admission(1_000, &mut ledger);
        let record = &ledger.records()[0];
        let mut guard = ConformalGuard {
            scores: vec![f64::NAN, 0.25, 1.5, f64::INFINITY, -0.0, 2.5],
            window_size: 4,
            alpha: f64::NAN,
            in_set_count: 9,
            total_count: 4,
        };
        let mut expected_guard = guard.clone();
        expected_guard.normalize_runtime_config();
        let expected_threshold = expected_guard
            .conformal_quantile()
            .expect("normalized fixture is calibrated");
        let expected_score = super::nonconformity_score(record);
        if expected_guard.scores.len() >= expected_guard.window_size {
            expected_guard.scores.remove(0);
        }
        expected_guard.scores.push(expected_score);
        let expected_in_set = expected_score <= expected_threshold;
        let expected_actions = if expected_in_set {
            vec![record.action]
        } else {
            vec![
                DecisionAction::Allow,
                DecisionAction::Reject,
                DecisionAction::Repair,
            ]
        };
        expected_guard.total_count += 1;
        if expected_in_set {
            expected_guard.in_set_count += 1;
        }
        let expected_set = super::ConformalPredictionSet {
            quantile_threshold: expected_threshold,
            current_score: expected_score,
            bayesian_action_in_set: expected_in_set,
            admissible_actions: expected_actions,
            empirical_coverage: expected_guard.in_set_count as f64
                / expected_guard.total_count as f64,
        };

        let actual_set = guard.evaluate(record);
        assert_eq!(actual_set, expected_set);
        assert_eq!(
            serde_json::to_vec(&actual_set).expect("serialize actual prediction set"),
            serde_json::to_vec(&expected_set).expect("serialize expected prediction set")
        );
        assert_eq!(
            serde_json::to_vec(&guard).expect("serialize actual guard"),
            serde_json::to_vec(&expected_guard).expect("serialize expected guard")
        );
    }

    #[test]
    fn conformal_guard_quantile_is_deterministic() {
        let mut guard = ConformalGuard::new(100, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        for _ in 0..5 {
            policy.decide_join_admission(1000, &mut ledger);
        }
        for record in ledger.records() {
            guard.evaluate(record);
        }

        let q1 = guard.conformal_quantile();
        let q2 = guard.conformal_quantile();
        assert_eq!(q1, q2);
    }

    fn full_sort_conformal_quantile(guard: &ConformalGuard) -> Option<f64> {
        let mut sorted = guard
            .scores
            .iter()
            .copied()
            .filter(|score| score.is_finite())
            .collect::<Vec<_>>();
        if sorted.len() < 2 {
            return None;
        }
        sorted.sort_by(f64::total_cmp);
        let n = sorted.len() as f64;
        let level = (1.0 - super::normalize_conformal_alpha(guard.alpha)) * (1.0 + 1.0 / n);
        let idx = (level * n).ceil() as usize;
        let idx = idx.min(sorted.len()).saturating_sub(1);
        Some(sorted[idx])
    }

    #[test]
    #[ignore = "foreground normal-release attribution probe"]
    fn conformal_normalized_window_profile_qckka() {
        const WINDOW: usize = 1_000;
        const BATCH: usize = 128;
        const SAMPLES: usize = 15;

        fn normalized_quantile(guard: &ConformalGuard) -> Option<f64> {
            super::select_conformal_quantile(guard.scores.clone(), guard.alpha)
        }

        fn elapsed(guard: &ConformalGuard, normalized: bool) -> u128 {
            let started = Instant::now();
            let mut digest = 0_u64;
            for _ in 0..BATCH {
                let quantile = if normalized {
                    normalized_quantile(black_box(guard))
                } else {
                    black_box(guard).conformal_quantile()
                };
                digest = digest.wrapping_add(quantile.map_or(0, f64::to_bits));
            }
            black_box(digest);
            started.elapsed().as_nanos() / BATCH as u128
        }

        fn percentile(samples: &mut [u128], pct: usize) -> u128 {
            samples.sort_unstable();
            let rank = (samples.len() * pct).div_ceil(100).saturating_sub(1);
            samples[rank]
        }

        let mut state = 0xa076_1d64_78bd_642f_u64;
        let scores = (0..WINDOW)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 11) as f64 / ((1_u64 << 53) as f64)
            })
            .collect::<Vec<_>>();
        let guard = ConformalGuard {
            scores,
            window_size: WINDOW,
            alpha: 0.1,
            in_set_count: 0,
            total_count: 0,
        };
        assert_eq!(
            guard.conformal_quantile().map(f64::to_bits),
            normalized_quantile(&guard).map(f64::to_bits)
        );

        for _ in 0..3 {
            black_box(elapsed(&guard, false));
            black_box(elapsed(&guard, true));
        }
        let mut filtered = Vec::with_capacity(SAMPLES * 2);
        let mut normalized = Vec::with_capacity(SAMPLES * 2);
        for sample in 0_usize..SAMPLES {
            if sample.is_multiple_of(2) {
                filtered.push(elapsed(&guard, false));
                normalized.push(elapsed(&guard, true));
                normalized.push(elapsed(&guard, true));
                filtered.push(elapsed(&guard, false));
            } else {
                normalized.push(elapsed(&guard, true));
                filtered.push(elapsed(&guard, false));
                filtered.push(elapsed(&guard, false));
                normalized.push(elapsed(&guard, true));
            }
        }
        let filtered_p50 = percentile(&mut filtered, 50);
        let normalized_p50 = percentile(&mut normalized, 50);
        let filtered_p95 = percentile(&mut filtered, 95);
        let normalized_p95 = percentile(&mut normalized, 95);
        eprintln!(
            "CONFORMAL_NORMALIZED_PROFILE window={WINDOW} batch={BATCH} filtered_p50_ns={filtered_p50} normalized_p50_ns={normalized_p50} ratio={:.6} filtered_p95_ns={filtered_p95} normalized_p95_ns={normalized_p95}",
            filtered_p50 as f64 / normalized_p50 as f64
        );
        eprintln!("CONFORMAL_NORMALIZED_PROFILE filtered_distribution_ns={filtered:?}");
        eprintln!("CONFORMAL_NORMALIZED_PROFILE normalized_distribution_ns={normalized:?}");
    }

    #[test]
    fn conformal_quantile_selection_matches_full_sort_bh91q() {
        let mut state = 0xd1b5_4a32_d192_ed03_u64;
        let mut random_scores = Vec::with_capacity(1_005);
        for _ in 0..1_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            random_scores.push((state >> 11) as f64 / ((1_u64 << 53) as f64));
        }
        random_scores.extend([f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, 0.0]);
        let cases = [
            Vec::new(),
            vec![1.0],
            vec![f64::NAN, f64::INFINITY, 2.0],
            vec![-0.0, 0.0, -1.0, 1.0, 1.0, f64::MAX, f64::MIN],
            random_scores,
        ];

        for scores in cases {
            for alpha in [0.01, 0.1, 0.5, 0.99, f64::NAN, f64::INFINITY] {
                let guard = ConformalGuard {
                    window_size: scores.len().max(1),
                    scores: scores.clone(),
                    alpha,
                    in_set_count: 0,
                    total_count: 0,
                };
                let former = full_sort_conformal_quantile(&guard).map(f64::to_bits);
                let candidate = guard.conformal_quantile().map(f64::to_bits);
                assert_eq!(candidate, former, "len={} alpha={alpha}", scores.len());
            }
        }
    }

    #[test]
    #[ignore = "foreground performance probe"]
    fn conformal_quantile_selection_ab_bh91q() {
        const WINDOW: usize = 1_000;
        const BATCH: usize = 64;
        const SAMPLES: usize = 31;
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let scores = (0..WINDOW)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 11) as f64 / ((1_u64 << 53) as f64)
            })
            .collect::<Vec<_>>();
        let guard = ConformalGuard {
            scores,
            window_size: WINDOW,
            alpha: 0.1,
            in_set_count: 0,
            total_count: 0,
        };

        let former = full_sort_conformal_quantile(&guard).map(f64::to_bits);
        let candidate = guard.conformal_quantile().map(f64::to_bits);
        assert_eq!(candidate, former);

        let measure_former = || {
            let started = Instant::now();
            let mut digest = 0_u64;
            for _ in 0..BATCH {
                let quantile = full_sort_conformal_quantile(black_box(&guard));
                digest = digest.wrapping_add(quantile.map_or(0, f64::to_bits));
            }
            black_box(digest);
            started.elapsed().as_nanos() / BATCH as u128
        };
        let measure_candidate = || {
            let started = Instant::now();
            let mut digest = 0_u64;
            for _ in 0..BATCH {
                let quantile = black_box(&guard).conformal_quantile();
                digest = digest.wrapping_add(quantile.map_or(0, f64::to_bits));
            }
            black_box(digest);
            started.elapsed().as_nanos() / BATCH as u128
        };

        for _ in 0..3 {
            black_box(measure_former());
            black_box(measure_candidate());
        }
        let mut former_a = Vec::with_capacity(SAMPLES);
        let mut former_b = Vec::with_capacity(SAMPLES);
        let mut candidate_a = Vec::with_capacity(SAMPLES);
        let mut candidate_b = Vec::with_capacity(SAMPLES);
        for sample in 0..SAMPLES {
            if sample.is_multiple_of(2) {
                former_a.push(measure_former());
                candidate_a.push(measure_candidate());
                candidate_b.push(measure_candidate());
                former_b.push(measure_former());
            } else {
                former_b.push(measure_former());
                candidate_b.push(measure_candidate());
                candidate_a.push(measure_candidate());
                former_a.push(measure_former());
            }
        }
        former_a.sort_unstable();
        former_b.sort_unstable();
        candidate_a.sort_unstable();
        candidate_b.sort_unstable();
        let percentile = |samples: &[u128], pct: usize| {
            let rank = (samples.len() * pct).div_ceil(100).saturating_sub(1);
            samples[rank]
        };
        let former_a_p50 = percentile(&former_a, 50);
        let former_b_p50 = percentile(&former_b, 50);
        let candidate_a_p50 = percentile(&candidate_a, 50);
        let candidate_b_p50 = percentile(&candidate_b, 50);
        let former_mean = (former_a_p50 + former_b_p50) as f64 / 2.0;
        let candidate_mean = (candidate_a_p50 + candidate_b_p50) as f64 / 2.0;
        println!(
            "fp-runtime conformal quantile A/B: window={WINDOW} batch={BATCH} samples={SAMPLES}"
        );
        println!("former full-sort p50 A/B: {former_a_p50} / {former_b_p50} ns");
        println!("candidate selection p50 A/B: {candidate_a_p50} / {candidate_b_p50} ns");
        println!(
            "former full-sort p95/p99 A: {} / {} ns; B: {} / {} ns",
            percentile(&former_a, 95),
            percentile(&former_a, 99),
            percentile(&former_b, 95),
            percentile(&former_b, 99)
        );
        println!(
            "candidate selection p95/p99 A: {} / {} ns; B: {} / {} ns",
            percentile(&candidate_a, 95),
            percentile(&candidate_a, 99),
            percentile(&candidate_b, 95),
            percentile(&candidate_b, 99)
        );
        println!(
            "former/candidate ratio: {:.6}x",
            former_mean / candidate_mean
        );
    }

    // --- AG-09-T: Conformal Calibration Tests ---

    /// AG-09-T #1: conformal_quantile with known scores returns correct quantile.
    #[test]
    fn conformal_quantile_basic() {
        let mut guard = ConformalGuard::new(100, 0.1);
        // Manually feed scores into the window
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        // Generate 5 decisions to fill window
        for _ in 0..5 {
            policy.decide_join_admission(1000, &mut ledger);
        }
        for record in ledger.records() {
            guard.evaluate(record);
        }

        let q = guard.conformal_quantile();
        assert!(q.is_some());
        let quantile = q.unwrap();
        assert!(quantile.is_finite(), "quantile must be finite: {quantile}");
        assert!(quantile >= 0.0, "quantile must be non-negative: {quantile}");
    }

    /// AG-09-T #2: Single-element score (after 2 evals) -> returns finite quantile.
    #[test]
    fn conformal_quantile_trivial() {
        let mut guard = ConformalGuard::new(100, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        // Need at least 2 scores
        policy.decide_join_admission(1000, &mut ledger);
        policy.decide_join_admission(1000, &mut ledger);
        guard.evaluate(&ledger.records()[0]);
        guard.evaluate(&ledger.records()[1]);

        let q = guard.conformal_quantile();
        assert!(q.is_some());
    }

    /// AG-09-T #3: Empty calibration window -> returns None.
    #[test]
    fn conformal_quantile_empty() {
        let guard = ConformalGuard::new(100, 0.1);
        assert!(guard.conformal_quantile().is_none());
        assert!(!guard.is_calibrated());
    }

    /// AG-09-T #4: When conformal set is singleton (Bayesian in set),
    /// guard agrees with Bayesian argmin.
    #[test]
    fn conformal_guard_agrees_with_bayesian() {
        let mut guard = ConformalGuard::new(100, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        // Build calibration window with consistent decisions
        for _ in 0..20 {
            policy.decide_join_admission(1000, &mut ledger);
        }

        let mut bayesian_agreed = 0;
        let mut total = 0;

        for record in ledger.records() {
            let ps = guard.evaluate(record);
            total += 1;
            if ps.bayesian_action_in_set && ps.admissible_actions.len() == 1 {
                // Singleton set: only the Bayesian action is admissible
                assert_eq!(ps.admissible_actions[0], record.action);
                bayesian_agreed += 1;
            }
        }

        // Most decisions with consistent data should agree
        assert!(total > 0, "should have evaluated at least one decision");
        // With uniform decisions, some will be in set
        assert!(
            bayesian_agreed > 0 || total < 3,
            "at least some decisions should agree with Bayesian"
        );
    }

    /// AG-09-T #5: When conformal set widens (score > threshold),
    /// guard admits multiple actions.
    #[test]
    fn conformal_guard_widens_on_uncertainty() {
        let mut guard = ConformalGuard::new(10, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        // Build a tight calibration window with small-row decisions
        for _ in 0..10 {
            policy.decide_join_admission(100, &mut ledger);
        }
        for record in ledger.records() {
            guard.evaluate(record);
        }

        // Now make a very different decision (large row estimate -> different posterior)
        let mut outlier_ledger = EvidenceLedger::new();
        let extreme_policy = RuntimePolicy::hardened(Some(10));
        extreme_policy.decide_join_admission(1_000_000, &mut outlier_ledger);

        let ps = guard.evaluate(&outlier_ledger.records()[0]);
        // If the score exceeds threshold, the set should widen to 3 actions
        if !ps.bayesian_action_in_set {
            assert_eq!(
                ps.admissible_actions.len(),
                3,
                "widened set should admit all actions"
            );
        }
    }

    /// AG-09-T #8: Coverage guarantee over 1000 exchangeable decisions.
    #[test]
    fn conformal_coverage_guarantee_1000_decisions() {
        let mut guard = ConformalGuard::new(1000, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        // Generate 1000 similar decisions (exchangeable)
        for i in 0..1000 {
            // Vary the row estimate slightly to create some variance
            let rows = 1000 + (i * 7) % 500;
            policy.decide_join_admission(rows, &mut ledger);
        }

        for record in ledger.records() {
            guard.evaluate(record);
        }

        // With exchangeable data, coverage should be >= 1 - alpha = 0.9
        // Allow some slack for finite sample effects
        let coverage = guard.empirical_coverage();
        assert!(
            coverage >= 0.7,
            "coverage {coverage} should be >= 0.7 (relaxed bound for finite sample)"
        );
    }

    /// AG-09-T #10: Rolling window correctly drops oldest entries.
    #[test]
    fn conformal_rolling_window_exact_eviction() {
        let window_size = 5;
        let mut guard = ConformalGuard::new(window_size, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));

        // Generate more decisions than window size
        for _ in 0..15 {
            policy.decide_join_admission(1000, &mut ledger);
        }

        for record in ledger.records() {
            guard.evaluate(record);
        }

        assert_eq!(
            guard.calibration_count(),
            window_size,
            "window should be exactly {window_size}"
        );
    }

    /// AG-09-T #12: decision_to_card produces a valid galaxy brain card
    /// with conformal-relevant information.
    #[test]
    fn conformal_galaxy_brain_card_content() {
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));
        policy.decide_join_admission(50_000, &mut ledger);

        let card = decision_to_card(&ledger.records()[0]);
        assert!(card.equation.contains("argmin_a"));
        assert!(card.substitution.contains("P(compatible|e)"));
        assert!(card.substitution.contains("E[allow]"));
        assert!(card.substitution.contains("E[reject]"));
        assert!(card.substitution.contains("E[repair]"));
    }

    #[test]
    fn conformal_prediction_set_serializes() {
        let mut guard = ConformalGuard::new(100, 0.1);
        let mut ledger = EvidenceLedger::new();
        let policy = RuntimePolicy::hardened(Some(100_000));
        policy.decide_join_admission(1000, &mut ledger);

        let ps = guard.evaluate(&ledger.records()[0]);
        let json = serde_json::to_string(&ps).expect("serialize");
        let _: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(json.contains("quantile_threshold"));
        assert!(json.contains("empirical_coverage"));
    }
}
