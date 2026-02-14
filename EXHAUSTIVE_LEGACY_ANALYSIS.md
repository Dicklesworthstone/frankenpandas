# EXHAUSTIVE_LEGACY_ANALYSIS.md — FrankenPandas

Date: 2026-02-13  
Method stack: `$porting-to-rust` Phase-2 Deep Extraction + `$alien-artifact-coding` + `$extreme-software-optimization` + RaptorQ durability + frankenlibc/frankenfs strict/hardened doctrine.

## 0. Mission and Completion Criteria

This document is the Phase-2 extraction control plane for FrankenPandas. It is complete only when each scoped subsystem has:
1. explicit invariants,
2. explicit crate ownership,
3. explicit oracle test families,
4. explicit security/compatibility mode behavior,
5. explicit performance + artifact gates.

## 1. Source-of-Truth Crosswalk

Legacy corpus:
- `/data/projects/frankenpandas/legacy_pandas_code/pandas`
- Upstream oracle: `pandas-dev/pandas`

Project contracts:
- `/data/projects/frankenpandas/COMPREHENSIVE_SPEC_FOR_FRANKENPANDAS_V1.md` (sections 14-21 are operationally binding)
- `/data/projects/frankenpandas/EXISTING_PANDAS_STRUCTURE.md`
- `/data/projects/frankenpandas/PLAN_TO_PORT_PANDAS_TO_RUST.md`
- `/data/projects/frankenpandas/PROPOSED_ARCHITECTURE.md`
- `/data/projects/frankenpandas/FEATURE_PARITY.md`

## 2. Quantitative Legacy Inventory (Measured)

- Total files: `2652`
- Python: `1507`
- Cython: `pyx=41`, `pxd=23`
- C/C headers: `c=12`, `h=12`
- Test-like files: `1620`

High-density zones:
- `pandas/tests/io` (614 files)
- `pandas/tests/indexes` (191)
- `pandas/tests/frame` (119)
- `pandas/tests/series` (105)
- `pandas/tests/arrays` (101)
- `pandas/_libs/tslibs` (48)

## 3. Subsystem Extraction Matrix (Legacy -> Rust)

| Legacy locus | Non-negotiable behavior to preserve | Target crates | Primary oracles | Phase-2 extraction deliverables |
|---|---|---|---|---|
| `pandas/core/frame.py`, `pandas/core/series.py` | label-aligned construction, assignment, arithmetic | `fp-frame`, `fp-types`, `fp-columnar` | `pandas/tests/frame/*`, `pandas/tests/series/*` | struct+method inventory, default-value matrix, edge-case table |
| `pandas/core/indexes/base.py` | hashability, duplicate-label rules, deterministic indexer behavior | `fp-index`, `fp-types` | `pandas/tests/indexes/*` | index law catalog, ordering+lookup invariants, error-surface map |
| `pandas/core/internals/*` | BlockManager axis/block mapping integrity | `fp-columnar` | `pandas/tests/internals/*` + constructor/indexing suites | storage layout model, blknos/blklocs invariant proofs |
| `pandas/core/indexing.py`, `core/indexers/*` | `loc`/`iloc` semantic split | `fp-frame`, `fp-index` | `pandas/tests/indexing/*` | path-by-path decision table, missing-label behavior fixtures |
| `pandas/core/groupby/*`, `_libs/groupby.pyx` | key grouping, aggregate ordering/default semantics | `fp-groupby`, `fp-expr` | `pandas/tests/groupby/*` | aggregation contract table, null-key behavior matrix |
| `pandas/core/reshape/*`, `_libs/join.pyx` | join cardinality, duplicate-key and null-key behavior | `fp-join` | `pandas/tests/reshape/*`, `tests/reshape/merge/*` | join-plan parity matrix, cardinality witness corpus |
| `pandas/core/missing.py`, `core/nanops.py`, `_libs/missing.pyx` | `NA`/`NaN`/`NaT` propagation | `fp-types`, `fp-expr` | `pandas/tests/arrays/*`, `tests/scalar/*` | null propagation truth tables by dtype family |
| `pandas/io/*` | parser behavior, schema mapping, round-trip stability | `fp-io` | `pandas/tests/io/*` | parse-state machine notes, malformed-input fixture corpus |
| `pandas/_libs/tslibs/*` | datetime/timezone and `NaT` semantics | `fp-types`, `fp-expr` | `pandas/tests/tslibs/*`, `tests/tseries/*` | temporal semantic ledger, timezone edge-case set |

## 4. Alien-Artifact Invariant Ledger (Formal Obligations)

- `FP-I1` Alignment homomorphism: label-domain transforms preserve semantic correspondence under binary ops.
- `FP-I2` Missingness monotonicity: missing markers cannot be silently dropped by transform composition.
- `FP-I3` Join cardinality integrity: output cardinality matches declared join semantics for each key multiplicity regime.
- `FP-I4` Index determinism: identical inputs and mode produce identical index ordering and lookup outputs.
- `FP-I5` Temporal sentinel safety: `NaT` behavior remains closed under scoped datetime operations.

