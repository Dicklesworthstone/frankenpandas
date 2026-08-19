# COMPREHENSIVE_SPEC_FOR_FRANKENPANDAS_V1

## 0. Prime Directive

Build a system that is simultaneously:

1. Behaviorally trustworthy for scoped compatibility.
2. Mathematically explicit in decision and risk handling.
3. Operationally resilient via RaptorQ-backed durability.
4. Performance-competitive via profile-and-proof discipline.

Crown-jewel innovation:

Alignment-Aware Columnar Execution (AACE): lazy index-alignment graphs with explicit semantic witness ledgers for every materialization boundary.

Legacy oracle:

- /dp/frankenpandas/legacy_pandas_code/pandas
- upstream: https://github.com/pandas-dev/pandas

Reference exemplar imported into this repository:

- references/frankensqlite/COMPREHENSIVE_SPEC_FOR_FRANKENSQLITE_V1.md

## 1. Product Thesis

Most reimplementations fail by being partially compatible and operationally brittle. FrankenPandas will instead combine compatibility realism with first-principles architecture and strict quality gates.

## 2. V1 Scope Contract

Included in V1:

- DataFrame/Series construction and dtype/null/index semantics; - projection/filter/mask/alignment arithmetic; - groupby and join core families; - CSV and key tabular IO paths.

Deferred from V1:

- long-tail API surface outside highest-value use cases
- broad ecosystem parity not required for core migration value
- distributed/platform expansion not needed for V1 acceptance

## 3. Architecture Blueprint

API -> expression planner -> vectorized kernels -> columnar storage -> IO

Planned crate families:
- fp-types
- fp-columnar
- fp-frame
- fp-expr
- fp-groupby
- fp-join
- fp-io
- fp-conformance
- frankenpandas

## 4. Compatibility Model (frankenlibc/frankenfs-inspired)

Two explicit operating modes:

1. strict mode:
   - maximize observable compatibility for scoped APIs
   - no behavior-altering repair heuristics
2. hardened mode:
   - maintain outward contract while enabling defensive runtime checks and bounded repairs

Compatibility focus for this project:

Preserve pandas-observable behavior for scoped APIs, especially alignment rules, dtype coercions, null behavior, and join/groupby output contracts.

Fail-closed policy:

- unknown incompatible features or protocol fields must fail closed by default
- compatibility exceptions require explicit allowlist entries and audit traces

## 5. Security Model

Security focus for this project:

Defend against malformed data ingestion, schema confusion, unsafe coercion paths, and state drift between strict and hardened modes.

Threat model baseline:

1. malformed input and parser abuse
2. state-machine desynchronization
3. downgrade and compatibility confusion paths
4. persistence corruption and replay tampering

Mandatory controls:

- adversarial fixtures and fuzz/property suites for high-risk entry points
- deterministic audit trail for recoveries and mode/policy overrides
- explicit subsystem ownership and trust-boundary notes

## 6. Alien-Artifact Decision Layer

Runtime controllers (scheduling, adaptation, fallback, admission) must document:

1. state space
2. evidence signals
3. loss matrix with asymmetric costs
4. posterior or confidence update model
5. action rule minimizing expected loss
6. calibration fallback trigger

Output requirements:

- evidence ledger entries for consequential decisions
- calibrated confidence metrics and drift alarms

## 7. Extreme Optimization Contract

Track p50/p95/p99 latency and throughput for filter/groupby/join; enforce memory and allocation budgets on representative datasets.

Optimization loop is mandatory:

1. baseline metrics
2. hotspot profile
3. single-lever optimization
4. behavior-isomorphism proof
5. re-profile and compare

No optimization is accepted without associated correctness evidence.

## 8. Correctness and Conformance Contract

Maintain deterministic null propagation, NaN handling, dtype promotion, and output ordering contracts for scoped operations.

Conformance process:

