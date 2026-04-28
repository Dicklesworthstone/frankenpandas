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
    storage: Arc<Mutex<BTreeMap<String, EncodedArtifact>>>,
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
        guard.insert(artifact_id.clone(), artifact);

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

        let guard = self.storage.lock().map_err(|_| {
            AsupersyncError::Transport("in-memory transport lock poisoned".to_string())
        })?;
        guard
            .get(artifact_id)
            .cloned()
            .ok_or_else(|| AsupersyncError::ArtifactNotFound(artifact_id.to_string()))
    }
}
