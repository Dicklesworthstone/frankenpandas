# FP-P2D-018 Risk Note

Primary risk: constructor parity can regress silently on sparse-record null materialization and index cardinality checks while still producing shape-compatible outputs.

Mitigations:
1. Packet matrix covers dict constructors, records constructors, explicit/implicit index behavior, and fail-closed malformed-input cases.
2. Conformance harness executes dedicated constructor operation paths with strict expected-frame and expected-error comparison logic.
3. Live oracle handlers anchor dict/records constructor behavior to pandas outcomes in differential mode.

## Invariant Ledger Hooks

- `FP-I1` (shape consistency): constructor output rows/columns are deterministic for the same payload.
- `FP-I2` (missingness monotonicity): sparse record keys map to deterministic null markers.
- `FP-I4` (determinism): constructor result is stable across repeated execution.
- `FP-I7` (fail-closed semantics): malformed constructor payloads reject instead of implicitly repairing.
