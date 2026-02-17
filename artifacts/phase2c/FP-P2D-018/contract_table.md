# FP-P2D-018 Contract Table

| Field | Contract |
|---|---|
| packet_id | `FP-P2D-018` |
| input_contract | constructor payloads encoded as `dict_columns` (dict-of-columns) or `records` (list-of-row dicts) with optional `column_order` and `index` |
| output_contract | `dataframe_from_dict` and `dataframe_from_records` produce deterministic DataFrame index/column/value payloads |
| error_contract | malformed constructor payloads (missing columns, length mismatches, index/row cardinality mismatch) fail closed with explicit errors |
| null_contract | missing record keys materialize as deterministic null markers with dtype-aware coercion rules |
| dtype_contract | mixed numeric values promote to `Float64`; incompatible coercions are rejected by constructor pipeline |
| strict_mode_policy | zero critical drift tolerated |
| hardened_mode_policy | bounded divergence only in explicitly allowlisted defensive categories |
| constructor_scope | dict-of-columns with inferred/explicit index, records with sparse keys, column subset/expansion semantics |
| excluded_scope | pandas constructor kwargs beyond packet schema (`dtype=`, `copy=`, nested records, MultiIndex constructors) |
| oracle_tests | pandas `DataFrame(dict, ...)` and `DataFrame(records, ...)` constructor baselines |
| performance_sentinels | constructor throughput under sparse records and explicit column filtering |
| compatibility_risks | null-kind drift in sparse records, index cardinality validation drift, column-selection contract regressions |
| raptorq_artifacts | parity report, RaptorQ sidecar, and decode proof emitted per packet run |
