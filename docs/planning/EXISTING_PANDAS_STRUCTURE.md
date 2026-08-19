# EXISTING_PANDAS_STRUCTURE.md — Red-Team Reviewed Structural Map

Date: 2026-02-15  
Review bead: `bd-2gi.23.13` (independent contradiction/completeness pass)  
Mandate: total pandas feature/functionality parity, clean-room reimplementation, no permanent scope exclusion.

## 0. Review Purpose

This document is the legacy-structure orientation map used by extraction and conformance planning.
For this pass, it is explicitly corrected to remove contradiction-prone shorthand and to align with the full-parity doctrine.

## 1. Legacy Oracle and Path Canonicalization

- canonical workspace path:
  - `/data/projects/frankenpandas/legacy_pandas_code/pandas`
- deployment alias in some docs/shell environments:
  - `/dp/frankenpandas/legacy_pandas_code/pandas`
- upstream behavioral oracle:
  - `https://github.com/pandas-dev/pandas`

Path rule: both local path forms must resolve to the same corpus snapshot when generating fixture anchors and parity evidence.

## 2. Legacy Subsystem Topology (Behavioral Weighting)

| Legacy zone | Core responsibility | Compatibility risk weight | Primary Rust landing zones |
|---|---|---|---|
| `pandas/core/frame.py`, `series.py` | constructor semantics, assignment, arithmetic dispatch, alignment-visible behavior | critical | `fp-frame`, `fp-columnar`, `fp-types`, `fp-runtime` |
| `pandas/core/indexes/*`, `indexing.py`, `core/indexers/*` | label algebra, lookup/indexer contracts, `loc`/`iloc` semantics | critical | `fp-index`, `fp-frame`, `fp-runtime` |
| `pandas/core/internals/*` | storage invariants, axis/block consistency | critical | `fp-columnar`, `fp-frame` |
| `pandas/core/groupby/*`, `core/window/*` | grouping defaults, aggregation ordering, rolling/window edge rules | high | `fp-groupby`, `fp-expr`, `fp-runtime` |
| `pandas/core/reshape/*`, `_libs/join.pyx` | join/merge cardinality and null/duplicate key rules | high | `fp-join`, `fp-index`, `fp-runtime` |
| `pandas/core/missing.py`, `nanops.py`, `_libs/missing.pyx` | NA/NaN/NaT propagation and reduction semantics | high | `fp-types`, `fp-columnar`, `fp-expr` |
| `pandas/io/*` | parser and serializer contracts, malformed-input behavior | high | `fp-io`, `fp-conformance` |
| `pandas/tests/*`, `pandas/_testing/*` | externally visible behavior oracle corpus | critical | `fp-conformance` |

## 3. Compatibility-Critical Behavioral Contracts

1. constructor/index alignment must preserve observable label/value correspondence for all indexed operations.
2. duplicate-label and missing-label behavior must remain deterministic and mode-explicit.
3. arithmetic and comparison ops must route through alignment semantics before kernel application.
4. dtype coercion/promotion must be explicit, reproducible, and fail-closed on unsupported conversions.
5. null/NaN/NaT propagation must preserve deterministic output contracts and reduction edge behavior.
6. join semantics must preserve declared cardinality behavior across inner/left/right/outer and duplicate-key regimes.
7. groupby output ordering and null-key treatment must be explicit and reproducible.
8. IO parsing/writing must preserve schema and missingness semantics while handling malformed inputs with bounded behavior.
9. strict/hardened divergence must be policy-anchored and audit-visible, not incidental.
10. every behavior claim must be backed by differential evidence, not inferred from implementation intent.

## 4. Security and Stability Risk Surfaces

| Surface | Primary risk | Required control/evidence |
|---|---|---|
| parser ingestion (CSV/JSON) | resource exhaustion, malformed payload drift | bounded parser policy + adversarial fixtures + deterministic failure logs |
| coercion paths | silent narrowing or lossy cast drift | typed cast errors + differential mismatch taxonomy |
| join/groupby cardinality | memory blowup under skew/duplicates | estimator + runtime admission policy + gate-enforced drift budgets |
| strict/hardened policy boundary | accidental silent divergence | mode-tagged decision ledgers + allowlisted hardened divergence categories |
| artifact durability | corrupted parity evidence/replay breakage | RaptorQ sidecar + scrub report + decode proof |

## 5. Execution Boundary Doctrine (No Permanent Exclusions)

The prior "V1 include/exclude" framing is removed.

Policy now:
1. no feature family is permanently out-of-scope if pandas exposes it.
2. phased sequencing is allowed, but each deferred slice must carry an explicit follow-on bead ID.
3. deferment does not relax compatibility expectations for implemented surfaces.
4. closure claims are invalid without a claim-to-evidence chain (fixtures, drift report, gates, replay artifacts).

## 6. Conformance Family Matrix (Priority-Ordered)

| Family | Why it is non-negotiable | Legacy anchors | Current Rust touchpoints |
|---|---|---|---|
| frame/series construction and assignment | foundational observable behavior | `pandas/tests/frame/*`, `pandas/tests/series/*` | `fp-frame`, `fp-conformance` fixture ops |
| indexing and index laws | `loc/iloc` and indexer determinism | `pandas/tests/indexing/*`, `pandas/tests/indexes/*` | `fp-index`, `fp-frame`, `fp-conformance` |
| join/reshape | cardinality and key semantics | `pandas/tests/reshape/*` | `fp-join`, `fp-runtime`, `fp-conformance` |
| groupby/window | aggregation defaults/order semantics | `pandas/tests/groupby/*`, `pandas/tests/window/*` | `fp-groupby`, `fp-expr`, `fp-conformance` |
| null/nan/nat and dtype behavior | pervasive semantic contract | `pandas/tests/arrays/*`, `pandas/tests/scalar/*`, `pandas/tests/tslibs/*` | `fp-types`, `fp-columnar`, `fp-conformance` |
| io round-trip and malformed input | ingestion correctness and resilience | `pandas/tests/io/*` | `fp-io`, `fp-conformance` |

