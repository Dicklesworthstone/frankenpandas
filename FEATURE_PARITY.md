# FEATURE_PARITY

## Status Legend

- not_started
- in_progress
- parity_green
- parity_gap

## Parity Matrix

| Feature Family | Status | Notes |
|---|---|---|
| DataFrame/Series constructors | in_progress | `Series::from_values` + `DataFrame::from_series` MVP implemented; DataFrame constructor scalar-broadcast parity now available via `from_dict_mixed`/`from_dict_with_index_mixed` (fail-closed all-scalar-without-index and mismatched-shape guards); `FP-P2C-003` extends arithmetic fixture coverage; broader constructor parity pending |
| Expression planner arithmetic | in_progress | `fp-expr` now supports `Expr::Add`/`Sub`/`Mul`/`Div`, logical mask composition (`Expr::And`/`Or`/`Not`), plus `Expr::Compare` (`eq`/`ne`/`lt`/`le`/`gt`/`ge`) with full-eval and incremental-delta paths (including series-scalar anchoring), and includes DataFrame-backed eval/query bridges (`EvalContext::from_dataframe`, `evaluate_on_dataframe`, `filter_dataframe_on_expr`); broader window/string/date expression surface still pending |
| Index alignment and selection | in_progress | `FP-P2C-001`/`FP-P2C-002` packet suites green with gate validation and RaptorQ sidecars; `FP-P2C-010` adds series/dataframe filter-head-loc-iloc basics; `FP-P2D-025` extends DataFrame loc/iloc row+column selector parity; `FP-P2D-026` adds DataFrame head/tail parity; `FP-P2D-027` adds DataFrame head/tail negative-`n` parity; `fp-frame` now also exposes Series `head`/`tail` including negative-`n` semantics, DataFrame `set_index`/`reset_index` (single-index model), and row-level `duplicated`/`drop_duplicates`; core DataFrame ordering APIs `sort_index`/`sort_values` now implemented in `fp-frame`; full selector+ordering matrix still pending |
| GroupBy core aggregates | in_progress | `FP-P2C-005` and `FP-P2C-011` suites green (`sum`/`mean`/`count` core semantics); broader aggregate matrix still pending |
| Join/merge/concat core | in_progress | `FP-P2C-004` and `FP-P2C-006` suites green for series-level join/concat semantics; `FP-P2D-014` covers DataFrame merge + axis=0 concat matrix; `FP-P2D-028` adds DataFrame concat axis=1 outer alignment parity; `FP-P2D-029` adds axis=1 `join=inner` parity; `FP-P2D-030` adds axis=0 `join=inner` shared-column parity; `FP-P2D-031` adds axis=0 `join=outer` union-column/null-fill parity; `FP-P2D-032` adds axis=0 `join=outer` first-seen column-order (`sort=False`) parity; `FP-P2D-039` adds DataFrame merge `how='cross'` semantics (including suffix/indicator and invalid key/index guard rails); full DataFrame merge/concat contracts still pending |
| Null/NaN semantics | in_progress | `FP-P2C-007` suite green for `dropna`/`fillna`/`nansum`; `fp-frame` now also exposes Series/DataFrame `isna`/`notna` plus `isnull`/`notnull` aliases, DataFrame `fillna`, optioned row-wise `dropna` (`how='any'/'all'` + `thresh` with column `subset` selectors), and optioned column-wise `dropna` (`axis=1`, `how='any'/'all'` + `thresh`, row-label `subset`, plus default `dropna_columns()`); full nanops matrix still pending |
| Core CSV ingest/export | in_progress | `FP-P2C-008` suite green for CSV round-trip core cases; `fp-io` now supports optioned file-based CSV reads (`read_csv_with_options_path`) and JSON `records`/`columns`/`split`/`index` orients (including split index-label roundtrip); broader parser/formatter parity matrix pending |
| Storage/dtype invariants | in_progress | `FP-P2C-009` suite green for dtype invariant checks; `fp-frame` now exposes `Series::astype` plus DataFrame single-column coercion via `astype_column`; broader dtype coercion/storage matrix pending |

## Phase-2C Packet Evidence (Current)

