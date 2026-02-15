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
6. deferment ledger: temporarily deferred behavior slices with mandatory follow-on bead IDs (no permanent scope exclusion),
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

## 18. Current Rust Execution Graph (Source-Anchored, Pass-B)

The implemented execution path is no longer conceptual only; it is observable in crate-to-crate call chains:

1. API surfaces (`Series`, `DataFrame`) trigger alignment and operation dispatch in `fp-frame`:
   - `Series::binary_op_with_policy` aligns indexes, reindexes columns, asks runtime admission policy, then applies vectorized/scalar arithmetic:
     - `crates/fp-frame/src/lib.rs:106`
     - `crates/fp-frame/src/lib.rs:125`
     - `crates/fp-frame/src/lib.rs:131`
2. Index alignment plans come from `fp-index` and are validated before materialization:
   - `crates/fp-index/src/lib.rs:461`
   - `crates/fp-index/src/lib.rs:519`
   - `crates/fp-index/src/lib.rs:548`
3. Column materialization (`reindex_by_positions`) guarantees output length parity with alignment vectors and fills absent slots with dtype-aware missing sentinels:
   - `crates/fp-columnar/src/lib.rs:551`
4. Runtime policy and evidence ledger capture admission decisions for unknown features and join-cardinality stress:
   - `crates/fp-runtime/src/lib.rs:149`
   - `crates/fp-runtime/src/lib.rs:175`
   - `crates/fp-runtime/src/lib.rs:209`
5. Conformance runs execute packet fixtures through the same runtime-aware APIs, classify drift, write parity artifacts, and enforce packet gates:
   - `crates/fp-conformance/src/lib.rs:1920`
   - `crates/fp-conformance/src/lib.rs:260`
   - `crates/fp-conformance/src/lib.rs:846`
6. E2E orchestration writes forensic lifecycle logs, artifact index events, failure digests, and replay commands:
   - `crates/fp-conformance/src/lib.rs:3330`
   - `crates/fp-conformance/src/lib.rs:3491`
   - `crates/fp-conformance/src/lib.rs:3697`

Operational consequence: architectural claims in this document should now be treated as contracts over the concrete call graph above, not as future intent statements.

## 19. Alignment + Index Semantics Ledger (AACE Core Contract)

### 19.1 Canonical alignment mechanics

- `AlignMode::{Inner,Left,Right,Outer}` is explicit and centralized:
  - `crates/fp-index/src/lib.rs:444`
- Outer alignment (`align_union`) uses first-occurrence maps and append-right-unseen behavior:
  - `crates/fp-index/src/lib.rs:519`
- Alignment vector lengths are invariants; invalid vector lengths are rejected:
  - `crates/fp-index/src/lib.rs:548`

### 19.2 Duplicate-label semantics (current scoped behavior)

- Duplicate detection is cached per index:
  - `crates/fp-index/src/lib.rs:170`
- Arithmetic path in strict mode rejects duplicate-label alignment (documented MVP slice behavior):
  - `crates/fp-frame/src/lib.rs:114`
  - `crates/fp-frame/src/lib.rs:120`
- Hardened mode may continue through runtime decision path and ledger recording:
  - `crates/fp-frame/src/lib.rs:115`
  - `crates/fp-runtime/src/lib.rs:175`

### 19.3 Lookup/ordering implications

- `position_map_first` establishes first-match semantics for reindexing and duplicate lookup:
  - `crates/fp-index/src/lib.rs:233`
- Adaptive index lookup path (`binary_search` for sorted int/utf8, linear fallback otherwise) is already present:
  - `crates/fp-index/src/lib.rs:191`
- Set operations (`intersection`, `union_with`, `difference`, `symmetric_difference`) are deterministic and form the reusable index algebra basis:
  - `crates/fp-index/src/lib.rs:328`

### 19.4 Alignment risks to track

