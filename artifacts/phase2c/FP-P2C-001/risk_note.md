# FP-P2C-001 Risk Note

Primary risk: duplicate-label alignment can silently drift from pandas behavior.

Mitigations:
1. strict mode hard-rejects unsupported duplicate-label semantics.
2. hardened mode only permits bounded repair with explicit evidence ledger entries.
3. conformance fixtures include duplicate-label adversarial case.
4. packet gate (`parity_gate.yaml`) is enforced with machine-readable result output.
5. mismatch corpus is emitted for every run, even when empty, to preserve replay hooks.

## Isomorphism Proof Hook

- ordering preserved: yes for current union strategy (left-order + right-unseen append)
- tie-breaking preserved: yes within current strategy; full pandas tie behavior deferred
- null/NaN/NaT behavior preserved: yes for implemented arithmetic + missing propagation slice
- fixture checksum verification: complete (`artifacts/perf/golden_checksums.txt`)

## Invariant Ledger Hooks

- `FP-I1` (alignment homomorphism):
  - evidence: `artifacts/phase2c/FP-P2C-001/legacy_anchor_map.md`, `artifacts/phase2c/FP-P2C-001/contract_table.md`
- `FP-I2` (missingness monotonicity):
  - evidence: `artifacts/phase2c/FP-P2C-001/contract_table.md`, `crates/fp-conformance/fixtures/packets/fp_p2c_001_*`
- `FP-I4` (index determinism):
  - evidence: `artifacts/phase2c/FP-P2C-001/parity_gate.yaml`, packet differential report outputs
