# FP-P2D-020 Risk Note

Primary risk: constructor parity can drift silently on scalar broadcast shape checks and dict-of-series alignment/projection, producing shape-correct but value-incorrect frames.

Mitigations:
1. Packet matrix covers scalar broadcast, empty-axis edges, dict-of-series union alignment, explicit row/column projection, missing-column null fill, and malformed-payload failures.
2. Differential harness compares full expected-frame semantics and classifies constructor drift as critical.
3. Live oracle branch anchors behavior to pandas scalar and dict-of-series constructor outcomes.

## Invariant Ledger Hooks

- `FP-I1` (shape consistency): constructor output shape is deterministic under explicit index/column controls.
- `FP-I2` (missingness monotonicity): unseen labels/columns map to deterministic null values.
- `FP-I4` (determinism): repeated constructor payloads produce identical output.
- `FP-I7` (fail-closed semantics): malformed constructor payloads reject with explicit diagnostics.