| Risk | Why it matters | Current anchor | Required evidence |
|---|---|---|---|
| Duplicate strict-reject coverage gaps | pandas allows broader duplicate-label behaviors than current MVP strict slice | `crates/fp-frame/src/lib.rs:120` | differential fixtures with duplicate arithmetic/join/indexing families |
| First-match-only behavior drift | multi-hit semantics can diverge from expected user mental model in edge joins/reindex | `crates/fp-index/src/lib.rs:233` | explicit duplicate-policy contract rows + replay fixtures |
| Alignment vector integrity regressions | malformed vectors can silently corrupt correspondence | `crates/fp-index/src/lib.rs:548` | invariant tests + gate-level failure classification |

## 20. Join and GroupBy Semantics Ledger (Cardinality, Ordering, Null-Key Policy)

### 20.1 Join execution and cardinality controls

- Join path builds borrowed-key hash maps and estimates output rows before materialization:
  - `crates/fp-join/src/lib.rs:82`
  - `crates/fp-join/src/lib.rs:133`
- Arena-vs-global allocator path selection is budget-driven:
  - `crates/fp-join/src/lib.rs:99`
  - `crates/fp-join/src/lib.rs:103`
- Join rows preserve left-first traversal order for inner/left/outer, then append unmatched right rows in outer:
  - `crates/fp-join/src/lib.rs:198`
  - `crates/fp-join/src/lib.rs:218`
- Key conversion excludes missing/NaN join keys for merge paths:
  - `crates/fp-join/src/lib.rs:341`

### 20.2 GroupBy ordering and null treatment

- Default groupby option is `dropna = true`:
  - `crates/fp-groupby/src/lib.rs:13`
- Aligned fast path bypasses index reconstruction only when indexes already match and are duplicate-free:
  - `crates/fp-groupby/src/lib.rs:96`
- First-seen group order is preserved by explicit `ordering` vector in both global and arena paths:
  - `crates/fp-groupby/src/lib.rs:163`
  - `crates/fp-groupby/src/lib.rs:211`
- Dense-int fast path is bounded by fixed span limit (`65,536`), with fallback when range is too large:
  - `crates/fp-groupby/src/lib.rs:297`
  - `crates/fp-groupby/src/lib.rs:335`

### 20.3 Cardinality and memory risk model (current)

| Zone | Strict mode | Hardened mode | Primary evidence path |
|---|---|---|---|
| Alignment/join admission | no hard cap by default (`hardened_join_row_cap: None`) | optional cap + forced `Repair` when exceeded | `crates/fp-runtime/src/lib.rs:162`, `crates/fp-runtime/src/lib.rs:245` |
| Join output growth | estimator-only guard; materializer still executes computed path | same estimator, but policy may redirect action | `crates/fp-join/src/lib.rs:133`, `crates/fp-runtime/src/lib.rs:209` |
| GroupBy span explosions | dense path disabled when span too large; map path fallback | same | `crates/fp-groupby/src/lib.rs:335` |

## 21. DType / Missingness / NaN / NaT Semantics Ledger

### 21.1 Scalar model and missingness taxonomy

- `NullKind` distinguishes `Null`, `NaN`, `NaT` explicitly:
  - `crates/fp-types/src/lib.rs:18`
- `Scalar::is_missing` treats both explicit null variants and IEEE NaN as missing:
  - `crates/fp-types/src/lib.rs:47`
- `semantic_eq` intentionally collapses NaN-vs-NaN equivalence for compatibility semantics:
  - `crates/fp-types/src/lib.rs:70`

### 21.2 Coercion and cast boundaries

- `common_dtype` codifies supported promotion graph and rejects unsupported pairs:
  - `crates/fp-types/src/lib.rs:130`
- Cast failures are typed and explicit (`InvalidCast`, `LossyFloatToInt`, bool-specific failures):
  - `crates/fp-types/src/lib.rs:112`
- Missing casts preserve missingness through dtype-specific sentinel generation:
  - `crates/fp-types/src/lib.rs:158`

### 21.3 Column arithmetic missingness propagation

- Reindex fills with `missing_for_dtype` for absent positions:
  - `crates/fp-columnar/src/lib.rs:551`
- Arithmetic fast paths preserve NaN-vs-Null distinction in invalid lanes:
  - `crates/fp-columnar/src/lib.rs:589`
  - `crates/fp-columnar/src/lib.rs:605`
