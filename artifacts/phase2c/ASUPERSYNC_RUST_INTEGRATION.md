# ASUPERSYNC Rust Integration Plan + Module Boundary Skeleton

Bead: `bd-2gi.27.4` [ASUPERSYNC-D]
Subsystem: `fp-runtime` asupersync module decomposition, trait design, and implementation sequencing
Source anchors:
- `ASUPERSYNC_ANCHOR_MAP.md` (bd-2gi.27.1)
- `ASUPERSYNC_CONTRACT_TABLE.md` (bd-2gi.27.2)
- `ASUPERSYNC_THREAT_MODEL.md` (bd-2gi.27.3)

---

## 1. Summary

This document translates the behavioral contracts, threat model, and anchor map
produced by ASUPERSYNC beads A/B/C into concrete Rust module boundaries, trait
definitions, type skeletons, and a phased implementation sequence.

**Scope:** The ASUPERSYNC subsystem provides artifact replication and recovery
via RaptorQ erasure coding, an async transport abstraction, integrity
verification with SHA-256 sidecar validation, and a recovery workflow with
configurable retry policies. All of this integrates into the `fp-runtime` crate
under the existing `asupersync` feature flag, interacting with the `Cx`
capability context for authority control and the conformance harness for
end-to-end verification.

**Goals:**
1. Define module boundaries that enforce separation of concerns (codec, transport,
   integrity, recovery, config) with minimal cross-module coupling.
2. Specify traits with full Rust signatures so that implementors have a clear
   contract and test harnesses can substitute mock implementations.
3. Sequence implementation phases so that each phase produces testable artifacts
   and no phase depends on unfinished work from a later phase.
4. Remediate the INV-NO-AMBIENT-AUTHORITY violation (ATS-3.1) as the first
   concrete code change.

**Non-goals:** This document does not implement any code. It does not specify
FTUI dashboard integration (deferred to FrankenTUI beads). It does not design
the full Cx capability threading through the FrankenPandas call stack (deferred
to a separate Cx integration bead).

---

## 2. Current Integration State

### 2.1 What Exists in fp-runtime

The `fp-runtime` crate (`crates/fp-runtime/src/lib.rs`, 919 lines) contains:

| Component | Lines | Status |
|---|---|---|
| `RuntimeMode`, `DecisionAction`, `IssueKind` enums | 1-30 | Stable |
| `CompatibilityIssue`, `EvidenceTerm`, `LossMatrix` types | 32-67 | Stable |
| `DecisionMetrics`, `DecisionRecord` types | 69-87 | Stable |
| `GalaxyBrainCard` + `decision_to_card()` | 89-121 | Stable |
| `EvidenceLedger` (append-only) | 123-144 | Stable |
| `RuntimePolicy` (strict/hardened) | 146-256 | Stable |
| `RuntimeError::ClockSkew` + `now_unix_ms()` | 258-270 | **Violation site** (ATS-3.1) |
| `decide()` (Bayesian engine) | 272-322 | Stable (calls `now_unix_ms`) |
| `RaptorQEnvelope`, `RaptorQMetadata`, `ScrubStatus`, `DecodeProof` | 324-376 | Placeholder-only |
| `ConformalGuard` + `ConformalPredictionSet` | 378-551 | Stable |
| `outcome_to_action()` (feature-gated) | 553-563 | Stable, gated on `asupersync` |
| Tests (18 test functions) | 565-919 | Passing |

### 2.2 Feature Flag Status

```toml
# crates/fp-runtime/Cargo.toml
[features]
default = []
asupersync = ["dep:asupersync"]

[dependencies]
asupersync = { version = "0.1.1", optional = true, default-features = false }
```

- The `asupersync` feature is **default-off**. No other workspace crate enables it.
- When enabled, only `outcome_to_action()` is compiled (6 lines of feature-gated code).
- The RaptorQ envelope types are **not** feature-gated; they are structural placeholders
  available unconditionally.

### 2.3 fp-conformance Integration

The `fp-conformance` crate already performs real RaptorQ encode/decode using the
`raptorq` crate (not `asupersync`). It defines:

- `RaptorQSidecarArtifact` -- wraps `RaptorQEnvelope` with OTI data and packet records
- `RaptorQPacketRecord` -- individual encoded symbol metadata
- `RaptorQScrubReport` -- integrity verification result
- `generate_raptorq_sidecar()` -- encodes payloads with repair symbols
- `run_raptorq_decode_recovery_drill()` -- drops source packets and recovers via repair
- `verify_raptorq_sidecar()` -- validates packet hashes against source data

This conformance-layer RaptorQ usage is the primary consumer of the envelope types
and will need to integrate with the new ASUPERSYNC modules.

### 2.4 Current Test Coverage

| Area | Tests | Notes |
|---|---|---|
| Outcome bridge | 0 (feature off by default) | Would need `--features asupersync` |
| Decision engine (strict) | 1 | `strict_mode_fails_closed_for_unknown_features` |
| Decision engine (hardened) | 1 | `hardened_mode_repairs_large_join_estimates` |
| RaptorQ placeholder | 1 | `placeholder_raptorq_envelope_is_well_formed` |
| Conformal guard | 11 | AG-09-T suite, comprehensive |
| Galaxy brain card | 2 | Rendering + content checks |
| RaptorQ real encode/decode | 3 (in fp-conformance) | Sidecar generation, drill, scrub |

**Gap:** No tests exercise `now_unix_ms()` clock failure, no tests exercise the
outcome bridge (requires feature flag), and no integration tests cross
fp-runtime and fp-conformance for ASUPERSYNC workflows.

---

## 3. Module Boundary Design

### 3.1 Module Hierarchy

All new modules reside under `crates/fp-runtime/src/asupersync/`. A new
`mod asupersync` is added to `lib.rs`, gated on `#[cfg(feature = "asupersync")]`
at the module level (not per-item). Types that must remain available without
the feature flag (e.g., `RaptorQEnvelope`) stay in `lib.rs`.

```
crates/fp-runtime/src/
  lib.rs                     # existing: public API, re-exports
  clock.rs                   # NEW: Clock trait (NOT feature-gated)
  asupersync/                # NEW: feature-gated module tree
    mod.rs                   # re-exports, module-level docs
    codec.rs                 # RaptorQ encode/decode with fallback
    transport.rs             # async send/receive abstraction
    integrity.rs             # SHA-256 verification, sidecar validation
    recovery.rs              # recovery workflow, retry logic
    config.rs                # configuration types, capability reqs
    error.rs                 # AsupersyncError enum
```

**Rationale for `clock.rs` outside feature gate:** The Clock trait is needed to
remediate INV-NO-AMBIENT-AUTHORITY regardless of whether asupersync is enabled.
The decision engine must accept an injectable clock even in the base configuration.

### 3.2 Module Responsibilities

#### `clock.rs` (unconditional)

| Property | Value |
|---|---|
| Responsibility | Injectable time source replacing ambient `SystemTime::now()` |
| Public types | `Clock` (trait), `SystemClock` (struct), `FakeClock` (struct) |
| Internal types | None |
| Dependencies | `std::time::{SystemTime, UNIX_EPOCH}`, `std::sync::atomic` |
| Feature gate | None (always compiled) |

#### `asupersync::codec`

| Property | Value |
|---|---|
| Responsibility | RaptorQ encode/decode with configurable repair symbol ratio; fallback chain when asupersync crate unavailable |
| Public types | `ArtifactCodec` (trait), `RaptorQCodec` (struct), `CodecConfig` (struct), `EncodedArtifact` (struct) |
| Internal types | `SymbolBatch`, `BlockEncoder` (wrappers) |
| Dependencies | `asupersync::raptorq`, `asupersync::config::RaptorQConfig` |
| Feature gate | `#[cfg(feature = "asupersync")]` |

#### `asupersync::transport`

