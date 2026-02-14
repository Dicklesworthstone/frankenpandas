# FP-P2C-003 Risk Note

Primary risk: mixed-label and duplicate-label arithmetic semantics can drift from pandas in edge cases.

Mitigations:
1. strict mode remains fail-closed for unsupported compatibility surfaces.
2. hardened duplicate-label path is bounded and recorded.
3. packet gate thresholds enforce zero failed fixtures.
4. mismatch corpus is emitted each run for replay and drift triage.

## Isomorphism Proof Hook

- ordering preserved: yes for current union strategy (left order + right unseen append)
- tie-breaking preserved: yes for implemented first-hit behavior
- null/NaN/NaT behavior preserved: missing propagation on non-overlap paths is covered
- fixture checksum verification: tracked by `artifacts/perf/golden_checksums.txt`

## Invariant Ledger Hooks

- `FP-I1` (alignment homomorphism):
  - evidence: `artifacts/phase2c/FP-P2C-003/legacy_anchor_map.md`, `artifacts/phase2c/FP-P2C-003/contract_table.md`
- `FP-I2` (missingness monotonicity):
  - evidence: `artifacts/phase2c/FP-P2C-003/contract_table.md`, `crates/fp-conformance/fixtures/packets/fp_p2c_003_*`
- `FP-I4` (index determinism):
  - evidence: `artifacts/phase2c/FP-P2C-003/parity_gate.yaml`, packet differential report outputs