- Scalar fallback also branches NaN propagation separately from generic null:
  - `crates/fp-columnar/src/lib.rs:694`

### 21.4 Nanops expectations

- `nansum` returns `0.0` for all-missing inputs (intentional behavior contract for current scope):
  - `crates/fp-types/src/lib.rs:254`
- `nanmean`, `nanmin`, `nanmax`, `nanmedian`, `nanvar` return `Null(NaN)` when no valid numeric sample remains:
  - `crates/fp-types/src/lib.rs:262`
  - `crates/fp-types/src/lib.rs:276`
  - `crates/fp-types/src/lib.rs:284`
  - `crates/fp-types/src/lib.rs:292`
  - `crates/fp-types/src/lib.rs:306`

## 22. IO and Malformed-Input Semantics Ledger

### 22.1 CSV ingress/egress contracts

- Missing headers are hard failures:
  - `crates/fp-io/src/lib.rs:15`
  - `crates/fp-io/src/lib.rs:66`
- Parse policy (in `parse_scalar` and `parse_scalar_with_na`) is deterministic and precedence-ordered:
  1. empty/NA marker -> null
  2. int parse
  3. float parse
  4. bool parse
  5. utf8 fallback
  - `crates/fp-io/src/lib.rs:121`
  - `crates/fp-io/src/lib.rs:156`
- NaN write path emits empty CSV cell for round-trippable missing semantics:
  - `crates/fp-io/src/lib.rs:146`
- Optional `index_col` extraction rewires index deterministically and removes that source column:
  - `crates/fp-io/src/lib.rs:202`

### 22.2 JSON ingress contracts

- Records-orient requires array-of-objects and fails closed on shape violations:
  - `crates/fp-io/src/lib.rs:291`
  - `crates/fp-io/src/lib.rs:296`
  - `crates/fp-io/src/lib.rs:306`
- Non-finite floats serialize as JSON null, preserving compatibility-safe output:
  - `crates/fp-io/src/lib.rs:280`

### 22.3 IO risk notes (must remain explicit)

| Risk | Anchor | Follow-up evidence requirement |
|---|---|---|
| No explicit parser row-size/file-size hard budget in read loops | `crates/fp-io/src/lib.rs:79`, `crates/fp-io/src/lib.rs:193` | adversarial oversized CSV fixtures + bounded-ingest policy decision |
| Bool casing compatibility drift (`parse::<bool>()` semantics) | `crates/fp-io/src/lib.rs:133` | pandas-oracle differential on mixed-case boolean CSV tokens |
| `index_col` error type currently routed through `JsonFormat` variant | `crates/fp-io/src/lib.rs:207` | error-taxonomy cleanup + compatibility message fixture checks |

Related compatibility/security anchors:
- `artifacts/phase2c/DOC_ERROR_TAXONOMY.md`
- `artifacts/phase2c/FP-P2C-008/contract_table.md`
- `artifacts/phase2c/DOC_SECURITY_COMPAT_EDGE_CASES.md`

## 23. Runtime Policy + Conformance + Forensics Pipeline (Strict/Hardened)

### 23.1 Runtime decision layer

- Policy defaults to strict (`RuntimePolicy::default -> strict()`):
  - `crates/fp-runtime/src/lib.rs:255`
- Unknown features are force-rejected when fail-closed flag is enabled:
  - `crates/fp-runtime/src/lib.rs:201`
- Hardened mode can force `Repair` for cap-overflow join admissions:
  - `crates/fp-runtime/src/lib.rs:245`
- Decision records include posterior metrics and expected losses for all actions:
  - `crates/fp-runtime/src/lib.rs:292`

### 23.2 Differential and gate enforcement layer

- Differential result taxonomy splits by comparison category + drift severity:
  - `crates/fp-conformance/src/lib.rs:274`
  - `crates/fp-conformance/src/lib.rs:290`
- Packet gate checks enforce strict/hardened budgets and allowlist validation:
  - `crates/fp-conformance/src/lib.rs:914`
  - `crates/fp-conformance/src/lib.rs:926`
  - `crates/fp-conformance/src/lib.rs:932`
