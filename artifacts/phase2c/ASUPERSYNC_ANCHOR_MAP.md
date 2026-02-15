# ASUPERSYNC Anchor Map + Behavior/Workflow Extraction Ledger

Bead: `bd-2gi.27.1` [ASUPERSYNC-A]
Subsystem: `fp-runtime` asupersync outcome bridge + artifact synchronization integration

## Legacy Anchors

### External Crate (Published)

- `asupersync` crate v0.1.1 (registry: crates.io, checksum `88eafdfdd913f7464e5329597771f95bb6fa6b0edb6e75ac8542de43c806f822`)
- Source modules (per FrankenSQLite spec reference paths):
  - `src/raptorq/gf256.rs` -- GF(256) arithmetic engine (64KB MUL_TABLES)
  - `src/raptorq/linalg.rs` -- sparse/dense linear algebra over GF(256)
  - `src/raptorq/systematic.rs` -- systematic index table + tuple generator
  - `src/raptorq/decoder.rs` -- inactivation decoder (peeling + Gaussian)
  - `src/raptorq/proof.rs` -- explainable decode proofs / failure reasons
  - `src/raptorq/pipeline.rs` -- end-to-end sender/receiver pipelines
  - `src/distributed/` -- quorum routing + recovery
  - `src/cx.rs` -- `Cx` capability context (cancellation, budgets, capability narrowing)
  - `src/lab/` -- `LabRuntime`, deterministic scheduling, `InjectionStrategy`, oracles, e-process monitors
  - `src/channel/mpsc.rs` -- cancel-safe two-phase MPSC channels
  - `src/channel/session.rs` -- obligation-tracked session channels
  - `src/transport/` -- `SymbolSink`, `SymbolStream`, `SymbolRouter`, `VirtualTcp`
  - `src/security/` -- `SecurityContext`, `AuthenticatedSymbol`
  - `src/epoch/` -- `EpochClock`, `EpochId`, epoch barrier
  - `src/trace/` -- `TraceEvent`, `TlaExporter`, Mazurkiewicz trace monoid, sheaf checker
  - `src/runtime/` -- `RuntimeBuilder`, `spawn_blocking`, `spawn_blocking_io`, deadline monitoring
  - `src/combinator/` -- `quorum`, `pipeline`, `bulkhead`, `governor`, `rate_limit`
  - `src/net/unix.rs` -- `UnixStream`, `SocketAncillary` (SCM_RIGHTS)

### FrankenPandas Integration Points

- `crates/fp-runtime/Cargo.toml` -- optional dependency: `asupersync = { version = "0.1.1", optional = true, default-features = false }`
- `crates/fp-runtime/src/lib.rs` -- `outcome_to_action()` bridge function (lines 553-563, gated on `#[cfg(feature = "asupersync")]`)
- `README.md` line 31 -- "optional `asupersync` outcome bridge in `fp-runtime`"
- `PROPOSED_ARCHITECTURE.md` line 30 -- "`fp-runtime` offers optional `asupersync` outcome-to-policy adapter hooks"
- `TODO_EXECUTION_TRACKER.md` section N -- planned deep integration items N1-N6

### Reference Specifications

- `references/frankensqlite/COMPREHENSIVE_SPEC_FOR_FRANKENSQLITE_V1.md` section 4 ("Asupersync Deep Integration") -- canonical behavioral contract for Cx, LabRuntime, RaptorQ pipelines, obligations, supervision, resilience, epochs, remote effects, scheduler lanes

## Extracted Behavioral Contract

### Normal Conditions

1. **Outcome-to-action bridge:** `outcome_to_action()` maps the asupersync 4-valued `Outcome<T, E>` lattice to the FrankenPandas `DecisionAction` enum:
   - `Outcome::Ok(_)` maps to `DecisionAction::Allow`
   - `Outcome::Err(_)` maps to `DecisionAction::Repair`
   - `Outcome::Cancelled(_)` maps to `DecisionAction::Reject`
   - `Outcome::Panicked(_)` maps to `DecisionAction::Reject`

2. **Feature-gated compilation:** The integration is entirely optional. When `asupersync` feature is disabled (the default), no asupersync types or functions are compiled. The rest of `fp-runtime` (evidence ledger, Bayesian decision engine, RaptorQ envelope types, conformal guard) functions without the asupersync dependency.

3. **Cx capability threading (planned):** All FrankenPandas operations are expected to eventually accept `&Cx` for cooperative cancellation, deadline propagation, and capability narrowing. The `Cx` type parameter carries phantom capability flags `[SPAWN, TIME, RANDOM, IO, REMOTE]`.

