use serde::{Deserialize, Serialize};

use crate::asupersync::{config::AsupersyncConfig, error::AsupersyncError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPayload {
    pub artifact_id: String,
    pub bytes: Vec<u8>,
    pub expected_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedArtifact {
    pub artifact_id: String,
    pub source_len: usize,
    pub encoded_bytes: Vec<u8>,
    pub repair_symbols: u32,
}

pub trait ArtifactCodec {
    fn encode(
        &self,
        payload: &ArtifactPayload,
        config: &AsupersyncConfig,
    ) -> Result<EncodedArtifact, AsupersyncError>;

    fn decode(
        &self,
        encoded: &EncodedArtifact,
        config: &AsupersyncConfig,
    ) -> Result<ArtifactPayload, AsupersyncError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PassthroughCodec;

impl ArtifactCodec for PassthroughCodec {
    fn encode(
        &self,
        payload: &ArtifactPayload,
        config: &AsupersyncConfig,
    ) -> Result<EncodedArtifact, AsupersyncError> {
        if config.max_repair_symbols == 0 {
            return Err(AsupersyncError::Configuration(
                "max_repair_symbols must be greater than zero",
            ));
        }

        Ok(EncodedArtifact {
            artifact_id: payload.artifact_id.clone(),
            source_len: payload.bytes.len(),
            encoded_bytes: payload.bytes.clone(),
            repair_symbols: config.max_repair_symbols,
        })
    }

    fn decode(
        &self,
        encoded: &EncodedArtifact,
        _config: &AsupersyncConfig,
    ) -> Result<ArtifactPayload, AsupersyncError> {
        if encoded.source_len > encoded.encoded_bytes.len() {
            return Err(AsupersyncError::Codec(
                "source_len exceeds encoded payload length".to_string(),
            ));
        }

        Ok(ArtifactPayload {
            artifact_id: encoded.artifact_id.clone(),
            bytes: encoded.encoded_bytes[..encoded.source_len].to_vec(),
            expected_digest: None,
        })
    }
}