| Property | Value |
|---|---|
| Responsibility | Async artifact transfer abstraction; send/receive encoded symbol batches |
| Public types | `TransportLayer` (trait), `TransferReport` (struct), `TransferStatus` (enum) |
| Internal types | `SymbolSinkAdapter`, `SymbolStreamAdapter` |
| Dependencies | `asupersync::transport::{SymbolSink, SymbolStream}`, `asupersync::cx::Cx` |
| Feature gate | `#[cfg(feature = "asupersync")]` |

#### `asupersync::integrity`

| Property | Value |
|---|---|
| Responsibility | SHA-256 hash verification for artifact payloads; sidecar manifest validation; decode proof binding |
| Public types | `IntegrityVerifier` (trait), `Sha256Verifier` (struct), `SidecarManifest` (struct), `VerificationResult` (enum) |
| Internal types | `HashChain` |
| Dependencies | `sha2::Sha256` (already in workspace), `asupersync::security::SecurityContext` (optional) |
| Feature gate | `#[cfg(feature = "asupersync")]` |

#### `asupersync::recovery`

| Property | Value |
|---|---|
| Responsibility | Recovery workflow orchestration; retry logic with exponential backoff; supervision monotonicity enforcement |
| Public types | `RecoveryPolicy` (trait), `RecoveryPlan` (struct), `RecoveryOutcome` (enum), `DefaultRecoveryPolicy` (struct) |
| Internal types | `RetryState`, `BackoffCalculator` |
| Dependencies | `asupersync::Outcome`, codec + transport + integrity modules |
| Feature gate | `#[cfg(feature = "asupersync")]` |

#### `asupersync::config`

| Property | Value |
|---|---|
| Responsibility | Configuration types for all ASUPERSYNC modules; capability requirements per operation; feature sub-flags |
| Public types | `AsupersyncConfig` (struct), `CxCapability` (enum), `CapabilitySet` (struct), `OperationRequirements` (struct) |
| Internal types | None |
| Dependencies | `serde` |
| Feature gate | `#[cfg(feature = "asupersync")]` |

#### `asupersync::error`

| Property | Value |
|---|---|
| Responsibility | Unified error type for all ASUPERSYNC operations; conversion impls from io, RaptorQ, transport errors |
| Public types | `AsupersyncError` (enum) |
| Internal types | None |
| Dependencies | `std::io`, `thiserror` |
| Feature gate | `#[cfg(feature = "asupersync")]` |

---

## 4. Trait Definitions

### 4.1 `Clock` (unconditional, in `clock.rs`)

```rust
/// Injectable time source. Replaces ambient `SystemTime::now()` calls.
///
/// Remediates INV-NO-AMBIENT-AUTHORITY (ATS-3.1) by making time observation
/// an explicit capability that can be substituted in tests.
pub trait Clock: Send + Sync {
    /// Returns the current time as milliseconds since UNIX epoch.
    ///
    /// # Errors
    /// Returns `RuntimeError::ClockSkew` if the system clock is before UNIX epoch.
    fn now_unix_ms(&self) -> Result<u64, RuntimeError>;
}

/// Default clock using `SystemTime::now()`. For production use only.
/// Under lab-mode testing, use `FakeClock` instead.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_unix_ms(&self) -> Result<u64, RuntimeError> {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RuntimeError::ClockSkew)?
            .as_millis();
        Ok(ms as u64)
    }
}

/// Deterministic clock for testing. Returns a fixed or advancing timestamp.
#[derive(Debug, Clone)]
pub struct FakeClock {
    current_ms: std::sync::atomic::AtomicU64,
}

impl FakeClock {
    pub fn new(initial_ms: u64) -> Self {
        Self {
            current_ms: std::sync::atomic::AtomicU64::new(initial_ms),
        }
    }

    /// Advance the clock by the given number of milliseconds.
    pub fn advance(&self, ms: u64) {
        self.current_ms
            .fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now_unix_ms(&self) -> Result<u64, RuntimeError> {
        Ok(self.current_ms.load(std::sync::atomic::Ordering::SeqCst))
    }
}
```

### 4.2 `ArtifactCodec`

```rust
/// Encode and decode artifact payloads using erasure coding.
///
/// Implementations provide RaptorQ (or other FEC) encoding with configurable
/// redundancy. The codec operates on byte slices and produces `EncodedArtifact`
/// bundles containing source + repair symbols.
///
/// Contract: CON-CODEC-ROUNDTRIP -- for any payload P and codec C,
/// `C.decode(C.encode(P)?)` must return `Ok(P)` when all source symbols are
/// present, and must return `Ok(P)` when at least `k` of `k + repair` symbols
/// are available (where k = source symbol count).
pub trait ArtifactCodec: Send + Sync {
    /// Encode a payload into an `EncodedArtifact` with source and repair symbols.
    ///
    /// # Parameters
    /// - `artifact_id`: Unique identifier for the artifact being encoded.
    /// - `artifact_type`: Category string (e.g., "conformance", "benchmark").
    /// - `payload`: The raw bytes to encode.
    /// - `config`: Encoding configuration (repair ratio, symbol size, etc.).
    ///
    /// # Errors
    /// Returns `AsupersyncError::CodecEncode` on encoding failure.
    /// Returns `AsupersyncError::EmptyPayload` if payload is empty.
    fn encode(
        &self,
        artifact_id: &str,
        artifact_type: &str,
        payload: &[u8],
        config: &CodecConfig,
    ) -> Result<EncodedArtifact, AsupersyncError>;

    /// Decode an `EncodedArtifact` back to the original payload.
    ///
    /// # Parameters
    /// - `artifact`: The encoded artifact containing symbols and OTI.
    ///
    /// # Errors
    /// Returns `AsupersyncError::CodecDecode` if insufficient symbols remain.
    /// Returns `AsupersyncError::IntegrityFailure` if decoded payload hash
    /// does not match `artifact.source_hash`.
    fn decode(
        &self,
        artifact: &EncodedArtifact,
    ) -> Result<Vec<u8>, AsupersyncError>;

    /// Return the minimum number of symbols required for successful decode.
    fn min_symbols_for_decode(&self, artifact: &EncodedArtifact) -> u32;
}
```

### 4.3 `TransportLayer`

```rust
/// Async artifact transfer abstraction.
///
/// Implementations handle the network or IPC layer for sending/receiving
/// encoded artifacts. The trait is async to support non-blocking IO under
/// the asupersync runtime.
///
/// Contract: CON-TRANSPORT-DELIVERY -- a `send()` that returns `Ok(report)`
/// with `report.status == TransferStatus::Complete` guarantees that all
/// symbols were accepted by the remote receiver. The receiver may still
/// fail to decode if symbols are corrupted in transit (integrity check
/// is the caller's responsibility).
#[async_trait::async_trait]
pub trait TransportLayer: Send + Sync {
    /// Send an encoded artifact to one or more destinations.
    ///
    /// # Parameters
    /// - `cx`: Capability context for cancellation and budget enforcement.
    /// - `artifact`: The encoded artifact to transfer.
    /// - `destinations`: Target endpoint identifiers.
    ///
    /// # Errors
    /// Returns `AsupersyncError::TransportSend` on network failure.
    /// Returns `AsupersyncError::Cancelled` if `cx` cancellation fires.
    /// Returns `AsupersyncError::BudgetExhausted` if deadline/quota exceeded.
    async fn send(
        &self,
        artifact: &EncodedArtifact,
        destinations: &[String],
    ) -> Result<TransferReport, AsupersyncError>;

    /// Receive an encoded artifact from a source.
    ///
    /// # Parameters
    /// - `cx`: Capability context for cancellation and budget enforcement.
    /// - `artifact_id`: The identifier of the artifact to receive.
    /// - `source`: Source endpoint identifier.
    ///
    /// # Errors
    /// Returns `AsupersyncError::TransportReceive` on network failure.
    /// Returns `AsupersyncError::Cancelled` if `cx` cancellation fires.
    /// Returns `AsupersyncError::NotFound` if artifact does not exist at source.
    async fn receive(
        &self,
        artifact_id: &str,
        source: &str,
    ) -> Result<EncodedArtifact, AsupersyncError>;
}
```