- Artifact writer persists parity report, raptorq sidecar, decode proof, gate result, mismatch corpus:
  - `crates/fp-conformance/src/lib.rs:836`

### 23.3 Forensics/replay layer

- E2E lifecycle events are structured and timestamped:
  - `crates/fp-conformance/src/lib.rs:3330`
  - `crates/fp-conformance/src/lib.rs:3383`
- E2E run phases are explicit (run -> write -> enforce -> history):
  - `crates/fp-conformance/src/lib.rs:3493`
- Failure digest includes deterministic replay command and mismatch artifact pointer:
  - `crates/fp-conformance/src/lib.rs:3697`
  - `crates/fp-conformance/src/lib.rs:3801`

### 23.4 Known ambiguity zones (explicitly retained for now)

1. Tie-breaking in expected-loss comparison defaults to `Allow` unless strictly beaten:
   - `crates/fp-runtime/src/lib.rs:299`
2. Runtime timestamp fallback uses `unwrap_or_default` and can emit `0` sentinel:
   - `crates/fp-runtime/src/lib.rs:311`
3. E2E retrospective case timing currently records `elapsed_us: 0`:
   - `crates/fp-conformance/src/lib.rs:3540`

These remain documented as non-ignored compatibility/security risk notes and must stay in the threat/edge-case ledgers until resolved.

## 24. Test + Logging Crosswalk Integration (Pass-B)

Phase-2C execution for this document is now explicitly tied to the logging and error-taxonomy contracts:

- runtime/conformance field expectations and replay mapping:
  - `artifacts/phase2c/DOC_TEST_E2E_LOGGING_CROSSWALK.md`
- failure class and remediation semantics:
  - `artifacts/phase2c/DOC_ERROR_TAXONOMY.md`
- strict/hardened boundary risk zones:
  - `artifacts/phase2c/DOC_SECURITY_COMPAT_EDGE_CASES.md`
- program sign-off and parity artifacts:
  - `artifacts/phase2c/COMPAT_CLOSURE_CONTRACT_TABLE.md`

Mandatory per-packet evidence mapping in this pass:

| Contract surface | Must exist | Where validated |
|---|---|---|
| Unit/property traces with deterministic identifiers | structured logs include packet/case/mode/trace/replay hooks | `DOC_TEST_E2E_LOGGING_CROSSWALK.md` |
| Differential drift records and mismatch corpus | per-case drift category + severity + mismatch bundle | `crates/fp-conformance/src/lib.rs:290`, `crates/fp-conformance/src/lib.rs:826` |
| Gate outcomes with strict/hardened split | packet gate result + reasons | `crates/fp-conformance/src/lib.rs:846` |
| Forensic replay output | failure digest replay command + artifact path | `crates/fp-conformance/src/lib.rs:3697` |
| RaptorQ durability envelope | sidecar + decode proof for durable artifacts | `crates/fp-conformance/src/lib.rs:950`, `crates/fp-runtime/src/lib.rs:327` |

## 25. Pass-B Delta Summary (What This Expansion Adds)

This pass materially upgrades the document from high-level planning notes into a source-anchored operational ledger by adding:

1. concrete crate-level execution graph derived from current code,
2. explicit alignment/index/cardinality/nullness behavior anchors,
3. strict-vs-hardened runtime policy boundaries tied to gate enforcement,
4. IO malformed-input semantics and compatibility risk ledgers,
5. deterministic replay/forensics crosswalk linkage to phase2c artifacts.

Remaining follow-on work after this pass should focus on breadth expansion of fixture coverage (especially duplicate-label edges, CSV adversarial envelopes, and timing fidelity fields), not on redefining the core contracts above.

## 26. Red-Team Contradiction Ledger (Pass-12 Handshake)

This section captures contradiction findings discovered during independent review (`bd-2gi.23.13`) and how they were resolved.

