# FP-P2C-011 Risk Note

Primary risk: aggregate function semantics for edge cases (empty groups, single-value groups, all-null groups) have subtle pandas-specific behavior that may diverge.

Mitigations:
1. Each aggregate function documents explicit empty/all-null behavior.
2. Dense-int fast path has isomorphism proof against generic path.
3. Arena-backed path has behavioral equivalence proof against global allocator path.
4. std/var use ddof=1 (sample statistics) matching pandas default.
5. Hardened packet fixtures cover null-key exclusion and null-value skipping parity for `mean` and `count`.

## Isomorphism Proof Hook

- aggregate semantics: each function defines deterministic empty/all-null behavior
- ordering preserved: first-seen key encounter order is deterministic
- dense/generic isomorphism: dense-int fast path produces identical output to generic path
- arena/global isomorphism: arena-backed path produces identical output to global allocator path
- null handling: null keys excluded (dropna=true), null values skipped in aggregations

## Invariant Ledger Hooks

- `FP-I2` (missingness monotonicity):
  - evidence: `artifacts/phase2c/FP-P2C-011/contract_table.md`, null aggregation contracts
- `FP-I4` (index/order determinism):
  - evidence: first-seen key order contract in legacy anchor map
- aggregate isomorphism lock:
  - evidence: `artifacts/perf/ROUND3_ISOMORPHISM_PROOF.md`, `artifacts/perf/ROUND4_ISOMORPHISM_PROOF.md`
- dense-generic path equivalence:
  - evidence: `groupby_isomorphism_generic_vs_dense` test, `arena_groupby_matches_global_allocator_behavior` test