### 4.4 `IntegrityVerifier`

```rust
/// Hash verification for artifact payloads and encoded symbols.
///
/// Implementations compute and verify SHA-256 (or other) hashes for:
/// 1. Source payload integrity (pre-encode / post-decode)
/// 2. Individual symbol integrity (per-packet hash in sidecar)
/// 3. Sidecar manifest consistency (all hashes match recorded values)
///
/// Contract: CON-INTEGRITY-DETERMINISTIC -- for any payload P,
/// `verify(P, hash(P))` must always return `VerificationResult::Valid`.
/// `verify(P, hash(Q))` where P != Q must always return
/// `VerificationResult::Invalid`.
pub trait IntegrityVerifier: Send + Sync {
    /// Compute the hash of a payload, returning a prefixed hash string
    /// (e.g., "sha256:abcdef...").
    fn hash_payload(&self, payload: &[u8]) -> String;

    /// Verify that a payload matches an expected hash string.
    fn verify_payload(
        &self,
        payload: &[u8],
        expected_hash: &str,
    ) -> VerificationResult;

    /// Verify all entries in a sidecar manifest against actual symbol data.
    ///
    /// # Errors
    /// Returns `AsupersyncError::IntegrityFailure` with details on first
    /// mismatched symbol. In strict mode, verification stops at first failure.
    /// In hardened mode, all symbols are checked and all failures reported.
    fn verify_sidecar(
        &self,
        manifest: &SidecarManifest,
        mode: crate::RuntimeMode,
    ) -> Result<VerificationResult, AsupersyncError>;
}
```

### 4.5 `RecoveryPolicy`

```rust
/// Strategy for artifact recovery when decode or transfer fails.
///
/// Implementations define retry logic, backoff schedules, and escalation
/// behavior. The policy respects INV-SUPERVISION-MONOTONE: panicked
/// operations are never retried.
///
/// Contract: CON-RECOVERY-MONOTONE -- if `should_retry()` is called with
/// `RecoveryOutcome::Panicked`, it must return `false`. This is enforced
/// by a debug assertion in the default implementation.
pub trait RecoveryPolicy: Send + Sync {
    /// Determine whether a failed operation should be retried.
    ///
    /// # Parameters
    /// - `outcome`: The outcome of the previous attempt.
    /// - `attempt`: The current attempt number (1-indexed).
    /// - `plan`: The recovery plan with budget and constraints.
    ///
    /// # Returns
    /// `true` if the operation should be retried; `false` to give up.
    fn should_retry(
        &self,
        outcome: &RecoveryOutcome,
        attempt: u32,
        plan: &RecoveryPlan,
    ) -> bool;

    /// Compute the delay before the next retry attempt.
    ///
    /// # Parameters
    /// - `attempt`: The current attempt number (1-indexed).
    /// - `plan`: The recovery plan with budget and constraints.
    ///
    /// # Returns
    /// Delay in milliseconds. Returns 0 for immediate retry.
    fn retry_delay_ms(
        &self,
        attempt: u32,
        plan: &RecoveryPlan,
    ) -> u64;

    /// Maximum number of retry attempts before escalation.
    fn max_attempts(&self, plan: &RecoveryPlan) -> u32;
}
```

---

## 5. Type Skeleton

### 5.1 Existing Types (Verification)

#### `RaptorQEnvelope` (VERIFIED CORRECT)

```rust
// crates/fp-runtime/src/lib.rs lines 324-376
// NO CHANGES NEEDED to struct fields.
// Additions needed: impl blocks for real encode integration (Phase 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaptorQEnvelope {
    pub artifact_id: String,
    pub artifact_type: String,
    pub source_hash: String,          // "sha256:..." or "blake3:placeholder"
    pub raptorq: RaptorQMetadata,
    pub scrub: ScrubStatus,
    pub decode_proofs: Vec<DecodeProof>,
}
```

**Verification notes:**
- The `source_hash` field uses prefix-based format discrimination (`"sha256:"` for
  real, `"blake3:placeholder"` for placeholder). This is correct per CON-PLACEHOLDER-SENTINEL.
- The `decode_proofs` vector is unbounded. Per R-07, a `MAX_DECODE_PROOFS` cap should
  be added in Phase 1.

#### `DecodeProof` (NEEDS EXTENSION)

```rust
// Current (lib.rs lines 348-354):
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecodeProof {
    pub ts_unix_ms: u64,
    pub reason: String,
    pub recovered_blocks: u32,
    pub proof_hash: String,
}

// Planned additions (Phase 1, per R-04):
// pub artifact_id: String,   // bind proof to specific artifact
// pub nonce: u64,            // prevent cross-artifact replay
```

### 5.2 New Types

#### `SidecarManifest`

```rust
/// Manifest of an encoded artifact's symbol inventory with per-symbol hashes.
///
/// Used by `IntegrityVerifier::verify_sidecar()` to validate that all symbols
/// in a transferred artifact match their recorded hashes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SidecarManifest {
    /// Artifact identifier this manifest belongs to.
    pub artifact_id: String,
    /// Artifact type category.
    pub artifact_type: String,
    /// SHA-256 hash of the original source payload (prefixed: "sha256:...").
    pub source_hash: String,
    /// Number of source symbols (k parameter).
    pub source_symbol_count: u32,
    /// Number of repair symbols.
    pub repair_symbol_count: u32,
    /// Per-symbol hash entries, in encoding order.
    pub symbol_hashes: Vec<SymbolHashEntry>,
    /// Object Transmission Information for RaptorQ decoder.
    pub oti_hex: String,
    /// Timestamp when manifest was generated.
    pub generated_at_unix_ms: u64,
    /// Nonce for replay prevention.
    pub nonce: u64,
}

/// Hash entry for a single encoded symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolHashEntry {
    /// Source block number.
    pub source_block_number: u8,
    /// Encoding Symbol ID within the block.
    pub encoding_symbol_id: u32,
    /// Whether this is a source (true) or repair (false) symbol.
    pub is_source: bool,
    /// SHA-256 hash of the serialized symbol data.
    pub symbol_hash: String,
}
```

#### `RecoveryPlan`

```rust
/// Configuration for a recovery workflow attempt.
///
/// Specifies retry budgets, backoff parameters, and escalation thresholds.
/// The plan is consumed by `RecoveryPolicy` implementations to make
/// retry/abandon decisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveryPlan {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Initial backoff delay in milliseconds.
    pub initial_backoff_ms: u64,
    /// Maximum backoff delay in milliseconds.
    pub max_backoff_ms: u64,
    /// Backoff multiplier (e.g., 2.0 for exponential).
    pub backoff_multiplier: f64,
    /// Wall-clock deadline in milliseconds since epoch (0 = no deadline).
    pub deadline_unix_ms: u64,
    /// Whether to attempt alternate sources on failure.
    pub try_alternate_sources: bool,
    /// Alternate source endpoints to try if primary fails.
    pub alternate_sources: Vec<String>,
    /// Runtime mode governing fail-closed behavior.
    pub mode: crate::RuntimeMode,
}

impl Default for RecoveryPlan {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 100,
            max_backoff_ms: 10_000,
            backoff_multiplier: 2.0,
            deadline_unix_ms: 0,
            try_alternate_sources: false,
            alternate_sources: Vec::new(),
            mode: crate::RuntimeMode::Strict,
        }
    }
}
```

#### `TransferReport`