| Finding ID | Contradiction | Prior state | Resolution in current docs |
|---|---|---|---|
| `RTL-01` | V1 exclusion language contradicts total-parity doctrine | `EXISTING_PANDAS_STRUCTURE.md` contained explicit "Exclude for V1" scope language | replaced with no-exclusion phased sequencing doctrine; deferment allowed only with explicit closure bead |
| `RTL-02` | "out-of-scope" wording in packet payload contract could be misread as permanent narrowing | section 13 item 6 used "exclusion ledger" wording | rewritten to "deferment ledger" with mandatory follow-on bead traceability |
| `RTL-03` | Legacy-oracle path wording inconsistent between mount aliases | mixed `/dp/...` vs workspace-relative paths | structure doc now states canonical workspace path and alias equivalence requirement |

Open bounded uncertainties retained intentionally:
1. runtime tie-break defaults and zero-timestamp sentinel behavior (`crates/fp-runtime/src/lib.rs:299`, `crates/fp-runtime/src/lib.rs:311`)
2. retrospective E2E case timing field currently emitted as zero (`crates/fp-conformance/src/lib.rs:3540`)
3. CSV parser resource-envelope hard bounds pending dedicated ingest-budget implementation (`crates/fp-io/src/lib.rs:79`, `crates/fp-io/src/lib.rs:193`)

These uncertainties are explicitly tracked, not tolerated silently, and remain release-gate relevant via the threat/edge-case and drift-ledger artifacts.

## 27. Behavior-Specialist Deep Pass (Deterministic Contract Tables)

This section resolves behavior-level ambiguity by binding execution semantics to explicit tables and source anchors.

### 27.1 Alignment Behavior Table

| Case | Expected behavior | Determinism guard | Anchor |
|---|---|---|---|
| `align_inner` | output index includes only labels present in both sides; first-match positions | output vectors must match index length | `crates/fp-index/src/lib.rs:477`, `crates/fp-index/src/lib.rs:548` |
| `align_left` | preserves left index order exactly; missing right labels map to `None` | left positions always populated | `crates/fp-index/src/lib.rs:500` |
| `align_right` | mirrors left logic with swapped vectors | conversion from left-plan is explicit | `crates/fp-index/src/lib.rs:465` |
| `align_union` / arithmetic default | union index = left labels + right-only labels in encounter order | vector length validation before materialization | `crates/fp-index/src/lib.rs:519`, `crates/fp-frame/src/lib.rs:125` |
| `Series::reindex` | target labels resolved by first-position map; missing slots filled by dtype-missing sentinel | map lookup is deterministic and stable | `crates/fp-frame/src/lib.rs:266`, `crates/fp-columnar/src/lib.rs:551` |

### 27.2 Join Cardinality Contract Table

| Join type | Output row count rule (current) | Duplicate-key handling | Anchor |
|---|---|---|---|
| inner | sum over left labels of number of right matches | multiplicative expansion over matched vectors | `crates/fp-join/src/lib.rs:133`, `crates/fp-join/src/lib.rs:202` |
| left | same as inner + one row for unmatched left labels | unmatched right side uses `None` | `crates/fp-join/src/lib.rs:146`, `crates/fp-join/src/lib.rs:211` |
| right | symmetric with right traversal and left map | unmatched left side uses `None` | `crates/fp-join/src/lib.rs:153`, `crates/fp-join/src/lib.rs:230` |
| outer | left-emitted rows + unmatched right labels appended | preserves left traversal then right-unmatched append | `crates/fp-join/src/lib.rs:165`, `crates/fp-join/src/lib.rs:218` |

Strict/hardened admission note:
- join materialization path consults runtime admission with estimated row count.
- strict has no default cap, hardened can force repair when above cap.
- anchors: `crates/fp-runtime/src/lib.rs:162`, `crates/fp-runtime/src/lib.rs:245`.

### 27.3 GroupBy Behavior Table

| Scenario | Expected behavior | Anchor |
|---|---|---|
| default options | `dropna=true` (null keys skipped) | `crates/fp-groupby/src/lib.rs:13` |
| pre-aggregation alignment | if indexes differ, keys/values are union-aligned first | `crates/fp-groupby/src/lib.rs:96` |
| output ordering | first-seen key order is preserved through explicit ordering vector | `crates/fp-groupby/src/lib.rs:163`, `crates/fp-groupby/src/lib.rs:211` |
| dense-int fast path | enabled only when key span is positive and <= `65_536` | `crates/fp-groupby/src/lib.rs:297`, `crates/fp-groupby/src/lib.rs:335` |
| missing group values | missing value entries do not contribute to numeric sum | `crates/fp-groupby/src/lib.rs:181`, `crates/fp-groupby/src/lib.rs:229` |

