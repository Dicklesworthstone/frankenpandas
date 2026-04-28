use serde::{Deserialize, Serialize};

use crate::asupersync::{
    codec::ArtifactCodec,
    config::AsupersyncConfig,
    error::AsupersyncError,
    integrity::{IntegrityProof, IntegrityVerifier},
    transport::{TransferStatus, TransportLayer},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryOutcome {
    Recovered,
    RetryScheduled,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPlan {
    pub artifact_id: String,
    pub max_attempts: u32,
    pub deadline_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub artifact_id: String,
    pub attempts: u32,
    pub outcome: RecoveryOutcome,
    pub transfer_status: TransferStatus,
    pub integrity: Option<IntegrityProof>,
}

pub trait RecoveryPolicy {
    fn should_retry(&self, attempt: u32, max_attempts: u32) -> bool;

    fn classify(
        &self,
        transfer_status: TransferStatus,
        attempt: u32,
        max_attempts: u32,
    ) -> RecoveryOutcome {
        match transfer_status {
            TransferStatus::Completed => RecoveryOutcome::Recovered,
            TransferStatus::RetryableFailure => {
                if self.should_retry(attempt, max_attempts) {
                    RecoveryOutcome::RetryScheduled
                } else {
                    RecoveryOutcome::Rejected
                }
            }
            TransferStatus::PermanentFailure => RecoveryOutcome::Rejected,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ConservativeRecoveryPolicy;

impl RecoveryPolicy for ConservativeRecoveryPolicy {
    fn should_retry(&self, attempt: u32, max_attempts: u32) -> bool {
        attempt < max_attempts
    }
}

pub fn recover_once<C, T, V, P>(
    codec: &C,
    transport: &T,
    verifier: &V,
    policy: &P,
    config: &AsupersyncConfig,
    plan: &RecoveryPlan,
    expected_digest: &str,
) -> Result<RecoveryReport, AsupersyncError>
where
    C: ArtifactCodec,
    T: TransportLayer,
    V: IntegrityVerifier,
    P: RecoveryPolicy,
{
    if plan.max_attempts == 0 {
        return Err(AsupersyncError::Configuration(
            "max_attempts must be greater than zero",
        ));
    }

    let mut attempts = 0_u32;
    loop {
        attempts += 1;

        let encoded = match transport.receive(&plan.artifact_id, config) {
            Ok(encoded) => encoded,
            Err(_) => {
                if policy.should_retry(attempts, plan.max_attempts) {
                    continue;
                }
                return Err(AsupersyncError::RecoveryExhausted {
                    artifact_id: plan.artifact_id.clone(),
                    attempts,
                });
            }
        };

        let payload = match codec.decode(&encoded, config) {
            Ok(p) => p,
            Err(_) => {
                if policy.should_retry(attempts, plan.max_attempts) {
                    continue;
                }
                return Err(AsupersyncError::RecoveryExhausted {
                    artifact_id: plan.artifact_id.clone(),
                    attempts,
                });
            }
        };
        match verifier.verify(&plan.artifact_id, &payload.bytes, expected_digest) {
            Ok(integrity) => {
                return Ok(RecoveryReport {
                    artifact_id: plan.artifact_id.clone(),
                    attempts,
                    outcome: RecoveryOutcome::Recovered,
                    transfer_status: TransferStatus::Completed,
                    integrity: Some(integrity),
                });
            }
            Err(_) => {
                if policy.should_retry(attempts, plan.max_attempts) {
                    continue;
                }
                return Err(AsupersyncError::RecoveryExhausted {
                    artifact_id: plan.artifact_id.clone(),
                    attempts,
                });
            }
        }
    }
}