```rust
/// Report of an artifact transfer operation.
///
/// Captures timing, symbol counts, and status for audit and telemetry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransferReport {
    /// Artifact identifier that was transferred.
    pub artifact_id: String,
    /// Transfer status.
    pub status: TransferStatus,
    /// Number of symbols sent or received.
    pub symbols_transferred: u32,
    /// Total bytes transferred (sum of serialized symbol sizes).
    pub bytes_transferred: u64,
    /// Wall-clock duration of the transfer in milliseconds.
    pub duration_ms: u64,
    /// Destination or source endpoint.
    pub endpoint: String,
    /// Number of retries that occurred during transfer.
    pub retry_count: u32,
    /// Timestamp when transfer started.
    pub started_at_unix_ms: u64,
    /// Timestamp when transfer completed.
    pub completed_at_unix_ms: u64,
}

/// Status of an artifact transfer operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    /// All symbols transferred successfully.
    Complete,
    /// Transfer partially completed (some symbols missing).
    Partial,
    /// Transfer failed entirely.
    Failed,
    /// Transfer was cancelled via Cx cancellation.
    Cancelled,
}
```

#### `EncodedArtifact`

```rust
/// A fully encoded artifact ready for transport or storage.
///
/// Contains the RaptorQ envelope metadata, the OTI for decoder
/// initialization, and the actual encoded symbol data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncodedArtifact {
    /// Envelope metadata (artifact ID, type, hash, RaptorQ params).
    pub envelope: crate::RaptorQEnvelope,
    /// Object Transmission Information (hex-encoded 12-byte OTI).
    pub oti_hex: String,
    /// Encoded symbol packets (source + repair).
    pub symbols: Vec<EncodedSymbol>,
    /// Sidecar manifest with per-symbol hashes.
    pub manifest: SidecarManifest,
}

/// A single encoded symbol packet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncodedSymbol {
    /// Source block number.
    pub source_block_number: u8,
    /// Encoding Symbol ID.
    pub encoding_symbol_id: u32,
    /// Whether this is a source (true) or repair (false) symbol.
    pub is_source: bool,
    /// Hex-encoded serialized symbol data.
    pub data_hex: String,
    /// SHA-256 hash of the serialized data.
    pub hash: String,
}
```

#### `CxCapability` (enumeration of required capabilities)

```rust
/// Capability flags required by ASUPERSYNC operations.
///
/// Maps to the asupersync Cx phantom capability system:
/// `[SPAWN, TIME, RANDOM, IO, REMOTE]`.
///
/// Each ASUPERSYNC operation declares which capabilities it requires.
/// Fail-closed: if a required capability is missing from the Cx context,
/// the operation returns `AsupersyncError::CapabilityMissing`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CxCapability {
    /// Permission to observe wall-clock or virtual time.
    Time,
    /// Permission to perform IO (filesystem, network).
    Io,
    /// Permission to spawn new tasks.
    Spawn,
    /// Permission to use randomness sources.
    Random,
    /// Permission to perform remote operations (network transfer).
    Remote,
}

/// A set of capabilities required or available for an operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub capabilities: std::collections::BTreeSet<CxCapability>,
}

impl CapabilitySet {
    /// Check whether this set contains all capabilities in `required`.
    pub fn satisfies(&self, required: &CapabilitySet) -> bool {
        required.capabilities.is_subset(&self.capabilities)
    }

    /// Return the missing capabilities (in `required` but not in `self`).
    pub fn missing(&self, required: &CapabilitySet) -> Vec<CxCapability> {
        required.capabilities
            .difference(&self.capabilities)
            .copied()
            .collect()
    }
}

/// Declares which capabilities an ASUPERSYNC operation requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRequirements {
    /// Human-readable operation name (e.g., "codec.encode", "transport.send").
    pub operation: String,
    /// Required capabilities.
    pub required: CapabilitySet,
}
```

#### `CodecConfig`

```rust
/// Configuration for RaptorQ encoding operations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodecConfig {
    /// Maximum Transfer Unit (symbol payload size in bytes).
    pub symbol_size: u16,
    /// Number of repair symbols per source block.
    pub repair_symbols_per_block: u32,
    /// Whether to compute and store per-symbol hashes.
    pub compute_symbol_hashes: bool,
    /// Hash algorithm for source payload (currently always SHA-256).
    pub hash_algorithm: String,
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            symbol_size: 1400,
            repair_symbols_per_block: 8,
            compute_symbol_hashes: true,
            hash_algorithm: "sha256".to_owned(),
        }
    }
}
```

#### `AsupersyncConfig`

```rust
/// Top-level configuration for all ASUPERSYNC modules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AsupersyncConfig {
    /// Codec configuration.
    pub codec: CodecConfig,
    /// Default recovery plan for failed operations.
    pub default_recovery: RecoveryPlan,
    /// Maximum number of decode proofs stored per envelope.
    pub max_decode_proofs: usize,
    /// Whether to require authenticated symbols (SecurityContext integration).
    pub require_authenticated_symbols: bool,
    /// Runtime mode (strict/hardened) for integrity and recovery decisions.
    pub mode: crate::RuntimeMode,
}

impl Default for AsupersyncConfig {
    fn default() -> Self {
        Self {
            codec: CodecConfig::default(),
            default_recovery: RecoveryPlan::default(),
            max_decode_proofs: 1000,
            require_authenticated_symbols: false,
            mode: crate::RuntimeMode::Strict,
        }
    }
}
```

---

## 6. Error Taxonomy

### 6.1 `AsupersyncError` Enum

```rust
/// Unified error type for all ASUPERSYNC operations.
///
/// Each variant maps to one or more entries in the contract error ledger
/// (ASUPERSYNC_CONTRACT_TABLE.md section 4) and threat surfaces
/// (ASUPERSYNC_THREAT_MODEL.md section 1).
#[derive(Debug, thiserror::Error)]
pub enum AsupersyncError {
    // --- Codec errors ---

    /// Encoding failed (e.g., RaptorQ encoder initialization error).
    /// Maps to: Error Ledger #3 (RaptorQ decode failure, planned).
    #[error("codec encode failed: {reason}")]
    CodecEncode { reason: String },

    /// Decoding failed due to insufficient symbols.
    /// Maps to: Error Ledger #3, ATS-4.1.
    #[error("codec decode failed: {reason}, available_symbols={available}, required={required}")]
    CodecDecode {
        reason: String,
        available: u32,
        required: u32,
    },

    /// Payload was empty (cannot encode zero-length data).
    #[error("empty payload: cannot encode zero-length artifact")]
    EmptyPayload,

    // --- Integrity errors ---

    /// Hash verification failed for payload or symbol.
    /// Maps to: ATS-4.2, ATS-4.3, CON-INTEGRITY-DETERMINISTIC.
    #[error("integrity verification failed: expected={expected}, actual={actual}")]
    IntegrityFailure { expected: String, actual: String },

    /// Decode proof does not bind to the expected artifact.
    /// Maps to: ATS-4.3, R-04.
    #[error("decode proof artifact mismatch: proof_artifact={proof_artifact}, expected={expected}")]
    ProofArtifactMismatch {
        proof_artifact: String,
        expected: String,
    },

    /// Sidecar manifest is inconsistent (mismatched symbol counts, etc.).
    #[error("sidecar manifest inconsistency: {detail}")]
    SidecarInconsistency { detail: String },

    /// Placeholder envelope encountered in a decode path.
    /// Maps to: ATS-4.1, R-08.
    #[error("placeholder envelope in decode path: artifact_id={artifact_id}")]
    PlaceholderInDecodePath { artifact_id: String },

    // --- Transport errors ---

    /// Send operation failed.
    #[error("transport send failed to {endpoint}: {reason}")]
    TransportSend { endpoint: String, reason: String },

    /// Receive operation failed.
    #[error("transport receive failed from {endpoint}: {reason}")]
    TransportReceive { endpoint: String, reason: String },

    /// Requested artifact not found at source.
    #[error("artifact not found: id={artifact_id}, source={source}")]
    NotFound { artifact_id: String, source: String },

    // --- Capability errors ---

    /// Operation requires a capability not present in the Cx context.
    /// Maps to: Cx capability threading (planned, section 4a).
    #[error("missing capability {capability:?} for operation {operation}")]
    CapabilityMissing {
        capability: CxCapability,
        operation: String,
    },

    // --- Budget/cancellation errors ---

    /// Operation was cancelled via Cx cancellation.
    /// Maps to: Error Ledger #4 (Cx cancellation propagation).
    #[error("operation cancelled: {reason}")]
    Cancelled { reason: String },

    /// Budget (deadline, poll, or cost) exhausted.
    /// Maps to: Error Ledger #5 (Budget exhaustion).
    #[error("budget exhausted: {kind}")]
    BudgetExhausted { kind: String },

    // --- Recovery errors ---

    /// Recovery attempts exhausted without success.
    #[error("recovery failed after {attempts} attempts: {last_reason}")]
    RecoveryExhausted { attempts: u32, last_reason: String },

    /// Panicked outcome received -- must not retry (INV-SUPERVISION-MONOTONE).
    /// Maps to: ATS-6.1, rule 7a.
    #[error("panicked outcome: supervision monotonicity prohibits retry")]
    PanickedNoRetry,

    // --- Configuration errors ---

    /// Invalid configuration value.
    #[error("configuration error: {detail}")]
    Configuration { detail: String },

    // --- Underlying IO ---

    /// Wraps `std::io::Error`.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Wraps serialization/deserialization errors.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Decode proof count exceeds maximum (R-07).
    #[error("decode proof limit exceeded: count={count}, max={max}")]
    DecodeProofLimitExceeded { count: usize, max: usize },
}
```