1. generate canonical fixture corpus
2. run legacy oracle and capture normalized outputs
3. run FrankenPandas and compare under explicit equality/tolerance policy
4. produce machine-readable parity report artifact

Assurance ladder:

- Tier A: unit/integration/golden fixtures
- Tier B: differential conformance
- Tier C: property/fuzz/adversarial tests
- Tier D: regression corpus for historical failures

## 9. RaptorQ-Everywhere Durability Contract

RaptorQ repair-symbol sidecars are required for long-lived project evidence:

1. conformance snapshots
2. benchmark baselines
3. migration manifests
4. reproducibility ledgers
5. release-grade state artifacts

Required artifacts:

- symbol generation manifest
- scrub verification report
- decode proof for each recovery event

## 10. Milestones and Exit Criteria

### M0 — Bootstrap

- workspace skeleton
- CI and quality gate wiring

Exit:
- fmt/check/clippy/test baseline green

### M1 — Core Model

- core data/runtime structures
- first invariant suite

Exit:
- invariant suite green
- first conformance fixtures passing

### M2 — First Vertical Slice

- end-to-end scoped workflow implemented

Exit:
- differential parity for first major API family
- baseline benchmark report published

### M3 — Scope Expansion

- additional V1 API families

Exit:
- expanded parity reports green
- no unresolved critical compatibility defects

### M4 — Hardening

- adversarial coverage and perf hardening

Exit:
- regression gates stable
- conformance drift zero for V1 scope

## 11. Acceptance Gates

Gate A: compatibility parity report passes for V1 scope.

Gate B: security/fuzz/adversarial suite passes for high-risk paths.

Gate C: performance budgets pass with no semantic regressions.

Gate D: RaptorQ durability artifacts validated and scrub-clean.

All four gates must pass for V1 release readiness.

## 12. Risk Register

Primary risk focus:

Silent semantic drift in alignment/null behavior during aggressive performance optimization.

Mitigations:

1. compatibility-first development for risky API families
2. explicit invariants and adversarial tests
3. profile-driven optimization with proof artifacts
4. strict mode/hardened mode separation with audited policy transitions
5. RaptorQ-backed resilience for critical persistent artifacts

## 13. Immediate Execution Checklist

1. Create workspace and crate skeleton.
2. Implement smallest high-value end-to-end path in V1 scope.
3. Stand up differential conformance harness against legacy oracle.
4. Add benchmark baseline generation and regression gating.
5. Add RaptorQ sidecar pipeline for conformance and benchmark artifacts.

## 14. Detailed Crate Contracts (V1)

| Crate | Primary Responsibility | Explicit Non-Goal | Invariants | Mandatory Tests |
|---|---|---|---|---|
| fp-types | dtype, scalar, null, index metadata model | runtime execution | no invalid dtype tags, stable coercion table versioning | dtype matrix, coercion parity, serialization round-trip |
| fp-columnar | contiguous buffers + validity bitmaps | user-facing API | bitmap length = row count, no silent length drift | bitmap fuzz, buffer slicing property tests |
| fp-frame | DataFrame/Series façade and alignment orchestration | low-level kernel tuning | alignment determinism, stable row/column cardinality accounting | index alignment fixtures, null/NaN edge fixtures |
| fp-expr | logical expression DAG and rewrite rules | physical IO | rewrite must be semantics-preserving | rewrite differential tests, canonicalization snapshots |
| fp-groupby | grouping planner and aggregate kernels | join processing | group key determinism, aggregate null contract fidelity | cardinality skew fixtures, aggregate parity suite |
| fp-join | hash/sort-merge join paths | storage formats | join cardinality and null-side semantics preserved | inner/left edge fixtures, duplicate-key stress |
| fp-io | CSV/Parquet/IPC bridges | compute kernels | parser normalization deterministic, schema mapping auditable | malformed input adversarial corpus, round-trip matrix |
| fp-conformance | pandas differential harness | production execution | oracle-vs-target comparison policy explicit per dtype/op | differential runner, report schema tests |
| frankenpandas | binary/library integration and policy loading | algorithm development | strict/hardened mode wiring and evidence ledger always available | mode gate tests, startup policy tests |

