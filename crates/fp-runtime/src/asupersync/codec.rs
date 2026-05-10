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
        if encoded.repair_symbols == 0 {
            return Err(AsupersyncError::Codec(
                "repair_symbols must be greater than zero".to_string(),
            ));
        }
        if encoded.source_len > encoded.encoded_bytes.len() {
            return Err(AsupersyncError::Codec(
                "source_len exceeds encoded payload length".to_string(),
            ));
        }
        let bytes = encoded
            .encoded_bytes
            .get(..encoded.source_len)
            .ok_or_else(|| {
                AsupersyncError::Codec("source_len exceeds encoded payload length".to_string())
            })?
            .to_vec();

        Ok(ArtifactPayload {
            artifact_id: encoded.artifact_id.clone(),
            bytes,
            expected_digest: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtifactCodec, ArtifactPayload, EncodedArtifact, PassthroughCodec};
    use crate::asupersync::{config::AsupersyncConfig, error::AsupersyncError};

    #[test]
    fn passthrough_codec_rejects_zero_repair_symbols_on_encode_20jod() {
        let codec = PassthroughCodec;
        let config = AsupersyncConfig {
            max_repair_symbols: 0,
            ..AsupersyncConfig::default()
        };
        let payload = ArtifactPayload {
            artifact_id: "artifact-20jod".to_string(),
            bytes: b"payload".to_vec(),
            expected_digest: None,
        };

        let err = codec.encode(&payload, &config).err();
        assert!(matches!(err, Some(AsupersyncError::Configuration(_))));
    }

    #[test]
    fn passthrough_codec_rejects_zero_repair_symbols_on_decode_20jod() {
        let codec = PassthroughCodec;
        let config = AsupersyncConfig::default();
        let encoded = EncodedArtifact {
            artifact_id: "artifact-20jod".to_string(),
            source_len: 7,
            encoded_bytes: b"payload".to_vec(),
            repair_symbols: 0,
        };

        let err = codec.decode(&encoded, &config).err();
        assert!(
            matches!(err, Some(AsupersyncError::Codec(message)) if message.contains("repair_symbols"))
        );
    }

    #[test]
    fn passthrough_codec_round_trip_preserves_payload_with_manifest_20jod()
    -> Result<(), AsupersyncError> {
        let codec = PassthroughCodec;
        let config = AsupersyncConfig {
            max_repair_symbols: 4,
            ..AsupersyncConfig::default()
        };
        let payload = ArtifactPayload {
            artifact_id: "artifact-20jod".to_string(),
            bytes: b"payload".to_vec(),
            expected_digest: Some("digest".to_string()),
        };

        let encoded = codec.encode(&payload, &config)?;
        assert_eq!(encoded.repair_symbols, 4);

        let decoded = codec.decode(&encoded, &config)?;
        assert_eq!(decoded.artifact_id, payload.artifact_id);
        assert_eq!(decoded.bytes, payload.bytes);
        assert_eq!(decoded.expected_digest, None);
        Ok(())
    }
}
