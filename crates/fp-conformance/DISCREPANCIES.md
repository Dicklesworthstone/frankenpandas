# Known Conformance Divergences

> Every intentional divergence from pandas behavior is documented here.
> Format: DISC-NNN, status (ACCEPTED/INVESTIGATING/WILL-FIX), affected tests.

## Active Divergences

### DISC-001: Integer division by zero promotes to Float64 with NaN/inf
- **Reference:** pandas `int64 // int64` with zero divisor returns `float64` with `inf`
- **Our impl:** Same behavior - promotes to Float64, returns `inf` for floor division, `nan` for mod
- **Impact:** Dtype promotion matches, values match
- **Resolution:** ACCEPTED - exact pandas parity achieved
- **Tests affected:** `int64_mod_floordiv_with_zero_promotes_to_float`
- **Review date:** 2026-04-15

### DISC-002: Unicode width tables version
- **Reference:** pandas uses system's ICU or Python's unicodedata (varies by install)
- **Our impl:** Uses `unicode-width` crate (Unicode 15.1 tables)
- **Impact:** Some emoji/CJK width calculations may differ by 1 column
- **Resolution:** ACCEPTED - newer Unicode tables are more correct
- **Tests affected:** None currently - string display width not yet tested
- **Review date:** 2026-04-15

### DISC-003: Error message text differs
- **Reference:** pandas error messages vary by version and locale
- **Our impl:** Custom error messages with consistent format
- **Impact:** Error semantics match, exact text differs
- **Resolution:** ACCEPTED - tests check error category, not message text
- **Tests affected:** All error-expecting tests use `expected_error_contains`
- **Review date:** 2026-04-15

### DISC-004: CSV NA value handling default differs from pandas 1.x
- **Reference:** pandas 2.x treats "None" as NA by default; pandas 1.x did not
- **Our impl:** Follows pandas 2.x behavior with `keep_default_na=true`
- **Impact:** Users migrating from pandas 1.x may see different behavior
- **Resolution:** ACCEPTED - aligning with current pandas 2.x
- **Tests affected:** `csv_none_is_default_na`
- **Review date:** 2026-04-15

### DISC-006: Row MultiIndex is scaffolded, not full pandas parity
- **Reference:** pandas `MultiIndex` supports arbitrary-level hierarchical row labels with full slicing, `xs`, `droplevel`, `swaplevel`, `reindex`, `sort_index`, etc.
- **Our impl:** Row-MultiIndex first slice ships struct + constructor + level access. Full slicing / xs / droplevel / swaplevel coverage lands in subsequent slices (umbrella tracked by br-frankenpandas-1zzp).
- **Impact:** DataFrames built with a row MultiIndex may reject operations that pandas accepts, or return partial results. Error messages identify which operation is pending.
- **Resolution:** INVESTIGATING - slices land under br-1zzp child beads until coverage parity is reached.
- **Tests affected:** `live_oracle_dataframe_row_multiindex_*` suite (scoped to shipped operations).
- **Review date:** 2026-04-23

### DISC-007: SQL IO is SQLite-only; pandas supports multiple backends
- **Reference:** pandas `read_sql` / `to_sql` accept any SQLAlchemy-compatible backend (SQLite, PostgreSQL, MySQL, Oracle, MSSQL, etc.).
- **Our impl:** fp-io's `read_sql` / `write_sql` only accept a `rusqlite::Connection`. PostgreSQL / MySQL / Oracle connectors not shipped.
- **Impact:** Users whose pipelines depend on non-SQLite backends cannot drop-in replace pandas IO calls.
- **Resolution:** INVESTIGATING - tracked by br-frankenpandas-fd90 (SQL backend epic, 7 slices). SQLite remains the supported scope until slices 2+ land.
- **Tests affected:** `live_oracle_sql_*` suite (SQLite only).
- **Review date:** 2026-04-23

### DISC-008: No Python bindings shipped; pandas' Python-level drop-in positioning differs
- **Reference:** pandas IS a Python library. Users `import pandas`.
- **Our impl:** frankenpandas is a Rust library. Users `use frankenpandas::*` from Rust code. Python bindings (e.g. via PyO3) are not shipped.
- **Impact:** README's "drop-in pandas replacement" positioning applies at the API-shape level, not at the import-statement level. A Python pandas user cannot adopt frankenpandas without first porting to Rust.
- **Resolution:** ACCEPTED - README was updated to qualify the claim (br-frankenpandas-diic closed by wording rather than bindings). PyO3 bindings remain a future-epic candidate, not in scope for 0.1.0.
- **Tests affected:** N/A - positioning / documentation divergence, not behavioral.
- **Review date:** 2026-04-23