### 6.2 Contract Error Ledger Mapping

| Error Ledger Entry | `AsupersyncError` Variant |
|---|---|
| #1: `RuntimeError::ClockSkew` | N/A (remains in `RuntimeError`; Clock trait handles this) |
| #2: Compilation error (feature missing) | N/A (compile-time; no runtime representation) |
| #3: RaptorQ decode failure | `CodecDecode` |
| #4: Cx cancellation propagation | `Cancelled` |
| #5: Budget exhaustion | `BudgetExhausted` |
| (new) Obligation leak | Not represented yet (deferred to obligation model) |

### 6.3 Conversion Impls

```rust
impl From<std::io::Error> for AsupersyncError {
    fn from(e: std::io::Error) -> Self {
        AsupersyncError::Io(e)
    }
}

impl From<serde_json::Error> for AsupersyncError {
    fn from(e: serde_json::Error) -> Self {
        AsupersyncError::Serialization(e.to_string())
    }
}

// When the asupersync feature is enabled and provides typed errors:
// impl From<asupersync::raptorq::RaptorQError> for AsupersyncError { ... }
// impl From<asupersync::transport::TransportError> for AsupersyncError { ... }
```

---

## 7. Implementation Sequencing

### Phase 1: Clock + Codec + Integrity (Pure Computation, No IO)

**Estimated scope:** ~300-400 LOC implementation + ~200 LOC tests
**Duration:** 1 sprint

**Deliverables:**
1. `clock.rs` -- `Clock` trait, `SystemClock`, `FakeClock`
2. Refactor `now_unix_ms()` to use `Clock` trait; update `decide()` signature
3. `asupersync/error.rs` -- `AsupersyncError` enum (all variants)
4. `asupersync/codec.rs` -- `ArtifactCodec` trait + `RaptorQCodec` impl
5. `asupersync/integrity.rs` -- `IntegrityVerifier` trait + `Sha256Verifier` impl
6. Extend `DecodeProof` with `artifact_id` and `nonce` fields (R-04)
7. Add `MAX_DECODE_PROOFS` cap to `RaptorQEnvelope` (R-07)
8. Change placeholder `ScrubStatus.status` from `"ok"` to `"placeholder"` (R-08)

**Dependencies:** None. All work is in `fp-runtime` and is pure computation.

**Test strategy:**
- Unit tests for `Clock` trait: `SystemClock` returns non-zero, `FakeClock` is deterministic
- Unit tests for `decide()` with injected `FakeClock`: timestamps are deterministic
- Property tests for `ArtifactCodec`: roundtrip encode/decode for arbitrary payloads
- Unit tests for `IntegrityVerifier`: known SHA-256 vectors, mismatch detection
- Unit tests for `DecodeProof` artifact binding: reject unbound proofs
- Unit tests for decode proof cap: verify truncation at limit

**Breaking changes:**
- `decide()` gains a `clock: &dyn Clock` parameter (internal function, not public API).
- `RuntimePolicy::decide_unknown_feature()` and `decide_join_admission()` gain a
  `clock: &dyn Clock` parameter. These are public API changes. Provide a backward-
  compatible overload or use a builder pattern with `SystemClock` as default.
- `DecodeProof` struct gains two new fields. Add `#[serde(default)]` for backward
  compatibility with existing serialized proofs.
- `ScrubStatus` placeholder value changes from `"ok"` to `"placeholder"`. This
  affects `fp-conformance` tests that assert `scrub.status == "ok"` for placeholders.

### Phase 2: Config + Transport Abstraction (Trait Definitions)

**Estimated scope:** ~200-250 LOC implementation + ~100 LOC tests
**Duration:** 0.5 sprint

**Deliverables:**
1. `asupersync/config.rs` -- `AsupersyncConfig`, `CxCapability`, `CapabilitySet`,
   `OperationRequirements`, `CodecConfig`
2. `asupersync/transport.rs` -- `TransportLayer` trait, `TransferReport`, `TransferStatus`
3. New types: `SidecarManifest`, `SymbolHashEntry`, `EncodedArtifact`, `EncodedSymbol`
4. `asupersync/mod.rs` -- module re-exports

**Dependencies:** Phase 1 (error types, codec trait).

**Test strategy:**
- Unit tests for `CapabilitySet::satisfies()` and `missing()`: all combinations
- Unit tests for `AsupersyncConfig::default()`: verify all fields have sensible values
- Unit tests for `TransferReport` serialization roundtrip
- Mock `TransportLayer` implementation for testing (returns canned responses)
- No integration tests yet (no real transport implementation)

**Breaking changes:** None (all new types, feature-gated).

### Phase 3: Recovery Workflow (Depends on Phase 1 + 2)

**Estimated scope:** ~250-300 LOC implementation + ~150 LOC tests
**Duration:** 1 sprint

**Deliverables:**
1. `asupersync/recovery.rs` -- `RecoveryPolicy` trait, `RecoveryPlan`,
   `RecoveryOutcome`, `DefaultRecoveryPolicy`
2. Recovery orchestrator function that combines codec decode, integrity check,
   transport fetch, and retry logic
3. INV-SUPERVISION-MONOTONE enforcement: debug assertion that `Panicked` outcomes
   are never retried
4. Error classification hook: distinguish transient from permanent errors (R-05)

**Dependencies:** Phase 1 (codec, integrity, error types), Phase 2 (transport trait,
config types).

**Test strategy:**
- Unit tests for `DefaultRecoveryPolicy::should_retry()`:
  - Returns `false` for `RecoveryOutcome::Panicked` (monotonicity)
  - Returns `false` when `attempt >= max_retries`
  - Returns `true` for transient errors within budget
  - Returns `false` for permanent errors
- Unit tests for `retry_delay_ms()`: exponential backoff calculation
- Unit tests for recovery orchestrator:
  - Succeeds on first attempt when all symbols present
  - Retries and succeeds when transport fails once then succeeds
  - Gives up after max retries
  - Respects deadline (with FakeClock)
- Property test: retry count never exceeds `max_retries + 1`

**Breaking changes:** None (all new types, feature-gated).

### Phase 4: Integration Tests + Conformance Harness Hooks

