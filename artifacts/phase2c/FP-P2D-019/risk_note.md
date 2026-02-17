# FP-P2D-019 Risk Note

Primary risk: kwargs-based frame reconstruction can silently drift on missing-axis fill semantics (rows/columns), creating shape-correct but value-incorrect outputs.

Mitigations:
1. Packet matrix covers row-only, column-only, combined axis targeting, missing-axis expansion, and malformed-payload failure cases.
2. Differential harness compares full expected-frame semantics and classifies constructor drift as critical.
3. Live oracle branch anchors behavior to pandas `DataFrame(frame, index=..., columns=...)` outcomes.

## Invariant Ledger Hooks

- `FP-I1` (shape consistency): index/column cardinality is deterministic under kwargs targeting.
- `FP-I2` (missingness monotonicity): unseen axis labels map to deterministic null values.
- `FP-I4` (determinism): same constructor payload yields identical output.
- `FP-I7` (fail-closed semantics): missing required constructor payload rejects explicitly.