4. **RaptorQ durability pipeline:** Artifact bundles (conformance packets, benchmark baselines, migration manifests, reproducibility ledgers) are expected to carry RaptorQ sidecar repair symbols via asupersync's `RaptorQSenderBuilder`/`RaptorQReceiverBuilder` pipelines. Current implementation uses placeholder `RaptorQEnvelope` structs.

5. **Lab runtime determinism:** Under planned integration, deterministic testing via `LabRuntime` provides virtual time, deterministic scheduling, oracle suites, and replay capability. All lab-mode decode proofs are auditable.

### Edge Conditions

6. **Singleton outcome mapping:** The `outcome_to_action` function is a pure mapping with no state. It does not record to the evidence ledger, does not consult the loss matrix, and does not interact with the Bayesian posterior. It is a translation layer only.

7. **Feature flag isolation:** With `default = []` in `fp-runtime/Cargo.toml`, the asupersync dependency is never pulled transitively. Other crates in the workspace do not depend on asupersync. The Cargo.lock shows asupersync's dependency tree includes: `base64`, `bincode`, `crossbeam-queue`, `getrandom`, `libc`, `nix`, `parking_lot`, `pin-project`, `polling`, `rmp-serde`, `serde`.

8. **RaptorQ envelope placeholder path:** `RaptorQEnvelope::placeholder()` creates envelopes with `source_hash: "blake3:placeholder"`, `k: 0`, `repair_symbols: 0`, empty symbol hashes. This is valid for artifact metadata before actual RaptorQ encoding is wired.

9. **Clock dependency:** The `now_unix_ms()` function used by the decision engine calls `SystemTime::now()` directly. Under full asupersync integration, this would need to route through `Cx` time capabilities (INV-NO-AMBIENT-AUTHORITY). This is a known gap for lab-mode determinism.

10. **Budget/deadline absence:** Current FrankenPandas has no budget or deadline enforcement. The evidence ledger records timestamps but does not enforce time bounds on operations.

### Adversarial Conditions

11. **Poisoned Outcome variants:** `Outcome::Panicked` carries a panic payload. The bridge maps it to `Reject` without inspecting the payload. No information from the panic is propagated to the decision record.

12. **Feature flag mismatch:** If downstream code expects `outcome_to_action` but the feature is not enabled, compilation fails at the call site. There is no runtime fallback or dynamic feature detection.

13. **RaptorQ symbol injection:** Adversarial symbol data in repair streams could cause decode to produce incorrect results. Asupersync's `SecurityContext` + `AuthenticatedSymbol` system mitigates this via epoch-scoped auth tags. FrankenPandas does not currently use authenticated symbols.

14. **Clock skew under `now_unix_ms()`:** If `SystemTime::now()` returns a time before `UNIX_EPOCH`, `now_unix_ms()` returns `0` (via `unwrap_or_default()`) rather than propagating the error. Decision records silently get timestamp `0`.

15. **Cancellation starvation:** Without `Cx` checkpoint integration, long-running FrankenPandas operations (groupby on large datasets, full-table alignment) cannot be cooperatively cancelled. A cancelled asupersync task hosting FrankenPandas logic would need to wait for the entire operation to complete.

## Type Inventory

### FrankenPandas Types (fp-runtime)