### 27.4 Missingness and Numeric Semantics Table

| Operation | All-missing behavior | Mixed missing behavior | Anchor |
|---|---|---|---|
| `Scalar::is_missing` | true for `Null(*)` and IEEE `NaN` floats | non-missing scalars false | `crates/fp-types/src/lib.rs:47` |
| `common_dtype` | null + dtype resolves to dtype | incompatible pairs hard error | `crates/fp-types/src/lib.rs:130` |
| column arithmetic | missing lanes propagate missing output; NaN lanes preserve NaN-class missingness | scalar fallback mirrors vectorized rules | `crates/fp-columnar/src/lib.rs:589`, `crates/fp-columnar/src/lib.rs:694` |
| `nansum` | returns `0.0` | sums finite values only | `crates/fp-types/src/lib.rs:254` |
| `nanmean`/`nanmin`/`nanmax`/`nanmedian`/`nanvar` | returns `Null(NaN)` | computed over finite subset | `crates/fp-types/src/lib.rs:262`, `crates/fp-types/src/lib.rs:276`, `crates/fp-types/src/lib.rs:292`, `crates/fp-types/src/lib.rs:306` |

### 27.5 Strict vs Hardened Boundary Table

| Decision point | Strict behavior | Hardened behavior | Anchor |
|---|---|---|---|
| unknown feature | force reject (`fail_closed_unknown_features=true`) | Bayesian action without strict override unless separate policy rule applies | `crates/fp-runtime/src/lib.rs:158`, `crates/fp-runtime/src/lib.rs:201` |
| join admission over cap | no default cap (unless policy changed) | can force `Repair` when `estimated_rows > cap` | `crates/fp-runtime/src/lib.rs:162`, `crates/fp-runtime/src/lib.rs:245` |
| parity gate evaluation | zero critical drift budget and strict failure thresholds enforced | hardened divergence budget + allowlist constraints | `crates/fp-conformance/src/lib.rs:914`, `crates/fp-conformance/src/lib.rs:926`, `crates/fp-conformance/src/lib.rs:932` |

### 27.6 Ambiguity Resolution Rules (Behavior Pass)

1. If a behavior is implemented but not tabled, it is considered undocumented and cannot be treated as stable.
2. If a table row conflicts with a gate result, gate evidence supersedes prose and the row must be revised.
3. Temporary deferments are valid only with explicit follow-on bead IDs and replay evidence expectations.
4. Any strict/hardened divergence must be observable in decision logs or gate artifacts; implicit divergence is treated as regression risk.

## 28. Risk/Perf/Test Specialist Deep Pass (Pass-C)

### 28.1 Risk Priority and Mitigation Matrix

| Priority | Risk class | Trigger signal | Mandatory mitigation | Evidence artifact |
|---|---|---|---|---|
| `P0` | cardinality blowup / memory exhaustion (join/groupby/ingest) | estimator exceeds budget or runaway fixture growth | bounded admission policy + adversarial fixture suite + fail-closed gate | gate result + mismatch corpus + drift history row |
| `P0` | silent strict/hardened divergence | hardened pass with unexplained strict mismatch class | allowlist-scoped divergence only, otherwise hard fail | packet gate reasons + decision ledger |
| `P1` | dtype/coercion drift | unexpected cast behavior vs oracle | explicit typed error mapping + differential category tagging | drift taxonomy records (`type`, `value`, `nullness`) |
| `P1` | replay incompleteness | failure case without deterministic replay command | failure digest generation and artifact pointer enforcement | forensics report + replay command |
| `P2` | perf regression without semantic drift | p95/p99 exceeds envelope post-change | one-lever optimization loop + rollback trigger | baseline/re-baseline delta report |
| `P3` | observability gaps (missing latency, sparse context) | logs omit key fields or use placeholder values | schema-complete structured logs and gap ledger tracking | logging crosswalk conformance report |

