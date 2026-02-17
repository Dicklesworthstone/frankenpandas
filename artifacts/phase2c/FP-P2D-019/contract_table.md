# FP-P2D-019 Contract Table

| Field | Contract |
|---|---|
| packet_id | `FP-P2D-019` |
| input_contract | kwargs-style constructor payload from existing frame via `frame` + optional `index` + optional `column_order` |
| output_contract | reconstructed frame preserves requested index/column surfaces and deterministic null fill for missing axis labels |
| error_contract | missing required constructor payload (`frame`) fails closed with explicit diagnostics |
| null_contract | missing row labels and missing columns materialize as deterministic null values |
| index_contract | provided `index` controls row selection/reordering with nullable expansion for unseen labels |
| columns_contract | provided `column_order` controls projection/expansion; unseen columns produce all-null output columns |
| strict_mode_policy | zero critical drift tolerated |
| hardened_mode_policy | bounded divergence only in explicitly allowlisted defensive categories |
| kwargs_scope | frame-derived constructor with axis re-targeting (`index`, `columns`) |
| excluded_scope | full pandas constructor kwargs set (`dtype`, `copy`, extension-array backends, MultiIndex kwargs) |
| oracle_tests | pandas `DataFrame(frame, index=..., columns=...)` parity baseline |
| performance_sentinels | axis re-targeting overhead under sparse-label and missing-column paths |
| compatibility_risks | null-kind drift on missing axis fills, index-order drift, missing-column expansion regressions |
| raptorq_artifacts | parity report, RaptorQ sidecar, and decode proof emitted per packet run |
