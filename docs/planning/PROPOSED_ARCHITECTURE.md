# PROPOSED_ARCHITECTURE

## 1. Architecture Principles

1. Spec-first implementation, no line translation.
2. Strict mode for compatibility; hardened mode for defensive operation.
3. RaptorQ sidecars for long-lived conformance and benchmark artifacts.
4. Profile-first optimization with behavior proof artifacts.

## 2. Crate Map

- fp-types: dtype/null/index metadata
- fp-columnar: column buffers + validity bitmaps
- fp-frame: DataFrame/Series surface
- fp-index: index model and alignment
- fp-expr: logical expression DAG
- fp-groupby: group planners and kernels
- fp-join: join planners and kernels
- fp-io: CSV/Parquet/IPC adapters
- fp-conformance: pandas differential harness
- fp-runtime: strict/hardened policy + evidence ledger

## 3. Runtime Plan

- API layer normalizes inputs and validates invariants.
- Planner/dispatcher selects algorithm implementation.
- Core engine executes with explicit invariant checks.
- Conformance adapter executes packet-scoped suites and captures oracle + target outputs.
- Evidence layer emits parity reports, gate results, mismatch corpora, drift-history rows, and decode proofs.
- `fp-runtime` offers optional `asupersync` outcome-to-policy adapter hooks.
- decision records can be rendered as FTUI-friendly galaxy-brain cards.

## 4. Compatibility and Security

- strict mode: maximize scoped behavioral parity.
- hardened mode: same outward contract plus bounded defensive checks.
- fail-closed on unknown incompatible metadata/protocol fields.

## 5. Performance Contract

- baseline, profile, one-lever optimization, verify parity, re-baseline.
- p95/p99 and memory budgets enforced in CI.

## 6. Conformance Contract

- feature-family fixtures captured from legacy oracle.
- dual oracle mode:
  - fixture-expected mode for deterministic baseline checks.
  - live pandas mode (`crates/fp-conformance/oracle/pandas_oracle.py`) for direct behavioral capture.
- packet runner surfaces:
  - `run_packet_by_id`
  - `run_packets_grouped`
  - `evaluate_parity_gate`
  - `enforce_packet_gates`
  - `append_phase2c_drift_history`
- current fixture operation coverage:
  - `series_add`
  - `series_join`
  - `index_align_union`
  - `index_has_duplicates`
  - `index_first_positions`
- machine-readable artifacts per packet:
  - `parity_report.json`
  - `parity_gate_result.json`
  - `parity_mismatch_corpus.json`
  - `parity_report.raptorq.json`
  - `parity_report.decode_proof.json`
- cross-run artifact:
  - `artifacts/phase2c/drift_history.jsonl`
