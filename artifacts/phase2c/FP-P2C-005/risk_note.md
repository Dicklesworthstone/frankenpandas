# FP-P2C-005 Risk Note

Primary risk: groupby key ordering and null-key/dropna semantics can drift from pandas-observable behavior under mixed index alignment.

Mitigations:
1. strict packet gate enforces zero failed fixtures.
2. hardened divergence budget is explicit and allowlisted.
3. mismatch corpus is emitted every run for replay.
4. drift history records packet-level pass/fail trends.

## Isomorphism Proof Hook

- ordering preserved: first-seen key encounter order remains deterministic
- tie-breaking preserved: repeated keys accumulate in stable scan order
- null/NaN/NaT behavior preserved: missing keys skipped (`dropna=true`), missing values treated as additive no-op
- fixture checksum verification: tracked in `artifacts/perf/golden_checksums.txt`

## Invariant Ledger Hooks

- `FP-I2` (missingness monotonicity):
  - evidence: `artifacts/phase2c/FP-P2C-005/contract_table.md`, packet fixture family `fp_p2c_005_*`
- `FP-I4` (index/order determinism):
  - evidence: first-seen ordering contract in `artifacts/phase2c/FP-P2C-005/legacy_anchor_map.md` and parity gate checks
- aggregate contract lock (scoped sum):
  - evidence: `artifacts/phase2c/FP-P2C-005/parity_gate.yaml`, packet differential report outputs
