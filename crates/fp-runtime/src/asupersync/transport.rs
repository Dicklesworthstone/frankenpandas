use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};

use crate::asupersync::{
    codec::EncodedArtifact,
    config::{AsupersyncConfig, CapabilitySet, CxCapability},
    error::AsupersyncError,
    validate_capability_gate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Completed,
    RetryableFailure,
    PermanentFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferReport {
    pub artifact_id: String,
    pub bytes_transferred: usize,
    pub status: TransferStatus,
    pub detail: String,
}

pub trait TransportLayer {
    fn send(
        &self,
        artifact: EncodedArtifact,
        config: &AsupersyncConfig,
    ) -> Result<TransferReport, AsupersyncError>;

    fn receive(
        &self,
        artifact_id: &str,
        config: &AsupersyncConfig,
    ) -> Result<EncodedArtifact, AsupersyncError>;

    fn required_capabilities(&self) -> CapabilitySet {
        CapabilitySet::for_capability(CxCapability::Io)
            .union(CapabilitySet::for_capability(CxCapability::Remote))
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryTransport {
    storage: Arc<Mutex<BTreeMap<String, Arc<EncodedArtifact>>>>,
}

impl InMemoryTransport {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl TransportLayer for InMemoryTransport {
    fn send(
        &self,
        artifact: EncodedArtifact,
        config: &AsupersyncConfig,
    ) -> Result<TransferReport, AsupersyncError> {
        validate_capability_gate(config, self.required_capabilities())?;

        let mut guard = self.storage.lock().map_err(|_| {
            AsupersyncError::Transport("in-memory transport lock poisoned".to_string())
        })?;
        let bytes_transferred = artifact.encoded_bytes.len();
        let artifact_id = artifact.artifact_id.clone();
        guard.insert(artifact_id.clone(), Arc::new(artifact));

        Ok(TransferReport {
            artifact_id,
            bytes_transferred,
            status: TransferStatus::Completed,
            detail: "stored in in-memory transport".to_string(),
        })
    }

    fn receive(
        &self,
        artifact_id: &str,
        config: &AsupersyncConfig,
    ) -> Result<EncodedArtifact, AsupersyncError> {
        validate_capability_gate(config, self.required_capabilities())?;

        let artifact = {
            let guard = self.storage.lock().map_err(|_| {
                AsupersyncError::Transport("in-memory transport lock poisoned".to_string())
            })?;
            guard
                .get(artifact_id)
                .cloned()
                .ok_or_else(|| AsupersyncError::ArtifactNotFound(artifact_id.to_string()))?
        };

        match std::panic::catch_unwind(|| (*artifact).clone()) {
            Ok(artifact) => Ok(artifact),
            Err(payload) => {
                let _guard = self.storage.lock().map_err(|_| {
                    AsupersyncError::Transport("in-memory transport lock poisoned".to_string())
                })?;
                std::panic::resume_unwind(payload);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use super::{InMemoryTransport, TransportLayer};
    use crate::asupersync::{
        codec::EncodedArtifact, config::AsupersyncConfig, error::AsupersyncError,
    };

    fn artifact() -> EncodedArtifact {
        EncodedArtifact {
            artifact_id: "transport-artifact-3nzz3".to_string(),
            source_len: 7,
            encoded_bytes: b"payload".to_vec(),
            repair_symbols: 1,
        }
    }

    #[test]
    fn receive_returns_a_deep_copy_after_storing_an_arc_3nzz3() -> Result<(), AsupersyncError> {
        let transport = InMemoryTransport::new();
        let config = AsupersyncConfig::default();
        transport.send(artifact(), &config)?;

        let mut received = transport.receive("transport-artifact-3nzz3", &config)?;
        received.encoded_bytes = b"changed".to_vec();

        assert_eq!(
            transport
                .receive("transport-artifact-3nzz3", &config)?
                .encoded_bytes,
            b"payload"
        );
        Ok(())
    }

    #[test]
    fn receive_preserves_lock_poisoning_failure_3nzz3() {
        let transport = InMemoryTransport::new();
        let storage = transport.storage.clone();
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = storage
                .lock()
                .unwrap_or_else(|error| std::panic::resume_unwind(Box::new(error)));
            std::panic::resume_unwind(Box::new("poison in-memory transport lock"));
        }));

        let error = transport
            .receive("transport-artifact-3nzz3", &AsupersyncConfig::default())
            .expect_err("poisoned storage must be rejected");
        assert!(
            matches!(error, AsupersyncError::Transport(message) if message.contains("poisoned"))
        );
    }
}
