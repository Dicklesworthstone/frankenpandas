# FP-P2C-004 Risk Note

Primary risk: duplicate-key join cardinality and missing-right marker behavior can drift from pandas-observable semantics.

Mitigations:
1. strict packet gate enforces zero failed fixtures.
2. hardened duplicate path remains explicit and audited.
3. mismatch corpus is emitted every run for replay.
4. drift history records packet-level pass/fail trends.

## Isomorphism Proof Hook

- ordering preserved: left-driven output ordering is deterministic in current implementation
- tie-breaking preserved: duplicate-key expansion follows stable nested loop order
- null/NaN/NaT behavior preserved: unmatched right rows map to missing scalar markers
- fixture checksum verification: tracked in `artifacts/perf/golden_checksums.txt`

## Invariant Ledger Hooks

- `FP-I3` (join cardinality integrity):
  - evidence: `artifacts/phase2c/FP-P2C-004/legacy_anchor_map.md`, `artifacts/phase2c/FP-P2C-004/contract_table.md`
  - fixtures: `crates/fp-conformance/fixtures/packets/fp_p2c_004_*`
- `FP-I4` (index determinism):
  - evidence: left-driven stable ordering in `artifacts/phase2c/FP-P2C-004/contract_table.md` + packet parity gates
- `FP-I2` (missingness monotonicity):
  - evidence: left-join unmatched-right missing marker contract and packet fixtures
