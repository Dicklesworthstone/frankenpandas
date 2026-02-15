# FRANKENTUI Rust Integration Plan + Module Boundary Skeleton

Bead: `bd-2gi.28.4` [FRANKENTUI-D]
Subsystem: `fp-frankentui` integration architecture, module seams, and phased implementation sequence
Source anchors:
- `artifacts/phase2c/FRANKENTUI_ANCHOR_MAP.md` (bd-2gi.28.1)
- `artifacts/phase2c/FRANKENTUI_CONTRACT_TABLE.md` (bd-2gi.28.2)
- `artifacts/phase2c/FRANKENTUI_THREAT_MODEL.md` (bd-2gi.28.3)

---

## 1. Summary

This document translates the FRANKENTUI contract table and threat model into
concrete Rust module boundaries, integration seams, and an execution sequence
that minimizes rework while preserving strict/hardened compatibility semantics.

The current `fp-frankentui` crate already provides a read-only foundation:
- artifact ingestion via `FtuiDataSource`/`FsFtuiDataSource`
  (`crates/fp-frankentui/src/lib.rs:225`, `crates/fp-frankentui/src/lib.rs:235`),
- packet and dashboard snapshots (`crates/fp-frankentui/src/lib.rs:49`,
  `crates/fp-frankentui/src/lib.rs:118`),
- governance + forensics loading (`crates/fp-frankentui/src/lib.rs:463`,
  `crates/fp-frankentui/src/lib.rs:162`),
- and a non-interactive CLI façade (`crates/fp-frankentui/src/bin/fp-frankentui-cli.rs:18`).

This plan defines the next decomposition so `bd-2gi.28.5` onward can implement
unit/property/differential/e2e evidence against stable module seams.

**Status update (2026-02-15, SwiftTiger):** `bd-2gi.28.8` landed a single-lever
replay-bundle optimization in `crates/fp-frankentui/src/lib.rs`:
`build_frankentui_e2e_replay_bundles()` now resolves fallback case mode via an
indexed lookup table (`HashMap<(&str, &str), RuntimeMode>`) instead of repeated
linear scans over packet results for every forensic `CaseEnd` event. The change
is locked by two proofs:
- `e2e_replay_bundle_optimized_path_is_isomorphic_to_baseline`
- `e2e_replay_bundle_profile_snapshot_reports_lookup_delta`

Snapshot metrics from
`rch exec -- cargo test -p fp-frankentui --lib e2e_replay_bundle_profile_snapshot_reports_lookup_delta -- --nocapture`:
- Baseline `p50/p95/p99` (ns): `20614447 / 24878999 / 26117420`
- Optimized `p50/p95/p99` (ns): `7511826 / 8672051 / 9427574`
- Fallback mode-lookup steps (64 iterations, amplified workload):
  `33718464 -> 65664`

---

## 2. Current State (Implemented)

### 2.1 Implemented crate surface

| Surface | Status | Source anchors |
|---|---|---|
| Packet snapshot aggregation | Implemented | `crates/fp-frankentui/src/lib.rs:49`, `crates/fp-frankentui/src/lib.rs:378` |
| Drift history ingestion with malformed-line tolerance | Implemented | `crates/fp-frankentui/src/lib.rs:69`, `crates/fp-frankentui/src/lib.rs:419` |
| Forensic log ingestion with malformed-line tolerance | Implemented | `crates/fp-frankentui/src/lib.rs:162`, `crates/fp-frankentui/src/lib.rs:431` |
| Governance gate report loading | Implemented | `crates/fp-frankentui/src/lib.rs:169`, `crates/fp-frankentui/src/lib.rs:463` |
| Decision dashboard summarization | Implemented | `crates/fp-frankentui/src/lib.rs:100`, `crates/fp-frankentui/src/lib.rs:558` |
| CLI smoke path | Implemented | `crates/fp-frankentui/src/bin/fp-frankentui-cli.rs:28` |

### 2.2 Upstream integration dependencies

`fp-frankentui` consumes:
- conformance artifacts and types from `fp-conformance`
  (`PacketParityReport`, `PacketGateResult`, `PacketDriftHistoryEntry`,
  `ForensicLog`, `FailureDigest`, `E2eReport`):
  `crates/fp-conformance/src/lib.rs:494`, `crates/fp-conformance/src/lib.rs:512`,
  `crates/fp-conformance/src/lib.rs:575`, `crates/fp-conformance/src/lib.rs:3688`,
  `crates/fp-conformance/src/lib.rs:4044`, `crates/fp-conformance/src/lib.rs:3767`.