**Estimated scope:** ~200-300 LOC tests + ~50 LOC harness glue
**Duration:** 0.5 sprint

**Deliverables:**
1. Integration test: encode artifact -> transfer (mock) -> receive -> decode -> verify
2. Integration test: encode -> drop symbols -> recover via recovery workflow
3. Conformance harness hook: `fp-conformance` uses `ArtifactCodec` trait instead
   of direct `raptorq` crate calls (optional migration, non-breaking)
4. Feature-gated integration test for `outcome_to_action` bridge with real
   `asupersync::Outcome` values
5. CI configuration for `--features asupersync` test matrix

**Dependencies:** Phase 1 + 2 + 3 all complete.

**Test strategy:**
- End-to-end integration tests in `crates/fp-runtime/tests/`
- Cross-crate integration test if needed (fp-runtime + fp-conformance)
- Conformance harness: verify `generate_raptorq_sidecar` produces artifacts
  that pass `IntegrityVerifier::verify_sidecar()`
- CI: separate test job with `--features asupersync` flag

**Breaking changes:** None (new tests only).

### Phase Summary

| Phase | Scope | Duration | Depends On | Key Deliverable |
|---|---|---|---|---|
| 1 | Clock + Codec + Integrity | 1 sprint | Nothing | INV-NO-AMBIENT-AUTHORITY remediated |
| 2 | Config + Transport | 0.5 sprint | Phase 1 | Transport trait + config types |
| 3 | Recovery | 1 sprint | Phase 1+2 | Recovery orchestrator |
| 4 | Integration tests | 0.5 sprint | Phase 1+2+3 | End-to-end verification |

---

## 8. Capability Context Integration

### 8.1 Cx Interaction Model

The asupersync `Cx<Caps>` type carries phantom capability flags. FrankenPandas
ASUPERSYNC modules interact with Cx at these points:

| Module | Operation | Required Capabilities | Cx Interaction |
|---|---|---|---|
| `clock` | `Clock::now_unix_ms()` | `TIME` | Under Cx: `cx.time().unix_ms()` |
| `codec` | `ArtifactCodec::encode()` | (none -- pure computation) | `cx.checkpoint()` for cancellation |
| `codec` | `ArtifactCodec::decode()` | (none -- pure computation) | `cx.checkpoint()` for cancellation |
| `transport` | `TransportLayer::send()` | `IO`, `REMOTE` | `cx.scope_with_budget()` for deadline |
| `transport` | `TransportLayer::receive()` | `IO`, `REMOTE` | `cx.scope_with_budget()` for deadline |
| `integrity` | `IntegrityVerifier::verify_sidecar()` | (none -- pure computation) | `cx.checkpoint()` for large manifests |
| `recovery` | Recovery orchestrator | `TIME`, `IO`, `REMOTE` | Full Cx threading for retry delays |

### 8.2 Capability Narrowing

Following the FrankenSQLite pattern, capabilities are narrowed as operations
become more specific:

```
FpFullCaps         = { TIME, IO, SPAWN, RANDOM, REMOTE }
  |
  +-- FpTransportCaps = { TIME, IO, REMOTE }
  |     (for transport.send/receive)
  |
  +-- FpComputeCaps   = { TIME }
  |     (for codec.encode/decode, integrity.verify)
  |
  +-- FpRecoveryCaps  = { TIME, IO, REMOTE }
        (for recovery orchestrator)
```

### 8.3 Fail-Closed on Missing Capabilities

When a required capability is absent from the Cx context:

| Mode | Behavior |
|---|---|
| Strict | Return `AsupersyncError::CapabilityMissing` immediately; do not attempt operation |
| Hardened | Return `AsupersyncError::CapabilityMissing` immediately; log the gap to evidence ledger |

**No capability degradation is permitted.** An operation that requires `IO` must
not silently fall back to a no-op when `IO` is missing. This is the fail-closed
doctrine applied to the capability system.

### 8.4 Pre-Cx Stepping Stone

Before full Cx integration, operations accept an optional `&dyn Clock` parameter
(Phase 1) and use `CapabilitySet` for declarative requirements checking (Phase 2).
This allows test code to verify capability requirements without the full Cx type
parameter machinery.

---

## 9. INV-NO-AMBIENT-AUTHORITY Remediation

### 9.1 The Violation

**Location:** `crates/fp-runtime/src/lib.rs`, line 264-270.

```rust
fn now_unix_ms() -> Result<u64, RuntimeError> {
    let ms = SystemTime::now()           // <-- AMBIENT AUTHORITY
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RuntimeError::ClockSkew)?
        .as_millis();
    Ok(ms as u64)
}
```

This is the **sole** ambient authority violation in `fp-runtime` (confirmed by
ASUPERSYNC_THREAT_MODEL.md section 5.1, ATS-3.1 severity: HIGH, likelihood:
CERTAIN).

### 9.2 Callers

`now_unix_ms()` is called once, in `decide()` at line 308:

```rust
ts_unix_ms: now_unix_ms().unwrap_or_default(),
```

`decide()` is called by:
- `RuntimePolicy::decide_unknown_feature()` (line 197)
- `RuntimePolicy::decide_join_admission()` (line 240)

All `DecisionRecord` instances carry potentially non-deterministic timestamps.

### 9.3 Remediation Plan

**Step 1: Add `Clock` trait (new file `crates/fp-runtime/src/clock.rs`).**

See section 4.1 for the full trait definition. The trait is NOT feature-gated.

**Step 2: Refactor `decide()` to accept a clock.**

```rust
// Before:
fn decide(
    mode: RuntimeMode,
    issue: CompatibilityIssue,
    prior_compatible: f64,
    loss: LossMatrix,
    evidence: Vec<EvidenceTerm>,
) -> DecisionRecord {
    // ...
    ts_unix_ms: now_unix_ms().unwrap_or_default(),
    // ...
}

// After:
fn decide(
    mode: RuntimeMode,
    issue: CompatibilityIssue,
    prior_compatible: f64,
    loss: LossMatrix,
    evidence: Vec<EvidenceTerm>,
    clock: &dyn Clock,
) -> DecisionRecord {
    // ...
    ts_unix_ms: clock.now_unix_ms().unwrap_or_default(),
    // ...
}
```

**Step 3: Update public API methods.**

`RuntimePolicy` gains a `clock` field or the methods accept `&dyn Clock`:

```rust
// Option A: Clock in method signature (minimal change)
pub fn decide_unknown_feature(
    &self,
    subject: impl Into<String>,
    detail: impl Into<String>,
    ledger: &mut EvidenceLedger,
    clock: &dyn Clock,
) -> DecisionAction { ... }

// Option B: Clock in RuntimePolicy (more ergonomic, avoids threading)
pub struct RuntimePolicy {
    pub mode: RuntimeMode,
    pub fail_closed_unknown_features: bool,
    pub hardened_join_row_cap: Option<usize>,
    clock: Box<dyn Clock>,
}
```

**Recommendation:** Option A (clock in method signature) for Phase 1. It is
the smallest API change and does not require `Box<dyn Clock>` allocation. The
`SystemClock` struct is zero-sized, so `&SystemClock` costs nothing.

**Step 4: Remove `now_unix_ms()` free function.**

After all callers are migrated to `Clock::now_unix_ms()`, delete the free
function. The `use std::time::{SystemTime, UNIX_EPOCH}` import moves into
`clock.rs` only.

**Step 5: Update fp-conformance.**

The `fp-conformance` crate has its own `now_unix_ms()` function (line 989 in
`lib.rs`). This should also be updated to accept a `Clock` parameter, but this
is a lower priority since fp-conformance is a test harness, not production code.

### 9.4 Code Locations That Need Changes