Required proof artifacts per implemented slice:
1. invariant statement,
2. executable conformance witness fixtures,
3. counterexample ledger (if violated),
4. remediation and replay evidence.

## 5. Native/Cython Boundary Register

| Boundary | Files | Risk | Mandatory mitigation |
|---|---|---|---|
| hash/group kernels | `pandas/_libs/algos.pyx`, `hashtable.pyx`, `groupby.pyx` | high | differential fixtures before optimization |
| join/reshape kernels | `pandas/_libs/join.pyx`, `reshape.pyx` | high | join cardinality witness suite |
| missing/data cleaning kernels | `pandas/_libs/missing.pyx`, `ops.pyx` | high | null truth-table parity checks |
| datetime/timezone kernels | `pandas/_libs/tslibs/*` | critical | `NaT` and timezone edge-case corpus |

## 6. Compatibility and Security Doctrine (Mode-Split)

Decision law (runtime):
`mode + input_contract + risk_score + budget -> allow | full_validate | fail_closed`

| Threat | Strict mode | Hardened mode | Required ledger artifact |
|---|---|---|---|
| malformed schema/CSV payload | fail-closed | fail-closed with bounded diagnostics | parser incident ledger |
| coercion abuse | reject unscoped coercions | allow only policy-allowlisted coercions | coercion decision ledger |
| index poisoning payload | reject | quarantine+reject | index validation report |
| join explosion abuse | execute as-specified | admission guard + bounded cap policy | admission decision log |
| unknown incompatible metadata | fail-closed | fail-closed | compatibility drift report |

## 7. Conformance Program (Exhaustive First Wave)

### 7.1 Fixture families (mandatory before substantive optimization)

1. Frame construction/alignment fixtures (`tests/frame`, `tests/series`)
2. Index lookup/slicing/dup-label fixtures (`tests/indexes`, `tests/indexing`)
3. GroupBy aggregate and ordering fixtures (`tests/groupby`)
4. Join/reshape cardinality fixtures (`tests/reshape`)
5. Null/NaN/NaT propagation fixtures (`tests/arrays`, `tests/tslibs`, `tests/tseries`)
6. IO malformed + round-trip fixtures (`tests/io`)

### 7.2 Differential harness outputs (fp-conformance)

Each run emits:
- machine-readable parity report,
- mismatch class histogram,
- minimized repro fixture bundle,
- strict vs hardened divergence report.

Release gate rule: any critical family drift => hard fail.

## 8. Extreme Optimization Program

Primary hotspots:
- alignment arithmetic
- groupby reductions
- hash joins
- CSV parse + frame build

Budgets (from spec section 17):
- alignment arithmetic p95 <= 220 ms
- groupby p95 <= 350 ms
- hash join p95 <= 420 ms
- CSV throughput >= 250 MB/s
- p99 regression <= +7%, peak RSS regression <= +8%

Optimization governance:
1. baseline,
2. profile,
3. one lever,
4. conformance parity proof,
5. budget check,
6. evidence commit.

## 9. RaptorQ-Everywhere Artifact Contract

Durable artifacts requiring RaptorQ sidecars:
- conformance fixture bundles,
- benchmark baselines,
- compatibility ledgers,
- risk and proof ledgers.

Required envelope fields:
- source hash,
- symbol manifest,
- scrub status,
- decode proof chain for every recovery event.

## 10. Phase-2 Execution Backlog (Concrete)

1. Extract `frame.py` constructor and setitem decision tables.
2. Extract `series.py` alignment and arithmetic dispatch tables.
3. Extract `indexes/base.py` indexer/error semantics.
4. Extract `internals/managers.py` block-axis invariants.
5. Extract `indexing.py` loc/iloc branch matrix.
6. Extract `groupby` default-option semantics (`dropna`, `observed`, ordering).
7. Extract join/reshape cardinality semantics from `core/reshape/*` + `_libs/join.pyx`.
8. Extract null propagation matrix from `missing.py` + `nanops.py`.
9. Build initial differential fixture pack for sections 1-8.
10. Implement mismatch classifier taxonomy in `fp-conformance`.
11. Add strict/hardened mode divergence report output.
12. Add RaptorQ sidecar generation and decode-proof verification to conformance artifacts.

Definition of done for Phase-2:
- every row in section 3 has extraction artifacts,
- first six fixture families are runnable,
- G1-G6 gate definitions from comprehensive spec are traceable to concrete harness outputs.

## 11. Residual Gaps and Risks

- `PROPOSED_ARCHITECTURE.md` currently embeds literal `\n` in crate-map bullets; normalize to proper markdown before relying on it for automation.
- IO surface is the largest test-density area; extraction must avoid overfitting to a tiny subset.
- Cython boundary areas remain the highest semantic regression risk until differential corpus is broad.

