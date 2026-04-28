use self::{
    config::{AsupersyncConfig, CapabilitySet},
    error::AsupersyncError,
};

pub mod codec;
pub mod config;
pub mod error;
pub mod integrity;
pub mod recovery;
pub mod transport;

pub use codec::{ArtifactCodec, ArtifactPayload, EncodedArtifact, PassthroughCodec};
pub use config::{AsupersyncConfig as RuntimeAsupersyncConfig, CxCapability, RequiresCapabilities};
pub use error::AsupersyncError as RuntimeAsupersyncError;
pub use integrity::{Fnv1aVerifier, IntegrityProof, IntegrityVerifier};
pub use recovery::{
    ConservativeRecoveryPolicy, RecoveryOutcome, RecoveryPlan, RecoveryPolicy, RecoveryReport,
    recover_once,
};
pub use transport::{InMemoryTransport, TransferReport, TransferStatus, TransportLayer};

pub fn validate_capability_gate(
    config: &AsupersyncConfig,
    required: CapabilitySet,
) -> Result<(), AsupersyncError> {
    if config.capabilities.satisfies(required) {
        Ok(())
    } else {
        Err(AsupersyncError::CapabilityDenied {
            required,
            available: config.capabilities,
        })
    }
}
