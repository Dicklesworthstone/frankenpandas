# FrankenPandas

<div align="center">
  <img src="frankenpandas_illustration.webp" alt="FrankenPandas - Clean-room Rust reimplementation of pandas">
</div>

FrankenPandas is a clean-room Rust reimplementation targeting ABSOLUTE AND TOTAL feature/functionality overlap with pandas as a true drop-in replacement, with semantic fidelity, mathematical rigor, operational safety, and profile-proven performance.

## What Makes This Project Special

Alignment-Aware Columnar Execution (AACE): lazy index-alignment graphs with explicit semantic witness ledgers for every materialization boundary.

This is treated as a core identity constraint, not a best-effort nice-to-have.

## Methodological DNA

This project uses five pervasive disciplines:

1. alien-artifact-coding for decision theory, confidence calibration, and explainability.
2. extreme-software-optimization for profile-first, proof-backed performance work.
3. alien-graveyard for high-EV systems patterns and optimization levers.
4. RaptorQ-everywhere for self-healing durability of long-lived artifacts and state.
5. frankenlibc/frankenfs compatibility-security thinking: strict vs hardened mode separation, fail-closed compatibility gates, and explicit drift ledgers.

## Current State

- project charter docs established
- legacy oracle cloned:
  - /dp/frankenpandas/legacy_pandas_code/pandas
- executable MVP slice landed:
  - strict/hardened runtime policy + evidence ledger
  - optional `asupersync` outcome bridge in `fp-runtime`
  - FTUI-ready galaxy-brain decision cards for transparency surfaces
  - initial `fp-frankentui` foundation crate with read-only phase2c artifact ingestion, drift-history tolerance for malformed JSONL lines, and dashboard snapshot primitives
- Phase-2C conformance packet harness landed:
  - packet-scoped suite execution (`FP-P2C-001` through `FP-P2C-011`)
  - expanded packet coverage now includes concat, missingness/nanops, CSV round-trip, dtype invariants, filter/head, and groupby mean/count/min/max/first/last core slices
  - packet gates from `parity_gate.yaml` with machine-readable gate results
  - mismatch corpus emission (`parity_mismatch_corpus.json`) per packet
- live pandas oracle path landed:
  - python adapter at `crates/fp-conformance/oracle/pandas_oracle.py`
  - fixture mode vs live mode switching in `fp-conformance-cli`
- RaptorQ durability pipeline landed for parity reports:
  - repair-symbol sidecars with symbol digests
  - scrub verification report
  - decode-recovery proof artifacts
- blocking gate automation landed:
  - `fp-conformance-cli --require-green` exits non-zero on parity/gate drift
  - `scripts/phase2c_gate_check.sh` runs packet artifacts + gate enforcement
  - CI workflow executes phase2c gate check and required cargo validations
- drift trend ledger landed:
  - packet run summaries append to `artifacts/phase2c/drift_history.jsonl`
- Round-2 optimization evidence landed:
  - `fp-index::align_union` borrowed-key optimization (no semantic drift)
  - `artifacts/perf/ROUND2_BASELINE.md`
  - `artifacts/perf/ROUND2_OPPORTUNITY_MATRIX.md`
  - `artifacts/perf/ROUND2_ISOMORPHISM_PROOF.md`
  - `artifacts/perf/ROUND2_RECOMMENDATION_CONTRACT.md`
- Round-3 optimization evidence landed:
  - `fp-groupby::groupby_sum` guarded identity-alignment fast path (duplicate-safe)
  - `artifacts/perf/ROUND3_BASELINE.md`
  - `artifacts/perf/ROUND3_OPPORTUNITY_MATRIX.md`
  - `artifacts/perf/ROUND3_ISOMORPHISM_PROOF.md`
  - `artifacts/perf/ROUND3_RECOMMENDATION_CONTRACT.md`
- Round-4 optimization evidence landed:
  - `fp-groupby::groupby_sum` dense Int64 aggregation path with bounded fallback
  - `artifacts/perf/ROUND4_BASELINE.md`
  - `artifacts/perf/ROUND4_OPPORTUNITY_MATRIX.md`
  - `artifacts/perf/ROUND4_ISOMORPHISM_PROOF.md`
  - `artifacts/perf/ROUND4_RECOMMENDATION_CONTRACT.md`
- Round-5 optimization evidence landed:
  - `fp-index::has_duplicates` lazy memoization (`OnceCell`) with labels-only equality guarantee
  - benchmark delta: `0.2906s -> 0.0372s` mean on groupby benchmark command (`~87.2%` faster)
  - `artifacts/perf/ROUND5_BASELINE.md`
  - `artifacts/perf/ROUND5_OPPORTUNITY_MATRIX.md`
  - `artifacts/perf/ROUND5_ISOMORPHISM_PROOF.md`
  - `artifacts/perf/ROUND5_RECOMMENDATION_CONTRACT.md`

## V1 Scope

- DataFrame/Series construction and dtype/null/index semantics; - projection/filter/mask/alignment arithmetic; - groupby and join core families; - CSV and key tabular IO paths.

## Architecture Direction

API -> expression planner -> vectorized kernels -> columnar storage -> IO

## Compatibility and Security Stance

Preserve pandas-observable behavior for scoped APIs, especially alignment rules, dtype coercions, null behavior, and join/groupby output contracts.

Defend against malformed data ingestion, schema confusion, unsafe coercion paths, and state drift between strict and hardened modes.

## Performance and Correctness Bar

Track p50/p95/p99 latency and throughput for filter/groupby/join; enforce memory and allocation budgets on representative datasets.

Maintain deterministic null propagation, NaN handling, dtype promotion, and output ordering contracts for scoped operations.

## Key Documents

- AGENTS.md
- COMPREHENSIVE_SPEC_FOR_FRANKENPANDAS_V1.md
- references/frankensqlite/COMPREHENSIVE_SPEC_FOR_FRANKENSQLITE_V1.md (imported exemplar)

## Next Steps

1. Expand packet families beyond current alignment/join/groupby/filter/IO core slices into full DataFrame-level and IO-error parity matrices.
2. Extend kernel-level p50/p95/p99 baselines from current groupby slice to join/filter workloads and track post-Round-5 bottleneck shift.
3. Add adversarial + fuzz suites for high-risk parse and coercion paths in strict/hardened split.
4. Increase live-oracle coverage and environment reproducibility for deterministic replay.
5. Expand drift-history analysis tooling (alerts/threshold trend summaries).

## Conformance Gate Command

```bash
./scripts/phase2c_gate_check.sh
```

This command regenerates packet artifacts and fails closed if any packet parity report or packet gate is not green.

## Porting Artifact Set

- PLAN_TO_PORT_PANDAS_TO_RUST.md
- EXISTING_PANDAS_STRUCTURE.md
- PROPOSED_ARCHITECTURE.md
- FEATURE_PARITY.md

These four docs are now the canonical porting-to-rust workflow for this repo.