- `RuntimeMode` -- enum: `Strict`, `Hardened` (serde: snake_case)
- `DecisionAction` -- enum: `Allow`, `Reject`, `Repair` (serde: snake_case)
- `IssueKind` -- enum: `UnknownFeature`, `MalformedInput`, `JoinCardinality`, `PolicyOverride`
- `CompatibilityIssue` -- struct: `kind: IssueKind`, `subject: String`, `detail: String`
- `EvidenceTerm` -- struct: `name: String`, `log_likelihood_if_compatible: f64`, `log_likelihood_if_incompatible: f64`
- `LossMatrix` -- struct: 6 `f64` fields (allow/reject/repair cross compatible/incompatible)
- `DecisionMetrics` -- struct: `posterior_compatible`, `bayes_factor_compatible_over_incompatible`, `expected_loss_allow`, `expected_loss_reject`, `expected_loss_repair`
- `DecisionRecord` -- struct: `ts_unix_ms: u64`, `mode: RuntimeMode`, `action: DecisionAction`, `issue: CompatibilityIssue`, `prior_compatible: f64`, `metrics: DecisionMetrics`, `evidence: Vec<EvidenceTerm>`
- `GalaxyBrainCard` -- struct: `title`, `equation`, `substitution`, `intuition` (all `String`)
- `EvidenceLedger` -- struct: `records: Vec<DecisionRecord>` (append-only)
- `RuntimePolicy` -- struct: `mode: RuntimeMode`, `fail_closed_unknown_features: bool`, `hardened_join_row_cap: Option<usize>`
- `RuntimeError` -- enum: `ClockSkew`
- `RaptorQEnvelope` -- struct: `artifact_id`, `artifact_type`, `source_hash` (all `String`), `raptorq: RaptorQMetadata`, `scrub: ScrubStatus`, `decode_proofs: Vec<DecodeProof>`
- `RaptorQMetadata` -- struct: `k: u32`, `repair_symbols: u32`, `overhead_ratio: f64`, `symbol_hashes: Vec<String>`
- `ScrubStatus` -- struct: `last_ok_unix_ms: u64`, `status: String`
- `DecodeProof` -- struct: `ts_unix_ms: u64`, `reason: String`, `recovered_blocks: u32`, `proof_hash: String`
- `ConformalPredictionSet` -- struct: `quantile_threshold: f64`, `current_score: f64`, `bayesian_action_in_set: bool`, `admissible_actions: Vec<DecisionAction>`, `empirical_coverage: f64`
- `ConformalGuard` -- struct: `scores: Vec<f64>`, `window_size: usize`, `alpha: f64`, `in_set_count: usize`, `total_count: usize`

### Asupersync Types (external, feature-gated)

- `asupersync::Outcome<T, E>` -- 4-valued result lattice: `Ok(T)`, `Err(E)`, `Cancelled(CancelReason)`, `Panicked(PanicPayload)`. Ordering: `Ok < Err < Cancelled < Panicked`.
- `asupersync::cx::Cx<Caps>` -- capability context with phantom type parameter encoding `[SPAWN, TIME, RANDOM, IO, REMOTE]`
- `asupersync::types::Budget` -- product lattice with mixed meet/join (deadline + poll quota + cost quota + priority)
- `asupersync::lab::LabRuntime` -- deterministic runtime: scheduling, virtual time, oracle suite, trace certificates
- `asupersync::lab::LabConfig` -- seed, worker_count, max_steps
- `asupersync::lab::InjectionStrategy` -- cancellation injection modes (e.g., `AllPoints`)
- `asupersync::lab::ConformalCalibrator` -- oracle-level conformal calibration
- `asupersync::lab::oracle::eprocess::EProcess` -- e-process monitors for anytime-valid invariant testing
- `asupersync::config::RaptorQConfig` -- encoder/decoder configuration
- `asupersync::raptorq::RaptorQSenderBuilder` / `RaptorQReceiverBuilder` -- pipeline builders
- `asupersync::channel::mpsc` -- cancel-safe two-phase MPSC (reserve/send/abort)
- `asupersync::channel::session::TrackedSender` -- obligation-tracked session channels
- `asupersync::epoch::EpochId` -- monotone `u64` for distributed coordination
- `asupersync::transport::{SymbolSink, SymbolStream, SymbolRouter}` -- transport abstractions
- `asupersync::security::SecurityContext` -- authenticated symbol framework
- `asupersync::trace::TlaExporter` -- TLA+ behavior/skeleton export from traces
- `asupersync::trace::distributed::sheaf` -- sheaf consistency checker
- `asupersync::combinator::{quorum, pipeline, bulkhead, governor, rate_limit}` -- resilience combinators
- `asupersync::runtime::RuntimeBuilder` -- runtime configuration with deadline monitoring
- `asupersync::runtime::{spawn_blocking, spawn_blocking_io}` -- blocking pool dispatch
- `asupersync::net::unix::{UnixStream, SocketAncillary}` -- Unix domain socket support

## Rule Ledger

1. **Outcome mapping (outcome_to_action, line 555):**
   - 1a. `Outcome::Ok(_)` -> `DecisionAction::Allow`. Success outcomes permit the operation.
   - 1b. `Outcome::Err(_)` -> `DecisionAction::Repair`. Errors trigger repair/recovery path.
   - 1c. `Outcome::Cancelled(_)` -> `DecisionAction::Reject`. Cancelled operations are rejected outright.
   - 1d. `Outcome::Panicked(_)` -> `DecisionAction::Reject`. Panics are treated identically to cancellations at the policy layer.
   - 1e. The mapping is exhaustive (match arms cover all four variants).
   - 1f. The function is `#[must_use]` -- callers must consume the returned `DecisionAction`.
   - 1g. The function takes `&asupersync::Outcome<T, E>` by reference (does not consume the outcome).

