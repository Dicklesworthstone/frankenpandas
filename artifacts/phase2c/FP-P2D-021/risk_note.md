# FP-P2D-021 Risk Note

Primary risk: list-like constructor parity can drift on row/column cardinality handling, producing shape-correct but value-incorrect frames under ragged row inputs.

Mitigations:
1. Packet matrix covers rectangular inputs, ragged rows, empty rows, explicit index/column controls, mixed scalar domains, and malformed payload failures.
2. Differential harness compares full expected-frame semantics and classifies constructor drift as critical.
3. Live oracle parity anchors behavior to pandas list-like constructor outcomes.

## Invariant Ledger Hooks

- `FP-I1` (shape consistency): output index/column cardinality is deterministic under explicit and default controls.
- `FP-I2` (missingness monotonicity): short rows and column expansion produce deterministic null fill.
- `FP-I4` (determinism): identical list-like constructor payloads yield identical outputs.
- `FP-I7` (fail-closed semantics): malformed list-like constructor payloads reject explicitly.
