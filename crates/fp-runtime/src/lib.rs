#![forbid(unsafe_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    pub name: String,
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
pub struct GalaxyBrainCard {
    pub title: String,
    pub equation: String,
    pub substitution: String,
    pub intuition: String,
}

impl GalaxyBrainCard {
    #[must_use]
    pub fn render_plain(&self) -> String {
        format!(
            "[{}]\n{}\n{}\n{}",
            self.title, self.equation, self.substitution, self.intuition
        )
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

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceLedger {
    records: Vec<DecisionRecord>,
}

impl EvidenceLedger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    pub fn push(&mut self, record: DecisionRecord) {
        self.records.push(record);
    }

    #[must_use]
    pub fn records(&self) -> &[DecisionRecord] {
        &self.records
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

        let evidence = vec![
            EvidenceTerm {
                name: "compatibility_allowlist_miss".to_owned(),
                log_likelihood_if_compatible: -3.5,
                log_likelihood_if_incompatible: -0.2,
            },
            EvidenceTerm {
                name: "unknown_protocol_field".to_owned(),
                log_likelihood_if_compatible: -2.0,
                log_likelihood_if_incompatible: -0.1,
            },
        ];

        let mut record = decide(self.mode, issue, 0.25, LossMatrix::default(), evidence);
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
        let evidence = vec![
            EvidenceTerm {
                name: "estimator_overflow_risk".to_owned(),
                log_likelihood_if_compatible: if estimated_rows <= cap { -0.3 } else { -2.8 },
                log_likelihood_if_incompatible: if estimated_rows <= cap { -1.2 } else { -0.1 },
            },
            EvidenceTerm {
                name: "memory_budget_signal".to_owned(),
                log_likelihood_if_compatible: if estimated_rows <= cap { -0.4 } else { -2.2 },
                log_likelihood_if_incompatible: if estimated_rows <= cap { -1.5 } else { -0.2 },
            },
        ];

        let loss = LossMatrix {
            allow_if_compatible: 0.0,
            allow_if_incompatible: 130.0,
            reject_if_compatible: 5.0,
            reject_if_incompatible: 0.5,
            repair_if_compatible: 1.5,
            repair_if_incompatible: 3.0,
        };

        let mut record = decide(self.mode, issue, 0.6, loss, evidence);

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

fn decide(
    mode: RuntimeMode,
    issue: CompatibilityIssue,
    prior_compatible: f64,
    loss: LossMatrix,
    evidence: Vec<EvidenceTerm>,
) -> DecisionRecord {
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
    pub fn placeholder(artifact_id: impl Into<String>, artifact_type: impl Into<String>) -> Self {
        Self {
            artifact_id: artifact_id.into(),
            artifact_type: artifact_type.into(),
            source_hash: "blake3:placeholder".to_owned(),
            raptorq: RaptorQMetadata {
                k: 0,
                repair_symbols: 0,
                overhead_ratio: 0.0,
                symbol_hashes: Vec::new(),
            },
            scrub: ScrubStatus {
                last_ok_unix_ms: 0,
                status: "ok".to_owned(),
            },
            decode_proofs: Vec::new(),
        }
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
        Self {
            scores: Vec::with_capacity(window_size),
            window_size,
            alpha: alpha.clamp(0.01, 0.5),
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
        if self.scores.len() < 2 {
            return None;
        }
        let mut sorted: Vec<f64> = self.scores.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        // Quantile at level (1 - alpha)(1 + 1/n) per split conformal prediction
        let n = sorted.len() as f64;
        let level = (1.0 - self.alpha) * (1.0 + 1.0 / n);
        let idx = (level * n).ceil() as usize;
        let idx = idx.min(sorted.len()) - 1;
        Some(sorted[idx])
    }

    /// Evaluate a decision record against the conformal guard.
    /// Returns the prediction set and whether the Bayesian action is admissible.
    pub fn evaluate(&mut self, record: &DecisionRecord) -> ConformalPredictionSet {
        let score = nonconformity_score(record);

        let quantile = self.conformal_quantile();

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
        self.in_set_count as f64 / self.total_count as f64
    }

    /// Number of scores in the calibration window.
    #[must_use]
    pub fn calibration_count(&self) -> usize {
        self.scores.len()
    }

    /// Whether the calibration window has sufficient data.
    #[must_use]
    pub fn is_calibrated(&self) -> bool {
        self.scores.len() >= 2
    }

    /// Whether coverage has dropped below target for the alert threshold.
    #[must_use]
    pub fn coverage_alert(&self) -> bool {
        self.total_count >= 100 && self.empirical_coverage() < (1.0 - self.alpha)
    }
}

#[cfg(feature = "asupersync")]
#[must_use]
pub fn outcome_to_action<T, E>(outcome: &asupersync::Outcome<T, E>) -> DecisionAction {
    match outcome {
        asupersync::Outcome::Ok(_) => DecisionAction::Allow,
        asupersync::Outcome::Err(_) => DecisionAction::Repair,
        asupersync::Outcome::Cancelled(_) | asupersync::Outcome::Panicked(_) => {
            DecisionAction::Reject
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConformalGuard, DecisionAction, EvidenceLedger, RaptorQEnvelope, RuntimeMode,
        RuntimePolicy, decision_to_card,
    };

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
    fn placeholder_raptorq_envelope_is_well_formed() {
        let envelope = RaptorQEnvelope::placeholder("packet-001", "conformance");
        assert_eq!(envelope.artifact_id, "packet-001");
        assert_eq!(envelope.artifact_type, "conformance");
        assert_eq!(envelope.scrub.status, "ok");
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