Primary anchors:
- runtime policy decisions: `crates/fp-runtime/src/lib.rs:175`, `crates/fp-runtime/src/lib.rs:209`
- gate enforcement: `crates/fp-conformance/src/lib.rs:846`
- forensic replay: `crates/fp-conformance/src/lib.rs:3697`

### 28.2 Performance Proof Obligations (Non-Optional)

Each optimization-related change must provide all of:

1. baseline snapshot (`p50/p95/p99`, throughput, peak memory/allocation),
2. hotspot profile evidence tied to code path,
3. exactly one optimization lever per step,
4. behavior isomorphism proof (ordering, nullness, dtype, tie-breaking),
5. re-baseline delta with explicit pass/fail threshold,
6. rollback trigger command and fallback behavior.

Current workload anchors to preserve:
- alignment-heavy arithmetic paths (`fp-frame` + `fp-index`)
- skew-sensitive join paths (`fp-join`)
- dense and sparse groupby paths (`fp-groupby`)
- null-dense reduction paths (`fp-types`/`fp-columnar`)

### 28.3 Unit/Property/Differential/E2E Mapping Table

| Contract family | Unit/property minimum | Differential minimum | E2E/logging minimum | Gap status |
|---|---|---|---|---|
| alignment/index | duplicate-label, first-match, vector-length invariants | category-tagged index mismatch checks | case-level logs with mode + evidence count | partial (expanded, still needs broader duplicate families) |
| join cardinality | duplicate-key multiplicity + unmatched-row semantics | strict/hardened drift split across join types | gate + mismatch corpus + replay command | partial (needs adversarial scale envelopes) |
| groupby ordering/null keys | dropna, ordering, dense-fast-path fallback | ordering/nullness mismatch classes | deterministic case events and packet summaries | partial (latency field still retrospective zero) |
| dtype/null/NaN/NaT | cast error typing + nanops branch coverage | nullness/type/value drift categories | structured logs for coercion and reduction failures | partial (broader live-oracle coverage required) |
| IO malformed inputs | parser failure classes and schema drift paths | fixture/live-oracle parse mismatch capture | forensics artifact pointer + replay command | partial (resource-envelope budgets pending) |

### 28.4 Explicit Unresolved Gap Ledger (Bounded)

| Gap ID | Current state | Risk if ignored | Bound/next action |
|---|---|---|---|
| `GAP-LAT-001` | `CaseEnd.elapsed_us` is retrospective `0` in run-level emission | weak timing forensics and perf diagnosis | instrument per-case elapsed timings in run loop |
| `GAP-IO-002` | CSV read loop lacks explicit max-row/max-bytes enforcement | ingest-based resource exhaustion risk | introduce bounded ingest options + adversarial tests |
| `GAP-ORC-003` | live-oracle breadth does not yet match target API envelope | under-detected behavior drift | expand packet families and oracle-mode coverage |
| `GAP-DUP-004` | duplicate-label edge corpus still thin vs target breadth | hidden alignment/indexing divergence risk | add dedicated duplicate-index packet fixtures |

### 28.5 Acceptance Gate for This Pass

This pass is considered valid only when:
1. risk classes are prioritized and mapped to concrete mitigations,
2. performance obligations are explicit and testable,
3. test/e2e/logging mappings include explicit gap status,
4. unresolved gaps are bounded with deterministic next actions.

## 29. Final Integrated Sign-Off (Doc-Pass-13)

Sign-off date: 2026-02-15  
Sign-off bead: `bd-2gi.23.14`

Integration result:
1. base expansion pass complete (`sections 18-25`),
2. contradiction/completeness pass integrated (`section 26`),
3. behavior specialist annex integrated (`section 27`),
4. risk/perf/test specialist annex integrated (`section 28`).

Consistency checks completed:
- no active \"Exclude for V1\" execution guidance remains in the canonical structure document,
- packet payload contract uses deferment wording (not permanent exclusion wording),
- structural claims in companion doc bind to behavior/risk/test annexes,
- bounded unresolved gaps are explicit and traceable.

Remaining open items are only bounded gaps with explicit next actions (section 28.4), not unresolved contradictions.
