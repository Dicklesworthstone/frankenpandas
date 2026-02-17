# FP-P2D-017 Contract Table

| Field | Contract |
|---|---|
| packet_id | `FP-P2D-017` |
| input_contract | heterogeneous constructor payloads for Series/DataFrame creation (`left`, `right`, `groupby_keys`) |
| output_contract | constructor outputs preserve index/value contracts with deterministic dtype/null coercion |
| error_contract | incompatible dtype mixes and malformed constructor payloads fail closed with explicit errors |
| null_contract | null coercions are deterministic and dtype-aware (`null` for Int64/Utf8, `na_n` for Float64) |
| alignment_contract | `dataframe_from_series` performs union-index alignment with first-position semantics and deterministic ordering |
| strict_mode_policy | zero critical drift tolerated |
| hardened_mode_policy | bounded divergences only in explicit allowlisted defensive categories |
| dtype_contract | `Bool+Int64 -> Int64`, `Int64+Float64 -> Float64`, `Utf8+numeric -> incompatibility error` |
| excluded_scope | ctor kwargs parity (`dtype=`, `copy=`, `name=` variants), full DataFrame dict/records constructors beyond current payload schema |
| oracle_tests | pandas `Series(...)` and `pd.concat(..., axis=1, sort=False)` constructor baselines |
| performance_sentinels | constructor throughput under sparse-index union and mixed-type coercion |
| compatibility_risks | duplicate column-name resolution, null-kind drift, union-index ordering regressions |
| raptorq_artifacts | parity report, RaptorQ sidecar, and decode proof emitted per packet run |