- runtime decision-policy and evidence types from `fp-runtime`
  (`DecisionRecord`, `GalaxyBrainCard`, `EvidenceLedger`, `RuntimePolicy`,
  `ConformalGuard`):
  `crates/fp-runtime/src/lib.rs:134`, `crates/fp-runtime/src/lib.rs:145`,
  `crates/fp-runtime/src/lib.rs:179`, `crates/fp-runtime/src/lib.rs:202`,
  `crates/fp-runtime/src/lib.rs:445`.

### 2.3 Threat-model constraints that must remain invariant

From `FRANKENTUI_THREAT_MODEL.md`:
- fail-closed render/degrade behavior on unknown terminal capabilities,
- bounded handling for malformed/partial JSONL,
- non-panicking behavior under missing artifacts,
- explicit strict/hardened policy provenance visibility,
- no mutation of artifacts (read-only cockpit model).

---

## 3. Target Module Boundary Skeleton

`fp-frankentui` currently centralizes logic in `lib.rs`. The target decomposition
is intentionally layered to isolate ingestion, domain normalization, and view
composition.

```text
crates/fp-frankentui/src/
  lib.rs                       # public re-exports, high-level facade
  ingest/
    mod.rs                     # ingestion facade + shared helpers
    packets.rs                 # parity/gate/decode/mismatch loading
    drift.rs                   # drift_history.jsonl streaming parser
    forensic.rs                # forensic JSONL streaming parser
    governance.rs              # governance report parser
  domain/
    mod.rs
    snapshots.rs               # PacketSnapshot, ConformanceDashboardSnapshot, etc.
    policy.rs                  # PolicySnapshot + strict/hardened provenance mapping
    decision.rs                # decision-card conversion and evidence caps
  app/
    mod.rs
    state.rs                   # FtuiAppState and view routing
    selectors.rs               # packet selection and filter logic
  render/
    mod.rs
    plain.rs                   # deterministic plain-text renderers (baseline)
    tui.rs                     # interactive backend adapters (future)
  bin/
    fp-frankentui-cli.rs       # CLI entrypoint
```

### 3.1 Boundary rationale

| Module | Owns | Must not own |
|---|---|---|
| `ingest::*` | disk IO, deserialization, malformed-line tolerance | UI state, layout logic, terminal backend details |
| `domain::*` | normalized snapshots and policy projections | direct file reads, terminal IO |
| `app::*` | navigation state and selectors | deserialization, rendering side effects |
| `render::*` | presentation formatting | artifact loading and policy mutation |
| `bin/*` | CLI argument plumbing and orchestration | contract logic duplication |

### 3.2 Integration seams (trait-level)

Current seam already exists as `FtuiDataSource` (`crates/fp-frankentui/src/lib.rs:225`).
This remains the primary dependency inversion point.

Additional seams to formalize in implementation phase:

```rust
pub trait DriftHistoryReader {
    fn load_drift_history(&self) -> Result<DriftHistorySnapshot, FtuiError>;
}

pub trait ForensicLogReader {
    fn load_forensic_log(&self, path: &Path) -> Result<ForensicLogSnapshot, FtuiError>;
}

pub trait GovernanceReportReader {
    fn load_governance_gate_snapshot(&self) -> Result<Option<GovernanceGateSnapshot>, FtuiError>;
}

pub trait DecisionProjector {
    fn summarize_decision_dashboard(
        &self,
        ledger: &EvidenceLedger,
        policy: RuntimePolicy,
        guard: &ConformalGuard,
        evidence_term_cap: usize,
    ) -> DecisionDashboardSnapshot;
}
```

This split keeps ingestion testable without runtime-policy fixtures and keeps
runtime-policy behavior testable without filesystem fixtures.

---

## 4. Contract-to-Module Mapping

| Contract class | Authoritative source | Target module |
|---|---|---|
| artifact parse contracts | `FRANKENTUI_CONTRACT_TABLE.md` section 2 | `ingest::packets`, `ingest::drift`, `ingest::forensic`, `ingest::governance` |
| strict/hardened policy matrix | `FRANKENTUI_CONTRACT_TABLE.md` sections 6-8 | `domain::policy`, `domain::decision` |
| fail-closed threat handling | `FRANKENTUI_THREAT_MODEL.md` sections 5-7 | `ingest::*` error handling + `render::plain` safety formatting |
| performance sentinels | `FRANKENTUI_CONTRACT_TABLE.md` section 9 | `app::selectors` windowing + `render::*` pagination hooks |
| app navigation contract | `FRANKENTUI_ANCHOR_MAP.md` rule ledger | `app::state` |

