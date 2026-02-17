# FP-P2D-017 Risk Note

Primary risk: constructor parity can silently drift when dtype coercion or index-alignment rules change, causing downstream semantic divergence even when operations still compile and run.

Mitigations:
1. Packet fixtures cover mixed-type coercion, incompatible-type failures, sparse-index union alignment, nullable propagation, and duplicate-name resolution.
2. Differential harness now compares explicit constructor outputs for both series and frame constructors.
3. Live oracle handlers anchor constructor behavior to pandas `Series`/`concat` semantics under the same payload contracts.

## Invariant Ledger Hooks

- `FP-I1` (constructor determinism): identical constructor payloads produce identical output dtypes/index/value layouts.
- `FP-I2` (missingness monotonicity): nulls introduced by alignment/coercion remain deterministic and dtype-consistent.
- `FP-I4` (alignment ordering): union-index ordering is stable across strict/hardened modes.