2. **Feature gate discipline (Cargo.toml):**
   - 2a. `default = []` -- asupersync is never enabled implicitly.
   - 2b. `asupersync = ["dep:asupersync"]` -- feature name matches dependency name.
   - 2c. `default-features = false` on the dependency -- pulls minimal asupersync surface.
   - 2d. No other crate in the workspace declares an asupersync dependency or feature.

3. **Decision engine independence:**
   - 3a. The Bayesian decision engine (`decide()`) operates without asupersync.
   - 3b. The conformal guard (`ConformalGuard`) operates without asupersync.
   - 3c. The evidence ledger (`EvidenceLedger`) operates without asupersync.
   - 3d. The RaptorQ envelope types operate without asupersync (they are structural placeholders).
   - 3e. Galaxy-brain cards (`decision_to_card()`) operate without asupersync.

4. **Planned Cx integration (TODO_EXECUTION_TRACKER section N):**
   - 4a. N1: Extend outcome bridge to carry packet gate summaries and mismatch corpus pointers.
   - 4b. N2: Add deterministic sync schema for conformance/perf artifact bundles.
   - 4c. N3: Implement FTUI packet dashboard cards.
   - 4d. N4: Add FTUI drilldown views for mismatch corpus replay.
   - 4e. N5: Add strict/hardened mode toggle telemetry surfaces in FTUI.
   - 4f. N6: Integration tests validating asupersync + FTUI contract compatibility under packet drift.

5. **RaptorQ placeholder contract:**
   - 5a. `RaptorQEnvelope::placeholder()` accepts `artifact_id` and `artifact_type` as `impl Into<String>`.
   - 5b. Placeholder source hash is `"blake3:placeholder"` (not a real hash).
   - 5c. Placeholder metadata has `k=0`, `repair_symbols=0`, `overhead_ratio=0.0`.
   - 5d. Placeholder scrub status is `"ok"` with `last_ok_unix_ms=0`.
   - 5e. Placeholder decode proofs list is empty.
   - 5f. Actual RaptorQ encoding will use `RaptorQSenderBuilder` from asupersync.

6. **Ambient authority prohibition (planned):**
   - 6a. `SystemTime::now()` is called directly in `now_unix_ms()` -- violates INV-NO-AMBIENT-AUTHORITY.
   - 6b. Under full Cx integration, time must flow through `Cx` clocks.
   - 6c. Under full Cx integration, randomness (if any) must flow through `Cx` randomness.
   - 6d. No spawning occurs in fp-runtime currently (no violation of spawn prohibition).

7. **Supervision mapping (planned):**
   - 7a. `Outcome::Panicked` must NOT be restarted (INV-SUPERVISION-MONOTONE).
   - 7b. `Outcome::Cancelled` must stop (external directive).
   - 7c. `Outcome::Err` may restart if transient and within restart budget.
   - 7d. `Outcome::Ok` is success; no supervision action needed.

8. **Obligation discipline (planned):**
   - 8a. Reserved obligations must resolve to Committed or Aborted.
   - 8b. Dropping a reserved obligation without resolution is a Leaked state (bug).
   - 8c. Lab mode: leaked obligations must panic (fail fast).
   - 8d. Production mode: leaked obligations must emit diagnostics and escalate.

## Error Ledger

1. **`RuntimeError::ClockSkew`:**
   - Trigger: `SystemTime::now().duration_since(UNIX_EPOCH)` fails.
   - Handling: `now_unix_ms()` returns `Err(RuntimeError::ClockSkew)`.
   - Current caller: `decide()` catches via `unwrap_or_default()`, producing `ts_unix_ms = 0`.
   - Impact: Decision records silently get zero timestamps; no crash, no log.

2. **Compilation error when feature missing:**
   - Trigger: Calling `outcome_to_action()` without `asupersync` feature enabled.
   - Handling: Compile-time error (function does not exist without `#[cfg(feature = "asupersync")]`).
   - Mitigation: Callers must gate their code with `#[cfg(feature = "asupersync")]` as well.

3. **RaptorQ decode failure (planned):**
   - Trigger: Insufficient symbols for object reconstruction.
   - Handling (asupersync): `DecodeProof` artifact emitted with failure reason, received ESIs, recovery status.
   - FrankenPandas impact: Parity reports or conformance artifacts become unrecoverable without repair symbols.

