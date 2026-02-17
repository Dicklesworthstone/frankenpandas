# FP-P2D-020 Contract Table

| Field | Contract |
|---|---|
| packet_id | `FP-P2D-020` |
| input_contract | constructor payloads encoded as scalar broadcast (`fill_value` + `index` + `column_order`) or dict-of-series (`left`/`right`/`groupby_keys` with optional `index`/`column_order`) |
| output_contract | deterministic frame materialization for scalar broadcast and dict-of-series alignment/reprojection surfaces |
| error_contract | malformed constructor payloads fail closed with explicit diagnostics |
| null_contract | missing labels/columns in dict-of-series constructor paths materialize deterministic null markers |
| index_contract | explicit `index` re-targeting controls row subset/reorder and null expansion for unseen labels |
| columns_contract | explicit `column_order` controls column projection/expansion with null fill for unseen columns |
| strict_mode_policy | zero critical drift tolerated |
| hardened_mode_policy | bounded divergence only in explicitly allowlisted defensive categories |
| constructor_scope | scalar broadcast, dict-of-series union alignment, column subset, missing-column expansion, duplicate-name overwrite behavior |
| excluded_scope | constructor kwargs beyond packet schema (`dtype=`, extension arrays, MultiIndex constructors, duplicate target-column labels) |
| oracle_tests | pandas `DataFrame(scalar, index=..., columns=...)` and `DataFrame(dict_of_series, index=..., columns=...)` baselines |
| performance_sentinels | constructor overhead under sparse alignment, projection, and missing-column expansion paths |
| compatibility_risks | index-order drift, null-kind drift on expansion, duplicate-name overwrite contract drift |
| raptorq_artifacts | parity report, RaptorQ sidecar, and decode proof emitted per packet run |