## 12. Deep-Pass Hotspot Inventory (Measured)

Measured from `/data/projects/frankenpandas/legacy_pandas_code/pandas`:
- file count: `2652`
- test-heavy mass: `pandas/tests` (`1599` files)
- highest core concentration: `pandas/core` (`173` files), `pandas/_libs` (`144` files), `pandas/io` (`64` files)

Top source hotspots by line count (first-wave extraction anchors):
1. `pandas/core/frame.py` (`18679`)
2. `pandas/core/generic.py` (`12788`)
3. `pandas/core/series.py` (`9860`)
4. `pandas/core/indexes/base.py` (`8082`)
5. `pandas/_libs/tslibs/offsets.pyx` (`7730`)
6. `pandas/core/groupby/groupby.py` (`6036`)

Interpretation:
- DataFrame/Series/index internals dominate semantic surface area.
- Cython datetime/groupby kernels remain highest compatibility risk.
- IO and formatting are broad but can be staged after semantic core parity.

## 13. Phase-2C Extraction Payload Contract (Per Ticket)

Each `FP-P2C-*` ticket MUST produce all of the following extraction payloads:
1. type inventory: structs/classes + all relevant fields for scoped behavior,
2. rule ledger: branch predicates and default values,
3. error ledger: exact exception type/message-class contracts,
4. null/index ledger: `NA`/`NaN`/`NaT` + index alignment behavior,
5. strict/hardened split: explicit mode divergence policy,
6. exclusion ledger: exactly what is intentionally out-of-scope,
7. fixture mapping: source tests -> normalized fixture ids,
8. compatibility note: expected drift classes (if any),
9. optimization note: hotspot candidate + isomorphism risk,
10. RaptorQ note: artifact set requiring sidecars.

Artifact location (normative):
- `artifacts/phase2c/ESSENCE_EXTRACTION_LEDGER.md` (cross-packet canonical essence ledger for invariants/assumptions/undefined edges/divergence/non-goals)
- `artifacts/phase2c/FP-P2C-00X/legacy_anchor_map.md`
- `artifacts/phase2c/FP-P2C-00X/contract_table.md`
- `artifacts/phase2c/FP-P2C-00X/fixture_manifest.json`
- `artifacts/phase2c/FP-P2C-00X/parity_gate.yaml`
- `artifacts/phase2c/FP-P2C-00X/risk_note.md`

## 14. Strict/Hardened Compatibility Drift Budgets

Release-blocking budgets for Phase-2C packet acceptance:
- strict mode critical drift budget: `0`
- strict mode non-critical drift budget: `<= 0.10%` of packet fixtures
- hardened mode allowed divergence: `<= 1.00%` and only in explicitly allowlisted defensive categories
- unknown incompatibility handling: `fail-closed` in both modes

Every packet report MUST include:
- `strict_summary` (pass/fail + mismatch classes),
- `hardened_summary` (pass/fail + divergence classes),
- `decision_log` entries for every hardened-only repair/deny,
- `compatibility_drift_hash` for reproducibility.

## 15. Extreme-Software-Optimization Execution Law

No optimization may merge without this full loop:
1. baseline capture (`p50/p95/p99`, throughput, peak RSS),
2. hotspot profile,
3. one optimization lever only,
4. fixture parity + invariant replay,
5. re-baseline + delta artifact.

Primary sentinel workloads:
- alignment-heavy arithmetic (`FP-P2C-001`, `FP-P2C-004`)
- high-cardinality groupby (`FP-P2C-005`)
- skewed-key merge/join (`FP-P2C-006`)
- null-dense reductions (`FP-P2C-007`)

Optimization scoring gate (mandatory):
`score = (impact * confidence) / effort`
- implement only if `score >= 2.0`
- otherwise defer and document.

## 16. RaptorQ Evidence Topology and Recovery Drills

All durable Phase-2C evidence artifacts MUST emit sidecars:
- parity reports,
- fixture bundles,
- mismatch corpora,
- benchmark baselines,
- strict/hardened decision ledgers.

Naming convention:
- payload: `packet_<id>_<artifact>.json`
- sidecar: `packet_<id>_<artifact>.raptorq.json`
- proof: `packet_<id>_<artifact>.decode_proof.json`

Scrub requirements:
- pre-merge scrub for touched packet artifacts,
- scheduled scrub in CI for all packet artifacts,
- any decode failure is a hard release blocker.

## 17. Phase-2C Exit Checklist (Operational)

Phase-2C is complete only when all are true:
1. `FP-P2C-001..008` packet artifacts exist and are internally consistent.
2. Every packet has at least one strict-mode fixture family and one hardened-mode adversarial family.
3. Drift budgets in section 14 are met.
4. Optimization evidence exists for at least one hotspot per high-risk packet.
5. RaptorQ sidecars + decode proofs are present and scrub-clean.
6. Residual risks are enumerated with owners and next actions.