## 15. Conformance Matrix (V1)

| Family | Oracle Workload | Pass Criterion | Drift Severity |
|---|---|---|---|
| Construction + dtype inference | constructor fixture corpus from pandas | exact dtype + shape + null parity | critical |
| Index alignment arithmetic | mixed-index arithmetic suite | exact index order + value parity under policy | critical |
| Null and NaN propagation | null-heavy transforms and reductions | parity by documented null/NaN policy | critical |
| GroupBy aggregates | uniform + skew key workloads | aggregate outputs + ordering parity | high |
| Join semantics | inner/left joins with key anomalies | row count and column parity | critical |
| Sort and filter | stable/unstable key distribution fixtures | deterministic ordering + filter parity | high |
| CSV ingest/export | malformed and edge CSV fixtures | parsed frame parity + error parity | high |
| Mixed pipeline E2E | ingestion -> transform -> join -> groupby | end-to-end parity and reproducible report | critical |

## 16. Phase-2C Packet Conformance/Durability Contract

Packetized conformance execution is the mandatory proving surface for scoped parity.

Current packet families:

- `FP-P2C-001`: series arithmetic with index alignment and duplicate-label mode split.
- `FP-P2C-002`: index alignment union, duplicate detection, and first-position semantics.
- `FP-P2C-003`: mixed-label series arithmetic and duplicate-label hardened behavior.
- `FP-P2C-004`: series-level join semantics (`inner`/`left`) with duplicate-key and missing-right behavior.
- `FP-P2C-005`: series-level `groupby_sum` semantics with first-seen key order and dropna alignment behavior.

Required runner capabilities:

1. run by packet id and as grouped packet suites.
2. dual oracle mode:
   - fixture-expected mode for deterministic replay.
   - live pandas oracle mode via project-controlled adapter.
3. packet gate validation from `parity_gate.yaml`.
4. machine-readable mismatch corpus emission for drift replay.
5. enforceable non-zero exit for packet drift via gate enforcement mode.
6. append-only drift trend ledger for packet outcomes.

Required per-packet artifacts:

1. `parity_report.json`
2. `parity_gate_result.json`
3. `parity_mismatch_corpus.json`
4. `parity_report.raptorq.json`
5. `parity_report.decode_proof.json`

Required cross-run artifact:

1. `artifacts/phase2c/drift_history.jsonl` (append-only JSONL packet summaries)

RaptorQ constraints for packet artifacts:

1. sidecar must include source hash, OTI, source/repair packet counts, and symbol digests.
2. scrub verification must assert source-hash and packet consistency.
3. decode drill must drop a subset of source packets and recover payload with proof hash.

Fail-closed rules:

1. unknown oracle operation or schema mismatch must fail the packet.
2. missing gate config or packet id mismatch must fail gate evaluation.
3. strict mode oracle import failure must not silently downgrade behavior.
4. gate-enforcement command must return non-zero when any packet parity/gate check fails.

## 16. Security and Compatibility Threat Matrix

| Threat | Strict Mode Response | Hardened Mode Response | Required Artifact |
|---|---|---|---|
| Schema confusion on ingest | fail with explicit parse/type error | fail + bounded normalization hints | parser incident ledger |
| Type coercion abuse | disallow unsafe coercion path | allow only policy-allowlisted coercions | coercion decision ledger |
| Index poisoning via malformed keys | reject invalid index payloads | quarantine + reject with reason code | index validation report |
| Join explosion input abuse | execute as-specified; may be expensive | admission guard + rate/size cap policy | workload admission log |
| Unknown incompatible file metadata | fail-closed | fail-closed | compatibility drift report |
| Differential harness oracle mismatch | hard fail release gate | hard fail release gate | conformance failure bundle |
| Corrupt artifact manifests | reject load | recover from RaptorQ sidecar if possible | decode proof + scrub report |
| Policy override misuse | explicit override required with audit trail | explicit override required with audit trail | override audit record |