| File | Line(s) | Change |
|---|---|---|
| `crates/fp-runtime/src/lib.rs` | 2 | Remove `use std::time::{SystemTime, UNIX_EPOCH}` |
| `crates/fp-runtime/src/lib.rs` | 264-270 | Delete `now_unix_ms()` free function |
| `crates/fp-runtime/src/lib.rs` | 272-322 | Add `clock: &dyn Clock` parameter to `decide()` |
| `crates/fp-runtime/src/lib.rs` | 172-204 | Add `clock: &dyn Clock` to `decide_unknown_feature()` |
| `crates/fp-runtime/src/lib.rs` | 206-249 | Add `clock: &dyn Clock` to `decide_join_admission()` |
| `crates/fp-runtime/src/lib.rs` | 565-919 | Update all tests to pass `&SystemClock` or `&FakeClock` |
| `crates/fp-runtime/src/clock.rs` | (new) | `Clock` trait, `SystemClock`, `FakeClock` |
| `crates/fp-conformance/src/lib.rs` | 989 | Replace `now_unix_ms()` call with `Clock` (deferred) |

### 9.5 CI Audit Gate (Phase 3 of Remediation)

After remediation, add a CI lint to prevent regression:

```yaml
# In CI pipeline (e.g., .github/workflows/ci.yml):
- name: Audit ambient authority
  run: |
    # Deny direct SystemTime::now() and Instant::now() in fp-runtime
    if grep -rn 'SystemTime::now\|Instant::now' crates/fp-runtime/src/ \
       --include='*.rs' | grep -v 'clock.rs'; then
      echo "ERROR: Ambient authority violation detected outside clock.rs"
      exit 1
    fi
```

---

## 10. Dependency Requirements

### 10.1 New Crate Dependencies

| Dependency | Purpose | Feature-Gated | Status |
|---|---|---|---|
| `async-trait` | `#[async_trait]` for `TransportLayer` | Yes (`asupersync`) | Needed in Phase 2 |
| `sha2` | SHA-256 for integrity verification | No (already in fp-conformance) | Move to workspace dep |
| `hex` | Hex encoding for symbol data | No (already used in fp-conformance) | Move to workspace dep |

**Note:** `sha2` and `hex` are already transitive dependencies via `fp-conformance`.
Promoting them to workspace-level dependencies in the root `Cargo.toml` avoids
version duplication.

No new dependencies are required for Phase 1 (the Clock trait and codec trait
definitions are pure Rust with no external deps).

### 10.2 Feature Flag Design

```toml
# crates/fp-runtime/Cargo.toml (proposed)
[features]
default = []
asupersync = ["dep:asupersync"]
asupersync-transport = ["asupersync", "dep:async-trait"]
asupersync-full = ["asupersync", "asupersync-transport"]
```

| Feature | Pulls In | Use Case |
|---|---|---|
| `asupersync` | `asupersync` crate (minimal) | Outcome bridge + codec + integrity |
| `asupersync-transport` | `async-trait` | Transport layer (async send/receive) |
| `asupersync-full` | All of the above | Complete ASUPERSYNC integration |

**Rationale:** Users who only need the outcome bridge and codec (synchronous,
no IO) should not pay for the async transport dependencies. The layered feature
flags allow progressive opt-in.

### 10.3 MSRV Implications

- `fp-runtime` uses `edition = "2024"`, requiring Rust 1.85+.
- `async-trait` requires Rust 1.75+ (well within the edition 2024 floor).
- `asupersync` 0.1.1 requires Rust 1.80+ (stated in its Cargo.toml).
- No MSRV concerns. All dependencies are compatible with the edition 2024 floor.

---

## 11. Test Strategy

### 11.1 Unit Tests Per Module

#### `clock.rs`

| Test | Description | Type |
|---|---|---|
| `system_clock_returns_nonzero` | `SystemClock::now_unix_ms()` returns a value > 1_000_000_000_000 | Unit |
| `fake_clock_deterministic` | `FakeClock::new(42)` returns 42 on every call | Unit |
| `fake_clock_advance` | `FakeClock::advance(100)` increments by 100 | Unit |
| `fake_clock_concurrent` | Two threads reading `FakeClock` see consistent values | Unit |
| `decide_with_fake_clock_produces_exact_timestamp` | `decide()` with `FakeClock(12345)` produces `ts_unix_ms = 12345` | Integration |

#### `asupersync::codec`

| Test | Description | Type |
|---|---|---|
| `encode_decode_roundtrip_small` | 100-byte payload encodes and decodes correctly | Unit |
| `encode_decode_roundtrip_large` | 1MB payload roundtrip | Unit |
| `encode_empty_payload_errors` | Empty slice returns `AsupersyncError::EmptyPayload` | Unit |
| `decode_with_dropped_source_symbols` | Drop 25% of source symbols, recover via repair | Unit |
| `decode_insufficient_symbols_errors` | Drop > repair count symbols, get `CodecDecode` | Unit |
| `min_symbols_for_decode_correct` | Returns `k` (source symbol count) | Unit |
| `encoded_artifact_serialization_roundtrip` | JSON serialize/deserialize `EncodedArtifact` | Unit |

#### `asupersync::integrity`

| Test | Description | Type |
|---|---|---|
| `sha256_known_vector` | Hash of "hello" matches known SHA-256 | Unit |
| `verify_payload_valid` | `verify_payload(data, hash(data))` returns `Valid` | Unit |
| `verify_payload_invalid` | `verify_payload(data, wrong_hash)` returns `Invalid` | Unit |
| `verify_sidecar_all_valid` | All symbol hashes match | Unit |
| `verify_sidecar_one_corrupt` | One symbol hash mismatches, strict mode fails fast | Unit |
| `verify_sidecar_hardened_reports_all` | Hardened mode reports all mismatches | Unit |
| `placeholder_sentinel_detected` | `"blake3:placeholder"` hash detected as placeholder | Unit |

#### `asupersync::recovery`

| Test | Description | Type |
|---|---|---|
| `panicked_outcome_never_retried` | `should_retry(Panicked, ...)` returns `false` | Unit |
| `transient_error_retried_within_budget` | Retries up to `max_retries` | Unit |
| `permanent_error_not_retried` | Permanent errors give up immediately | Unit |
| `backoff_exponential` | Delays double each attempt | Unit |
| `backoff_capped` | Delay does not exceed `max_backoff_ms` | Unit |
| `deadline_respected` | No retry after deadline passed (with FakeClock) | Unit |

#### `asupersync::config`

| Test | Description | Type |
|---|---|---|
| `capability_set_satisfies` | `{TIME, IO}` satisfies `{TIME}` | Unit |
| `capability_set_missing` | `{TIME}` missing `IO` from `{TIME, IO}` requirement | Unit |
| `default_config_sensible` | All defaults are non-zero and finite | Unit |
| `config_serialization_roundtrip` | JSON roundtrip for `AsupersyncConfig` | Unit |

### 11.2 Integration Tests

| Test | Description | Modules Exercised |
|---|---|---|
| `encode_transfer_decode_verify` | Full pipeline: encode -> mock transport -> decode -> integrity verify | codec, transport, integrity |
| `recovery_with_symbol_loss` | Encode, drop symbols, recover via recovery orchestrator | codec, recovery, integrity |
| `outcome_bridge_with_real_asupersync` | Feature-gated: test all 4 Outcome variants | outcome_to_action (existing) |
| `conformance_sidecar_via_trait` | fp-conformance uses `ArtifactCodec` trait | codec, integrity, fp-conformance |

### 11.3 Property Tests

Using `proptest` or `quickcheck` (already available in workspace for property testing):

| Property | Description |
|---|---|
| `forall payload: encode(payload) -> decode -> payload` | Codec roundtrip for arbitrary payloads |
| `forall payload: hash(payload) == hash(payload)` | Hash determinism |
| `forall payload, symbols_dropped <= repair_count: decode succeeds` | Erasure coding recovery guarantee |
| `forall plan, attempt > max_retries: should_retry == false` | Recovery policy termination |