| Packet | Result | Evidence |
|---|---|---|
| FP-P2C-001 | parity_green | `artifacts/phase2c/FP-P2C-001/parity_report.json`, `artifacts/phase2c/FP-P2C-001/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-001/parity_report.raptorq.json` |
| FP-P2C-002 | parity_green | `artifacts/phase2c/FP-P2C-002/parity_report.json`, `artifacts/phase2c/FP-P2C-002/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-002/parity_report.raptorq.json` |
| FP-P2C-003 | parity_green | `artifacts/phase2c/FP-P2C-003/parity_report.json`, `artifacts/phase2c/FP-P2C-003/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-003/parity_report.raptorq.json` |
| FP-P2C-004 | parity_green | `artifacts/phase2c/FP-P2C-004/parity_report.json`, `artifacts/phase2c/FP-P2C-004/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-004/parity_report.raptorq.json` |
| FP-P2C-005 | parity_green | `artifacts/phase2c/FP-P2C-005/parity_report.json`, `artifacts/phase2c/FP-P2C-005/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-005/parity_report.raptorq.json` |
| FP-P2C-006 | parity_green | `artifacts/phase2c/FP-P2C-006/parity_report.json`, `artifacts/phase2c/FP-P2C-006/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-006/parity_report.raptorq.json` |
| FP-P2C-007 | parity_green | `artifacts/phase2c/FP-P2C-007/parity_report.json`, `artifacts/phase2c/FP-P2C-007/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-007/parity_report.raptorq.json` |
| FP-P2C-008 | parity_green | `artifacts/phase2c/FP-P2C-008/parity_report.json`, `artifacts/phase2c/FP-P2C-008/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-008/parity_report.raptorq.json` |
| FP-P2C-009 | parity_green | `artifacts/phase2c/FP-P2C-009/parity_report.json`, `artifacts/phase2c/FP-P2C-009/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-009/parity_report.raptorq.json` |
| FP-P2C-010 | parity_green | `artifacts/phase2c/FP-P2C-010/parity_report.json`, `artifacts/phase2c/FP-P2C-010/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-010/parity_report.raptorq.json` |
| FP-P2C-011 | parity_green | `artifacts/phase2c/FP-P2C-011/parity_report.json`, `artifacts/phase2c/FP-P2C-011/parity_gate_result.json`, `artifacts/phase2c/FP-P2C-011/parity_report.raptorq.json` |
| FP-P2D-025 | parity_green | `artifacts/phase2c/FP-P2D-025/parity_report.json`, `artifacts/phase2c/FP-P2D-025/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-025/parity_report.raptorq.json` |
| FP-P2D-026 | parity_green | `artifacts/phase2c/FP-P2D-026/parity_report.json`, `artifacts/phase2c/FP-P2D-026/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-026/parity_report.raptorq.json` |
| FP-P2D-027 | parity_green | `artifacts/phase2c/FP-P2D-027/parity_report.json`, `artifacts/phase2c/FP-P2D-027/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-027/parity_report.raptorq.json` |
| FP-P2D-028 | parity_green | `artifacts/phase2c/FP-P2D-028/parity_report.json`, `artifacts/phase2c/FP-P2D-028/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-028/parity_report.raptorq.json` |
| FP-P2D-029 | parity_green | `artifacts/phase2c/FP-P2D-029/parity_report.json`, `artifacts/phase2c/FP-P2D-029/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-029/parity_report.raptorq.json` |
| FP-P2D-030 | parity_green | `artifacts/phase2c/FP-P2D-030/parity_report.json`, `artifacts/phase2c/FP-P2D-030/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-030/parity_report.raptorq.json` |
| FP-P2D-031 | parity_green | `artifacts/phase2c/FP-P2D-031/parity_report.json`, `artifacts/phase2c/FP-P2D-031/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-031/parity_report.raptorq.json` |
| FP-P2D-032 | parity_green | `artifacts/phase2c/FP-P2D-032/parity_report.json`, `artifacts/phase2c/FP-P2D-032/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-032/parity_report.raptorq.json` |
| FP-P2D-039 | parity_green | `artifacts/phase2c/FP-P2D-039/parity_report.json`, `artifacts/phase2c/FP-P2D-039/parity_gate_result.json`, `artifacts/phase2c/FP-P2D-039/parity_report.raptorq.json` |

Gate enforcement and trend history:

- blocking command: `./scripts/phase2c_gate_check.sh`
- CI workflow: `.github/workflows/ci.yml`
- drift history ledger: `artifacts/phase2c/drift_history.jsonl`

## Required Evidence Per Feature Family

1. Differential fixture report.
2. Edge-case/adversarial test results.
3. Benchmark delta (when performance-sensitive).
4. Documented compatibility exceptions (if any).