## 17. Performance Budgets and SLO Targets

| Path | Dataset Class | Budget |
|---|---|---|
| alignment arithmetic hot loop | 10M rows mixed index | p95 <= 220 ms |
| groupby sum/mean | 10M rows, skew ratio 0.9 | p95 <= 350 ms |
| hash join inner | 5M x 5M keys | p95 <= 420 ms |
| CSV parse + frame build | 1 GB realistic CSV | throughput >= 250 MB/s |
| null-mask-heavy transforms | 10M rows, 40% null | p95 regression <= +5% vs baseline |
| memory footprint | representative E2E pipeline | peak RSS regression <= +8% |
| allocation churn | groupby and join workloads | alloc count regression <= +10% |
| tail stability | all benchmark families | p99 regression <= +7% |

Optimization acceptance rule:

1. improvement CI excludes zero on primary metric,
2. no critical conformance drift,
3. p99 and memory budgets respected.

## 18. CI Gate Topology (Release-Critical)

| Gate | Name | Blocking | Output Artifact |
|---|---|---|---|
| G1 | format + lint | yes | lint report |
| G2 | unit + integration | yes | junit report |
| G3 | differential conformance | yes | parity report JSON + markdown summary |
| G4 | adversarial + property tests | yes | minimized counterexample corpus |
| G5 | benchmark regression | yes | baseline delta report |
| G6 | RaptorQ scrub + recovery drill | yes | scrub report + decode proof sample |

Release cannot proceed unless all gates pass on the same commit.

## 19. RaptorQ Artifact Envelope (Project-Wide)

Persistent evidence artifacts must be emitted with sidecars:

1. source artifact hash manifest,
2. RaptorQ symbol manifest,
3. scrub status,
4. decode proof log when recovery occurs.

Canonical envelope schema:

~~~json
{
  "artifact_id": "string",
  "artifact_type": "conformance|benchmark|ledger|manifest",
  "source_hash": "blake3:...",
  "raptorq": {
    "k": 0,
    "repair_symbols": 0,
    "overhead_ratio": 0.0,
    "symbol_hashes": ["..."]
  },
  "scrub": {
    "last_ok_unix_ms": 0,
    "status": "ok|recovered|failed"
  },
  "decode_proofs": [
    {
      "ts_unix_ms": 0,
      "reason": "...",
      "recovered_blocks": 0,
      "proof_hash": "blake3:..."
    }
  ]
}
~~~

## 20. 90-Day Execution Plan

Weeks 1-2:
- scaffold workspace and crate boundaries
- lock compatibility contract and fixture schema

Weeks 3-5:
- implement fp-types/fp-columnar/fp-frame minimal vertical slice
- land first strict-mode conformance harness

Weeks 6-8:
- implement groupby/join core paths
- publish first benchmark baselines with budgets

Weeks 9-10:
- harden parser/coercion threat paths and adversarial corpus
- wire strict/hardened policy transitions and audit trails

Weeks 11-12:
- enforce full gate topology G1-G6 in CI
- run release-candidate drill with complete artifact bundle

## 21. Porting Artifact Index

This spec is paired with the following methodology artifacts:

1. PLAN_TO_PORT_PANDAS_TO_RUST.md
2. EXISTING_PANDAS_STRUCTURE.md
3. PROPOSED_ARCHITECTURE.md
4. FEATURE_PARITY.md

Rule of use:

- Extraction and behavior understanding happens in EXISTING_PANDAS_STRUCTURE.md.
- Scope, exclusions, and phase sequencing live in PLAN_TO_PORT_PANDAS_TO_RUST.md.
- Rust crate boundaries live in PROPOSED_ARCHITECTURE.md.
- Delivery readiness is tracked in FEATURE_PARITY.md.
