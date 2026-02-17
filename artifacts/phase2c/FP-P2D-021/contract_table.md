# FP-P2D-021 Contract Table

| Field | Contract |
|---|---|
| packet_id | `FP-P2D-021` |
| input_contract | list-like / 2D-array constructor payloads encoded as `matrix_rows` with optional `index` and `column_order` |
| output_contract | deterministic DataFrame materialization for rectangular and ragged row payloads with explicit null fill behavior |
| error_contract | malformed payloads (missing `matrix_rows`, index-length mismatch, row-width overflow under explicit columns) fail closed with explicit diagnostics |
| null_contract | short rows and explicit column expansion materialize deterministic null values |
| index_contract | default index is range-based; explicit index is accepted only when cardinality matches row count |
| columns_contract | default columns are positional string labels (`"0"`, `"1"`, ...); explicit `column_order` controls projection/expansion |
| strict_mode_policy | zero critical drift tolerated |
| hardened_mode_policy | bounded divergence only in explicitly allowlisted defensive categories |
| constructor_scope | list-like rectangular rows, ragged rows, empty inputs, mixed scalar types, explicit index/column controls |
| excluded_scope | nested extension arrays, MultiIndex constructors, duplicate target-column labels |
| oracle_tests | pandas `DataFrame(matrix_rows, index=..., columns=...)` baseline |
| performance_sentinels | constructor throughput under ragged-row null fill and explicit projection workloads |
| compatibility_risks | index-cardinality validation drift, default-column labeling drift, null-kind drift in ragged rows |
| raptorq_artifacts | parity report, RaptorQ sidecar, and decode proof emitted per packet run |