## 7. Rust Structural Reality Check (Source-Anchored)

These live-code anchors validate that the current implementation already routes through explicit compatibility primitives:

- alignment plan + validation:
  - `crates/fp-index/src/lib.rs:461`
  - `crates/fp-index/src/lib.rs:548`
- series arithmetic with policy admission:
  - `crates/fp-frame/src/lib.rs:106`
  - `crates/fp-frame/src/lib.rs:131`
- dtype/null/NaN/NaT scalar model:
  - `crates/fp-types/src/lib.rs:18`
  - `crates/fp-types/src/lib.rs:47`
  - `crates/fp-types/src/lib.rs:130`
- join/groupby cardinality-sensitive paths:
  - `crates/fp-join/src/lib.rs:133`
  - `crates/fp-groupby/src/lib.rs:297`
- strict/hardened policy and fail-closed controls:
  - `crates/fp-runtime/src/lib.rs:158`
  - `crates/fp-runtime/src/lib.rs:201`
  - `crates/fp-runtime/src/lib.rs:245`
- differential/gate/forensics evidence pipeline:
  - `crates/fp-conformance/src/lib.rs:260`
  - `crates/fp-conformance/src/lib.rs:846`
  - `crates/fp-conformance/src/lib.rs:3330`

## 8. Red-Team Contradiction Findings (This Pass)

| ID | Finding | Disposition |
|---|---|---|
| `RTS-01` | prior doc had explicit "Exclude for V1" language | removed; replaced with no-permanent-exclusion doctrine |
| `RTS-02` | path alias inconsistency (`/dp` vs workspace path) could break reproducibility assumptions | explicit canonical + alias equivalence rule added |
| `RTS-03` | shallow subsystem bullets lacked risk weighting and Rust ownership map | replaced with weighted subsystem matrix and crate landing zones |
| `RTS-04` | compatibility-critical claims lacked enforcement hooks | added source-anchored runtime/conformance evidence hooks |

## 9. Bounded Uncertainty Ledger

The following are acknowledged as open risk items and must remain explicit until closed by dedicated beads:

1. full `loc/iloc` parity breadth is not yet fully evidenced across all edge contracts.
2. live-oracle coverage breadth still trails full fixture-space aspirations.
3. parser resource-envelope hard bounds need dedicated implementation and validation artifacts.
4. strict/hardened tie-break and timestamp sentinel ambiguities remain documented in the phase2c edge-case ledgers.

## 10. Operational Use

Use this document as:
1. legacy topology source for selecting the next extraction packet,
2. contradiction guard to reject scope-narrowing language,
3. cross-reference index into `EXHAUSTIVE_LEGACY_ANALYSIS.md` and phase2c evidence artifacts.

Companion artifacts for execution:
- `EXHAUSTIVE_LEGACY_ANALYSIS.md`
- `artifacts/phase2c/DOC_ERROR_TAXONOMY.md`
- `artifacts/phase2c/DOC_SECURITY_COMPAT_EDGE_CASES.md`
- `artifacts/phase2c/DOC_TEST_E2E_LOGGING_CROSSWALK.md`

## 11. Behavior Annex (Pass-B Specialist Addendum)

This addendum links structural map claims to deterministic behavior tables maintained in the exhaustive ledger.

Behavior contract index:
1. alignment rules and first-match semantics: `EXHAUSTIVE_LEGACY_ANALYSIS.md` section 27.1
2. join cardinality and duplicate expansion rules: `EXHAUSTIVE_LEGACY_ANALYSIS.md` section 27.2
3. groupby ordering/dropna contracts: `EXHAUSTIVE_LEGACY_ANALYSIS.md` section 27.3
4. null/NaN/NaT and nanops output rules: `EXHAUSTIVE_LEGACY_ANALYSIS.md` section 27.4
5. strict/hardened policy boundaries and gate interactions: `EXHAUSTIVE_LEGACY_ANALYSIS.md` section 27.5

Operational rule:
- structural claims in this file are authoritative only when they remain consistent with the behavior tables and their source anchors.

## 12. Risk/Perf/Test Annex (Pass-C Specialist Addendum)

Cross-document contract bindings:
1. risk priority matrix and mitigations: `EXHAUSTIVE_LEGACY_ANALYSIS.md` section 28.1
2. performance proof obligations and rollback requirements: `EXHAUSTIVE_LEGACY_ANALYSIS.md` section 28.2
3. unit/property/differential/e2e/logging mapping and status: `EXHAUSTIVE_LEGACY_ANALYSIS.md` section 28.3
4. bounded unresolved gap ledger: `EXHAUSTIVE_LEGACY_ANALYSIS.md` section 28.4

Operational rule:
- any new structural claim added to this file must include either a passing evidence mapping row or an explicit bounded gap row in the specialist annex.

## 13. Final Integration Sign-Off

Sign-off date: 2026-02-15  
Integration bead: `bd-2gi.23.14`

Final status:
1. contradiction-prone scope language removed and replaced with no-permanent-exclusion doctrine,
2. behavior annex (section 11) and risk/perf/test annex (section 12) are both present and linked,
3. structural claims are now explicitly tied to source-anchored contracts and bounded-gap reporting rules.