---

## 5. Error Taxonomy and Ownership Boundaries

### 5.1 Error ownership

| Error family | Owner | Notes |
|---|---|---|
| filesystem access and parse failures | `ingest::*` via `FtuiError`/`ArtifactIssue` | never panic; downgrade to panel-local issue where possible |
| policy-projection constraints | `domain::policy` | strict/hardened provenance always explicit |
| view navigation bounds | `app::state` | cursor wrap logic with empty-list guards |
| rendering fallback | `render::*` | terminal capability fallback must be fail-closed and deterministic |

### 5.2 Non-negotiable invariants

- read-only semantics: no artifact mutation paths.
- deterministic plain render path remains available for diagnostics and CI output.
- malformed JSONL never crashes entire dashboard.
- strict mode provenance (`fail_closed_unknown_features`) must stay visible.
- evidence-term display remains bounded by explicit cap.

---

## 6. Implementation Sequence (Risk-Minimizing)

This sequence is chosen to unblock downstream beads while minimizing interface
churn.

### Phase 0 (already landed)
- foundation snapshots and `FsFtuiDataSource` ingestion in `lib.rs`
- CLI smoke entrypoint for packet/dashboard/governance/forensic views

### Phase 1 (this bead: architecture lock)
1. Freeze module boundary contract in this document.
2. Keep all existing public types stable (no behavior churn).
3. Establish expected ownership for ingestion/domain/app/render layers.

### Phase 2 (`bd-2gi.28.5` unblocked)
1. Add unit/property tests against frozen seams:
   - malformed JSONL tolerance,
   - packet discovery determinism,
   - strict/hardened provenance projections,
   - evidence-term cap invariants.
2. Add deterministic structured test logging fields (`packet_id`, `case_id`,
   `mode`, `trace_id`, `assertion_path`, `result`).

### Phase 3 (`bd-2gi.28.6` / `bd-2gi.28.7`)
1. Differential/fault-injection coverage for artifact corruption and partial writes.
2. E2E scenario scripts for CLI-driven replay/forensics workflows.

### Phase 4 (`bd-2gi.28.8` / `bd-2gi.28.9`)
1. Profile loop on ingestion/render hotspots with one-lever-at-a-time changes.
2. Final evidence bundle + RaptorQ durability and decode-proof links.

---

## 7. EV + Decision Card for Architecture Choice

### 7.1 Baseline pressure points

Primary hotspots (qualitative baseline):
- large drift history JSONL parsing,
- forensic event stream pagination,
- mismatch corpus drilldown rendering,
- repeated policy projection and card formatting.

### 7.2 Chosen action

Action A: preserve monolithic `lib.rs` and add features in place.
Action B: lock layered module boundaries now, then implement tests/optimizations
against stable seams.

Selected: **Action B**.

Reason: lower expected rework across `28.5`-`28.9`, clearer ownership for threat
controls, and simpler proof obligations for strict/hardened invariants.

### 7.3 EV gate (architecture-level)

Using program formula `EV = (Impact * Confidence * Reuse) / (Effort * AdoptionFriction)`:
- Impact = 4.0 (unblocks test/differential/e2e/perf beads)
- Confidence = 0.8 (existing foundation crate already aligned)
- Reuse = 4.0 (seams reused by all downstream FRANKENTUI work)
- Effort = 2.0 (documented boundary lock + incremental refactor path)
- AdoptionFriction = 1.2 (no external API break required)

`EV = (4.0 * 0.8 * 4.0) / (2.0 * 1.2) = 5.33` (passes EV >= 2.0 gate).

---

## 8. Delivery Checklist for `bd-2gi.28.4`

- [x] module boundaries and integration seams explicitly justified by contracts.
- [x] implementation sequence defined to minimize risk/rework.
- [x] ownership/error boundaries captured for ingestion/domain/app/render layers.
- [x] downstream bead handoff points (`28.5+`) explicitly mapped.

---

## Changelog

- **bd-2gi.28.4 (2026-02-15):** Added FRANKENTUI Rust integration plan with
  source-anchored current-state inventory, target module boundary skeleton,
  trait seams, risk-minimizing phased execution, and EV-gated architecture decision.