4. **Cx cancellation propagation (planned):**
   - Trigger: Parent region cancelled while FrankenPandas operation in progress.
   - Handling (asupersync): `Cx::checkpoint()` returns `ErrorKind::Cancelled`.
   - FrankenPandas impact: Operation must abort cleanly; no partial results committed to evidence ledger.

5. **Budget exhaustion (planned):**
   - Trigger: Operation exceeds deadline, poll quota, or cost quota.
   - Handling (asupersync): Budget enforcement via `cx.scope_with_budget()`.
   - FrankenPandas impact: Long-running alignment or groupby operations must respect budget bounds.

## Hidden Assumptions

1. **The `Outcome` type has exactly four variants.** The `outcome_to_action` match is exhaustive. If asupersync adds new variants (e.g., `Timeout`), the function will fail to compile, which is the correct behavior.

2. **`Outcome::Err` always means "repairable."** The mapping assumes all error outcomes should trigger `Repair`, not `Reject`. This is a deliberate design choice: errors are treated as recoverable situations where the system should attempt repair before giving up.

3. **`Outcome::Cancelled` and `Outcome::Panicked` are equally severe at the policy layer.** Both map to `Reject`. The distinction between cancellation (cooperative) and panic (programming error) is lost at the `DecisionAction` level. This is intentional: the decision engine treats both as "operation did not succeed; do not proceed."

4. **The bridge function is stateless.** It does not push a `DecisionRecord` to any ledger. The caller is responsible for constructing and recording the full decision context. This separation is intentional to avoid coupling the bridge to a specific ledger instance.

5. **No async/await in fp-runtime.** The current crate is entirely synchronous. Asupersync's async runtime, structured concurrency, and region model are not used. The integration is limited to a type-level bridge.

6. **`default-features = false` on asupersync.** The minimal feature surface is pulled. This assumes asupersync's `Outcome` type is available in the default-free build. If `Outcome` requires a feature flag, the build will fail.

7. **RaptorQ envelopes are structural metadata only.** They do not contain actual encoded symbols or perform encoding/decoding. The `RaptorQMetadata.symbol_hashes` field stores string hashes but the actual RaptorQ encode/decode pipeline is not wired.

8. **The Bayesian decision engine's loss matrix is hardcoded.** The `LossMatrix::default()` values and per-decision priors are chosen by the developer, not derived from asupersync telemetry or calibration data.

## Undefined-Behavior Edges

1. **Full Cx integration path:** How `&Cx` will be threaded through the FrankenPandas call stack (API -> expression planner -> vectorized kernels -> columnar storage -> IO) is unspecified. The capability narrowing scheme (FullCaps -> StorageCaps -> ComputeCaps) from FrankenSQLite provides a template but is not yet adapted.

2. **Artifact sync schema:** The deterministic sync schema for conformance/perf artifact bundles (TODO N2) is unspecified. Which artifacts are ECS objects, what their RaptorQ symbol policies are, and how they are replicated is undefined.

3. **FTUI integration:** The packet dashboard cards, mismatch corpus drilldowns, and mode toggle surfaces (TODO N3-N5) are unspecified beyond their existence as planned items.

4. **Lab runtime harness:** How FrankenPandas conformance tests would run under `LabRuntime` deterministic scheduling is unspecified. Whether the conformance harness would use `FsLab`-style wrappers or direct `LabRuntime` is open.

5. **Cancellation checkpoint placement:** Where `cx.checkpoint()` calls would be inserted in FrankenPandas hot paths (alignment iteration, groupby aggregation, join cross-product expansion, IO parsing) is unspecified.

6. **Budget derivation:** What deadline, poll quota, and cost quota values are appropriate for FrankenPandas operations is unspecified. The budget product lattice semantics (resource constraints tighten by meet, priority propagates by join) apply but parametrization is open.

7. **Epoch semantics for FrankenPandas:** Whether FrankenPandas artifacts need epoch-scoped validity windows (for key rotation, retention policies, cross-process barriers) is unspecified.

8. **Obligation model for evidence ledger:** Whether `EvidenceLedger::push()` should be modeled as a two-phase obligation (reserve -> commit/abort) under asupersync's linear resource discipline is unspecified.

9. **Remote effect scope:** Whether FrankenPandas needs `RemoteCap` for any operations (remote artifact storage, distributed conformance testing) is unspecified.

10. **Supervision policy for FrankenPandas services:** What restart strategies, budgets, and escalation policies apply to FrankenPandas background services (if any) is unspecified.

11. **Authenticated symbols for FrankenPandas artifacts:** Whether conformance packets and benchmark baselines need auth-tagged RaptorQ symbols to prevent tampering is unspecified.
