use std::time::{SystemTime, UNIX_EPOCH};

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

fn current_unix_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    u64::try_from(millis).unwrap_or(u64::MAX)
}

fn recovery_deadline_expired(deadline_unix_ms: u64) -> bool {
    deadline_unix_ms != 0 && current_unix_ms() > deadline_unix_ms
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

    // Per br-frankenpandas-bc6fa4: enforce a hard ceiling at max_attempts even
    // if a buggy RecoveryPolicy returns should_retry=true past the limit.
    // saturating_add prevents u32 overflow panics in debug; the explicit
    // attempts >= max_attempts gate makes the contract independent of the
    // policy implementation.
    let mut attempts = 0_u32;
    let should_retry = |attempt: u32| {
        attempt < plan.max_attempts && policy.should_retry(attempt, plan.max_attempts)
    };
    loop {
        if recovery_deadline_expired(plan.deadline_unix_ms) {
            return Err(AsupersyncError::RecoveryExhausted {
                artifact_id: plan.artifact_id.clone(),
                attempts,
            });
        }

        attempts = attempts.saturating_add(1);

        let encoded = match transport.receive(&plan.artifact_id, config) {
            Ok(encoded) => encoded,
            Err(_) => {
                if should_retry(attempts) {
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
                if should_retry(attempts) {
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
                if should_retry(attempts) {
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

#[cfg(test)]
mod test_recover_once_bounded_loop_bc6fa4 {
    use std::cell::Cell;

    use super::*;
    use crate::asupersync::{
        codec::{ArtifactPayload, EncodedArtifact, PassthroughCodec},
        config::{AsupersyncConfig, CapabilitySet, CxCapability},
        integrity::{Fnv1aVerifier, IntegrityProof},
        transport::InMemoryTransport,
    };

    /// A buggy policy that always says "retry" — without the hard ceiling fix
    /// in br-bc6fa4, recover_once would loop forever and eventually overflow
    /// attempts: u32 in release builds (or panic in debug).
    struct AlwaysRetryPolicy;

    impl RecoveryPolicy for AlwaysRetryPolicy {
        fn should_retry(&self, _attempt: u32, _max_attempts: u32) -> bool {
            true
        }
    }

    /// A transport that always fails so the policy gets a chance to keep retrying.
    struct AlwaysFailingTransport;

    impl TransportLayer for AlwaysFailingTransport {
        fn send(
            &self,
            _artifact: EncodedArtifact,
            _config: &AsupersyncConfig,
        ) -> Result<crate::asupersync::transport::TransferReport, AsupersyncError> {
            Err(AsupersyncError::Transport(
                "always-failing send".to_string(),
            ))
        }
        fn receive(
            &self,
            _artifact_id: &str,
            _config: &AsupersyncConfig,
        ) -> Result<EncodedArtifact, AsupersyncError> {
            Err(AsupersyncError::Transport(
                "always-failing receive".to_string(),
            ))
        }
        fn required_capabilities(&self) -> CapabilitySet {
            CapabilitySet::for_capability(CxCapability::Io)
        }
    }

    struct CountingFailingTransport {
        receive_calls: Cell<u32>,
    }

    impl CountingFailingTransport {
        fn new() -> Self {
            Self {
                receive_calls: Cell::new(0),
            }
        }
    }

    impl TransportLayer for CountingFailingTransport {
        fn send(
            &self,
            _artifact: EncodedArtifact,
            _config: &AsupersyncConfig,
        ) -> Result<crate::asupersync::transport::TransferReport, AsupersyncError> {
            Err(AsupersyncError::Transport(
                "counting-failing send".to_string(),
            ))
        }
        fn receive(
            &self,
            _artifact_id: &str,
            _config: &AsupersyncConfig,
        ) -> Result<EncodedArtifact, AsupersyncError> {
            self.receive_calls
                .set(self.receive_calls.get().saturating_add(1));
            Err(AsupersyncError::Transport(
                "counting-failing receive".to_string(),
            ))
        }
        fn required_capabilities(&self) -> CapabilitySet {
            CapabilitySet::for_capability(CxCapability::Io)
        }
    }

    struct FailingVerifier;

    impl IntegrityVerifier for FailingVerifier {
        fn verify(
            &self,
            _artifact_id: &str,
            _bytes: &[u8],
            _expected_digest: &str,
        ) -> Result<IntegrityProof, AsupersyncError> {
            Err(AsupersyncError::IntegrityMismatch {
                artifact_id: "failing-verifier".to_string(),
                expected: "x".to_string(),
                observed: "y".to_string(),
            })
        }
    }

    fn fnv1a_hex_for_test(bytes: &[u8]) -> String {
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }

    #[test]
    fn recover_once_terminates_at_max_attempts_under_buggy_policy() {
        // Without the hard ceiling fix, this test would never return.
        let plan = RecoveryPlan {
            artifact_id: "test-artifact".to_string(),
            max_attempts: 5,
            deadline_unix_ms: 0,
        };
        let config = AsupersyncConfig::default()
            .with_capabilities(CapabilitySet::for_capability(CxCapability::Io));
        let result = recover_once(
            &PassthroughCodec,
            &AlwaysFailingTransport,
            &FailingVerifier,
            &AlwaysRetryPolicy,
            &config,
            &plan,
            "any-digest",
        );
        assert!(
            matches!(
                &result,
                Err(AsupersyncError::RecoveryExhausted {
                    artifact_id,
                    attempts,
                }) if artifact_id == "test-artifact" && *attempts == 5
            ),
            "expected RecoveryExhausted after exactly 5 attempts, got {result:?}"
        );
    }

    #[test]
    fn recover_once_zero_max_attempts_returns_configuration_error() {
        let plan = RecoveryPlan {
            artifact_id: "test".to_string(),
            max_attempts: 0,
            deadline_unix_ms: 0,
        };
        let config = AsupersyncConfig::default()
            .with_capabilities(CapabilitySet::for_capability(CxCapability::Io));
        let result = recover_once(
            &PassthroughCodec,
            &AlwaysFailingTransport,
            &FailingVerifier,
            &AlwaysRetryPolicy,
            &config,
            &plan,
            "x",
        );
        assert!(matches!(result, Err(AsupersyncError::Configuration(_))));
    }

    #[test]
    fn recover_once_expired_deadline_makes_zero_transport_attempts_ehn2c() {
        let transport = CountingFailingTransport::new();
        let plan = RecoveryPlan {
            artifact_id: "expired-artifact".to_string(),
            max_attempts: 3,
            deadline_unix_ms: 1,
        };
        let config = AsupersyncConfig::default()
            .with_capabilities(CapabilitySet::for_capability(CxCapability::Io));

        let result = recover_once(
            &PassthroughCodec,
            &transport,
            &FailingVerifier,
            &AlwaysRetryPolicy,
            &config,
            &plan,
            "any-digest",
        );

        assert!(
            matches!(
                &result,
                Err(AsupersyncError::RecoveryExhausted {
                    artifact_id,
                    attempts,
                }) if artifact_id == "expired-artifact" && *attempts == 0
            ),
            "expected deadline RecoveryExhausted, got {result:?}"
        );
        assert_eq!(
            transport.receive_calls.get(),
            0,
            "expired recovery plans must fail before transport.receive"
        );
    }

    #[test]
    fn recover_once_future_deadline_preserves_retry_budget_ehn2c() {
        let transport = CountingFailingTransport::new();
        let plan = RecoveryPlan {
            artifact_id: "future-artifact".to_string(),
            max_attempts: 2,
            deadline_unix_ms: current_unix_ms().saturating_add(60_000),
        };
        let config = AsupersyncConfig::default()
            .with_capabilities(CapabilitySet::for_capability(CxCapability::Io));

        let result = recover_once(
            &PassthroughCodec,
            &transport,
            &FailingVerifier,
            &AlwaysRetryPolicy,
            &config,
            &plan,
            "any-digest",
        );

        assert!(
            matches!(
                &result,
                Err(AsupersyncError::RecoveryExhausted {
                    artifact_id,
                    attempts,
                }) if artifact_id == "future-artifact" && *attempts == 2
            ),
            "expected retry-budget RecoveryExhausted, got {result:?}"
        );
        assert_eq!(transport.receive_calls.get(), 2);
    }

    #[test]
    fn recover_once_round_trips_real_codec_transport_and_verifier_2ryvf()
    -> Result<(), AsupersyncError> {
        let codec = PassthroughCodec;
        let transport = InMemoryTransport::new();
        let verifier = Fnv1aVerifier;
        let config = AsupersyncConfig::default().with_capabilities(
            CapabilitySet::for_capability(CxCapability::Io)
                .union(CapabilitySet::for_capability(CxCapability::Remote)),
        );
        let bytes = b"recoverable artifact payload".to_vec();
        let expected_digest = fnv1a_hex_for_test(&bytes);
        let payload = ArtifactPayload {
            artifact_id: "recoverable-artifact".to_string(),
            bytes,
            expected_digest: Some(expected_digest.clone()),
        };
        let encoded = codec.encode(&payload, &config)?;
        transport.send(encoded, &config)?;

        let plan = RecoveryPlan {
            artifact_id: payload.artifact_id.clone(),
            max_attempts: 3,
            deadline_unix_ms: 0,
        };
        let report = recover_once(
            &codec,
            &transport,
            &verifier,
            &ConservativeRecoveryPolicy,
            &config,
            &plan,
            &expected_digest,
        )?;

        assert_eq!(report.artifact_id, "recoverable-artifact");
        assert_eq!(report.attempts, 1);
        assert_eq!(report.outcome, RecoveryOutcome::Recovered);
        assert_eq!(report.transfer_status, TransferStatus::Completed);
        let Some(proof) = report.integrity else {
            return Err(AsupersyncError::IntegrityMismatch {
                artifact_id: "recoverable-artifact".to_string(),
                expected: expected_digest,
                observed: "<missing proof>".to_string(),
            });
        };
        assert!(proof.verified);
        assert_eq!(proof.algorithm, "fnv1a64");
        assert_eq!(proof.expected_digest, proof.observed_digest);
        Ok(())
    }
}