### 11.4 Conformance Harness Hooks

The `fp-conformance` crate currently calls `raptorq` directly. Phase 4 introduces
an optional migration path:

1. `fp-conformance` depends on `fp-runtime` with `features = ["asupersync"]`
2. `generate_raptorq_sidecar()` delegates to `ArtifactCodec::encode()` when available
3. `verify_raptorq_sidecar()` delegates to `IntegrityVerifier::verify_sidecar()`
4. Existing `raptorq`-direct code paths remain as fallback (feature off)

This ensures the conformance harness exercises the same code paths as production
ASUPERSYNC usage.

---

## 12. Appendix: Contract-to-Code Crosswalk

This table maps every behavioral contract and invariant from the ASUPERSYNC
artifact suite to specific Rust types, traits, and enforcement mechanisms.

### 12.1 Behavioral Contracts -> Rust Types

| Contract (Anchor Map) | Rust Type/Trait | Module | Enforcement |
|---|---|---|---|
| 1. Outcome-to-action bridge | `outcome_to_action()` | `lib.rs` (existing) | Exhaustive match, `#[must_use]` |
| 2. Feature-gated compilation | `#[cfg(feature = "asupersync")]` | `lib.rs`, `asupersync/` | Cargo feature system |
| 3. Cx capability threading | `CxCapability`, `CapabilitySet` | `asupersync::config` | `CapabilitySet::satisfies()` check |
| 4. RaptorQ durability pipeline | `ArtifactCodec`, `RaptorQCodec` | `asupersync::codec` | Trait contract + property tests |
| 5. Lab runtime determinism | `Clock` trait, `FakeClock` | `clock.rs` | Injectable time replaces ambient |
| 6. Singleton outcome mapping | `outcome_to_action()` | `lib.rs` (existing) | Stateless function, `&` input |
| 7. Feature flag isolation | `default = []` | `Cargo.toml` | No transitive enablement |
| 8. RaptorQ placeholder path | `RaptorQEnvelope::placeholder()` | `lib.rs` (existing) | Sentinel hash check |
| 9. Clock dependency | `Clock` trait | `clock.rs` | Replaces `SystemTime::now()` |
| 10. Budget/deadline absence | `RecoveryPlan.deadline_unix_ms` | `asupersync::recovery` | Budget field in plan |

### 12.2 Invariants -> Enforcement

| Invariant | Rust Enforcement | Location |
|---|---|---|
| INV-OUTCOME-EXHAUSTIVE | Exhaustive `match` on `Outcome` | `lib.rs:556-562` |
| INV-OUTCOME-STATELESS | `&Outcome` input, no `&mut`, no side effects | `lib.rs:555` |
| INV-OUTCOME-MUST-USE | `#[must_use]` attribute | `lib.rs:554` |
| INV-FEATURE-ISOLATED | `default = []` in Cargo.toml | `Cargo.toml:7` |
| INV-FEATURE-NO-TRANSITIVE | Workspace audit (CI check) | CI pipeline |
| INV-LEDGER-APPEND-ONLY | `EvidenceLedger` API: only `push()`, `records()` | `lib.rs:128-143` |
| INV-CONFORMAL-ALPHA-CLAMPED | `.clamp(0.01, 0.5)` | `lib.rs:428` |
| INV-STRICT-FAIL-CLOSED | Post-decision override to `Reject` | `lib.rs:198-199` |
| INV-HARDENED-CAP-REPAIR | Post-decision override to `Repair` | `lib.rs:242-244` |
| INV-PLACEHOLDER-SENTINEL | `"blake3:placeholder"` in constructor | `lib.rs:362` |
| INV-NO-UNSAFE | `#![forbid(unsafe_code)]` | `lib.rs:1` |
| INV-NO-AMBIENT-AUTHORITY | `Clock` trait injection | `clock.rs` (Phase 1) |
| INV-SUPERVISION-MONOTONE | `should_retry(Panicked) == false` + debug assert | `asupersync::recovery` (Phase 3) |
| INV-OBLIGATION-RESOLVE | (Deferred to obligation model design) | N/A |

### 12.3 Threat Surfaces -> Modules

| Threat Surface | Responsible Module | Mitigation |
|---|---|---|
| ATS-1 (Outcome bridge) | `lib.rs` (existing) | No change; exhaustive match |
| ATS-2 (Feature gate) | `Cargo.toml`, CI | Feature flag audit in CI |
| ATS-3 (Clock/timestamp) | `clock.rs` | `Clock` trait injection (Phase 1) |
| ATS-4 (RaptorQ integrity) | `asupersync::integrity`, `asupersync::codec` | `IntegrityVerifier`, proof binding, proof cap |
| ATS-5 (Cancellation/budget) | `asupersync::recovery`, `asupersync::config` | `RecoveryPlan` deadline, Cx checkpoint (future) |
| ATS-6 (Supervision) | `asupersync::recovery` | `RecoveryPolicy` monotonicity enforcement |

### 12.4 Recommendations -> Implementation Phase

| Recommendation | Phase | Module |
|---|---|---|
| R-01: Remediate `SystemTime::now()` | Phase 1 | `clock.rs` |
| R-02: Eliminate zero-timestamp sentinel | Phase 1 | `clock.rs`, `lib.rs` |
| R-03: Integrate `SecurityContext` | Future (beyond Phase 4) | `asupersync::integrity` |
| R-04: Add artifact binding to `DecodeProof` | Phase 1 | `lib.rs` |
| R-05: Error severity classification | Phase 3 | `asupersync::recovery` |
| R-06: Cancellation checkpoints | Future (Cx integration) | All kernel crates |
| R-07: Cap `decode_proofs` vector | Phase 1 | `lib.rs` |
| R-08: Placeholder status `"ok"` -> `"placeholder"` | Phase 1 | `lib.rs` |
| R-09: Prior clamping in `decide()` | Phase 1 | `lib.rs` |
| R-10: Cx capability narrowing | Future (Cx integration) | All crates |
| R-11: Supervision tree | Future | `asupersync::recovery` |
| R-12: Compile-time audit gate | Phase 1 (CI only) | CI pipeline |
| R-13: Budget enforcement | Future (Cx integration) | All entry points |
| R-14: `cargo-deny` configuration | Phase 1 (CI only) | CI pipeline |

### 12.5 Contract Table -> Error Mapping

| Contract Table Field | Rust Representation |
|---|---|
| `input_contract` | Function parameter types on trait methods |
| `output_contract` | Return types on trait methods |
| `error_contract` | `AsupersyncError` enum variants |
| `null_contract` | `Option<T>` fields, `#[serde(default)]`, sentinel checks |
| `strict_mode_policy` | `RuntimeMode::Strict` checks in integrity + recovery |
| `hardened_mode_policy` | `RuntimeMode::Hardened` checks in integrity + recovery |
| `excluded_scope` | Not implemented in Phase 1-4 (FTUI, full Cx, lab runtime) |
| `oracle_tests` | Property tests + conformance harness hooks (Phase 4) |
| `performance_sentinels` | `#[bench]` tests (future), complexity documented in trait docs |
| `compatibility_risks` | `AsupersyncError` variants + R-01 through R-14 |
| `raptorq_artifacts` | `ArtifactCodec` trait, `EncodedArtifact` type |

---

## Drift Gates

This integration plan remains valid as long as:

1. `fp-runtime` remains the sole workspace crate with an `asupersync` dependency.
2. The `asupersync` crate version stays within `0.1.x` semver range.
3. `Outcome<T, E>` retains exactly four variants without `#[non_exhaustive]`.
4. `#![forbid(unsafe_code)]` remains on `fp-runtime`.
5. The conformance harness (`fp-conformance`) continues to use the `raptorq` crate
   directly (until Phase 4 migration).
6. No other workspace crate introduces async runtime dependencies.

If any gate is violated, this document must be revised before proceeding with
the affected implementation phase.
