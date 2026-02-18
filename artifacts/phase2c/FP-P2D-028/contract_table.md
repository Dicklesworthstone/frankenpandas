# FP-P2D-028 Contract Table

| Field | Contract |
|---|---|
| packet_id | `FP-P2D-028` |
| input_contract | two DataFrame payloads (`frame`, `frame_right`) plus signed `concat_axis` selector |
| output_contract | `concat_axis=1` materializes column-wise concat with deterministic outer index alignment |
| error_contract | unsupported axis, duplicate output column names, and duplicate input index labels fail closed with explicit diagnostics |
| null_contract | missing row positions per side are materialized as `null` in that side's columns |
| index_alignment_contract | union index preserves left-then-right-unseen label order (`sort=False` semantics) |
| strict_mode_policy | zero critical drift tolerated |
| hardened_mode_policy | divergence only in explicit allowlisted defensive classes |
| selector_scope | `dataframe_concat` with `concat_axis=1` |
| excluded_scope | axis=0 concat (covered by `FP-P2D-014`), MultiIndex concat, duplicate-column preservation semantics |
| oracle_tests | pandas `concat(axis=1, sort=False)` over numeric + utf8 indexes with alignment + null fill matrix |
| performance_sentinels | sparse outer alignment null-fill path, union-index stability under non-monotonic labels |
| compatibility_risks | index-order drift (`sort` leakage), duplicate-column handling drift, duplicate-index fallback drift |
| raptorq_artifacts | parity report, RaptorQ sidecar, and decode proof emitted per packet run |