### DISC-009: Sparse dtype descriptor exists before compressed sparse storage
- **Reference:** pandas `SparseDtype` pairs an underlying value dtype with a fill value and stores only non-fill positions in `SparseArray`.
- **Our impl:** `fp-types::SparseDType` records the dtype/fill-value contract and `DType::Sparse` marks the logical dtype. fp-columnar still stores columns densely and IO falls back to textual sparse markers until a compressed sparse column representation lands.
- **Impact:** Code can now describe sparse dtype intent, but memory usage and `Series.sparse` accessor parity still differ from pandas.
- **Resolution:** WILL-FIX - remaining storage/accessor work tracked by br-frankenpandas-0xcm follow-up slices.
- **Tests affected:** Sparse storage/accessor conformance tests not yet enabled.
- **Review date:** 2026-04-24

### DISC-010: Rust GroupBy.apply uses explicit output-shape APIs
- **Reference:** pandas `DataFrameGroupBy.apply` dynamically dispatches scalar, Series, and DataFrame return values from one Python callable.
- **Our impl:** Rust's static return types expose the same shape families as explicit methods: `apply_scalar`, `apply_series`, `apply_series_stacked`, and DataFrame-returning `apply`. DataFrame-returning apply retains group-key row MultiIndex metadata; stacked Series output is represented as a one-column DataFrame until Series row MultiIndex metadata lands.
- **Impact:** Shape semantics are available, but Rust callers choose the expected output family at compile time instead of receiving a dynamic Python object.
- **Resolution:** INVESTIGATING - a future Python binding layer can restore one-call dynamic dispatch over these Rust shape-specific methods.
- **Tests affected:** `dataframe_groupby_apply`, `dataframe_groupby_apply_scalar_returns_series_indexed_by_keys`, `dataframe_groupby_apply_series_unions_sparse_result_columns`, `dataframe_groupby_apply_series_stacked_preserves_variable_labels`.
- **Review date:** 2026-04-25

### DISC-011: Int64 columns receiving null values promote to Float64 (no nullable Int64 extension dtype)
- **Reference:** Pandas (since v0.24) has a nullable `Int64` extension dtype (capital I) that preserves the integer encoding via a separate validity mask. When a non-nullable `int64` column receives a null (e.g. via index alignment introducing rows with no source data, or via `concat(axis=1)` aligning over a non-matching index), pandas can either preserve `Int64` (extension) or promote to `float64` depending on dtype. The conformance oracle uses extension `Int64` where the column was originally `int64`.
- **Our impl:** No nullable extension Int64 dtype yet. Int64 columns that gain null values are promoted to `Float64` with `NaN`. Downstream IO (JSON, CSV) then serializes the integer values with a trailing `.0` (`1.0` rather than `1`).
- **Impact:** Several conformance packets exhibit `actual=Float64(1.0), expected=Int64(1)` mismatches:
  - `FP-P2D-028` (dataframe_concat_axis1): 5 of 10 cases fail because alignment over a wider index introduces nulls into formerly-Int64 columns.
  - `FP-P2D-433` (dataframe_to_json_records): JSON output writes `"a":1.0` instead of `"a":1` for integer columns that were promoted via null introduction.
  - Plus other downstream packets where alignment + nulls hit Int64 columns.
- **Resolution:** WILL-FIX - implementing nullable extension Int64 is a significant architectural change touching storage (fp-columnar), arithmetic kernels (fp-frame), and serialization (fp-io). Tracked under a future epic, not in scope for the fd90 SQL backend work. Per br-frankenpandas-mywg (fd90.76).
- **Tests affected:** `packet_filter_runs_dataframe_concat_axis1_packet`, `packet_filter_runs_dataframe_to_json_records_packet`, plus downstream packets that hit the same root cause.
- **Review date:** 2026-04-26

## Resolved Divergences

### DISC-005: Mixed string/numeric constructors now preserve pandas object semantics
- **Reference:** `pd.Series(["x", 1])` and `pd.concat([pd.Series(["x", 1])], axis=1)` preserve heterogeneous values under pandas `object` dtype
- **Our impl:** Constructor inference now uses the existing `Utf8` storage bucket for pandas-style object columns while preserving heterogeneous `Scalar` payloads in order
- **Impact:** `Series::from_values` and `DataFrame::from_series` now match pandas for mixed string/numeric constructor inputs
- **Resolution:** ACCEPTED - parity achieved and covered by live-oracle plus fixture-backed tests
- **Tests affected:** `live_oracle_series_constructor_mixed_utf8_numeric_reports_object_values`, `live_oracle_dataframe_from_series_mixed_utf8_numeric_matches_object_values`, `series_constructor_utf8_numeric_object_strict`, `dataframe_from_series_utf8_numeric_object_strict`
- **Review date:** 2026-04-15

## Rules

1. Every divergence gets a sequential ID (DISC-NNN)
2. Must state whether ACCEPTED, INVESTIGATING, or WILL-FIX
3. Must list affected test cases
4. Must include review date
5. Tests for ACCEPTED divergences use XFAIL markers where applicable
