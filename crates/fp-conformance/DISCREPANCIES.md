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

### DISC-012: Mixed naive / tz-aware CSV parse_dates normalizes per value
- **Reference:** Pandas handles a CSV column with mixed naive + tz-aware datetime strings by parsing each row independently (the naive rows produce `Timestamp` without tz; the aware rows produce `Timestamp` with tz). When converted to strings, both forms are reformatted into pandas' canonical `YYYY-MM-DD HH:MM:SS[±HH:MM]` shape.
- **Our impl:** fp-io now parses `read_csv(parse_dates=[...])` mixed naive + tz-aware columns per value by calling `to_datetime_values_with_options` with `infer_mixed_timezone=false` and `mixed_tz_as_object=true`. The column remains object-like (`Utf8`) because pandas cannot unify mixed tz-naive/tz-aware values into one `datetime64[ns]` dtype, but each value is normalized to the pandas object-string form.
- **Impact:** Conformance packet `FP-P2D-429` (`csv_read_frame_parse_dates_mixed_timezone_strict`) now matches the fixture: the aware row is normalized from `2024-01-15T10:30:00Z` to `2024-01-15 10:30:00+00:00`.
- **Resolution:** RESOLVED — covered by fp-io test `csv_parse_dates_mixed_naive_and_aware_strings_normalizes_per_value`; the stale accepted-divergence note was superseded by the per-value parse path used by `parse_csv_datetime_values`.
- **Tests affected:** none expected; historical coverage remains `packet_filter_runs_csv_read_frame_parse_dates_mixed_timezone_packet`.
- **Review date:** 2026-06-17

### DISC-015: memory_usage exact bytes differ from pandas (structural divergence)
- **Reference:** pandas `DataFrame.memory_usage()` reports exact bytes consumed by numpy-backed columns. For the test frame in `FP-P2D-364`, pandas returns 234 bytes (index + column overhead + numpy array backing).
- **Our impl:** FrankenPandas uses `Vec<Scalar>` storage which has structurally different memory characteristics. The same frame reports 32 bytes — a 7x difference reflecting heap-allocated scalars vs numpy's contiguous buffer layout.
- **Impact:** Conformance packet `FP-P2D-364` (`dataframe_memory_usage_with_nulls_hardened`) fails with `actual=32, expected=234`. This is NOT a bug but a fundamental structural difference.
- **Resolution:** ACCEPTED — exact-byte parity is impossible without adopting numpy's physical layout. Documented in README Memory Model section. Relative/shape assertions remain valid (larger frames use more memory monotonically). Excluded from parity-score numerator via fixture waiver.
- **Tests affected:** `FP-P2D-364`, any exact memory_usage comparison tests.
- **Review date:** 2026-05-25
- **Waiver:** Signed by user request per br-frankenpandas-rg8ys.5.2.

### DISC-011: Int64 columns receiving null values promote to Float64 (no nullable Int64 extension dtype)
- **Reference:** Pandas (since v0.24) has a nullable `Int64` extension dtype (capital I) that preserves the integer encoding via a separate validity mask. When a non-nullable `int64` column receives a null (e.g. via index alignment introducing rows with no source data, or via `concat(axis=1)` aligning over a non-matching index), pandas can either preserve `Int64` (extension) or promote to `float64` depending on dtype. The conformance oracle uses extension `Int64` where the column was originally `int64`.
- **Our impl:** No nullable extension Int64 dtype yet. Int64 columns that gain null values are promoted to `Float64` with `NaN`. Downstream IO (JSON, CSV) then serializes the integer values with a trailing `.0` (`1.0` rather than `1`).
- **Impact:** Several conformance packets exhibit `actual=Float64(1.0), expected=Int64(1)` mismatches:
  - `FP-P2D-028` (dataframe_concat_axis1): 5 of 10 cases fail because alignment over a wider index introduces nulls into formerly-Int64 columns.
  - `FP-P2D-433` (dataframe_to_json_records): JSON output writes `"a":1.0` instead of `"a":1` for integer columns that were promoted via null introduction.
  - Plus other downstream packets where alignment + nulls hit Int64 columns.
- **Resolution:** WILL-FIX - implementing nullable extension Int64 is a significant architectural change touching storage (fp-columnar), arithmetic kernels (fp-frame), and serialization (fp-io). Tracked under a future epic, not in scope for the fd90 SQL backend work. Per br-frankenpandas-mywg (fd90.76).
- **Tests affected:** `packet_filter_runs_dataframe_concat_axis1_packet`, `packet_filter_runs_dataframe_to_json_records_packet`, `fuzz_json_io_bytes_accepts_records_seed_fixture` (the records seed has `[{"temp":72},{"temp":null}]` — read promotes to Float64, write emits `72.0` instead of `72`, reparse + diff detects the drift), plus other downstream packets that hit the same root cause.
- **Review date:** 2026-04-26

## Resolved Divergences

### DISC-005: Mixed string/numeric constructors now preserve pandas object semantics
- **Reference:** `pd.Series(["x", 1])` and `pd.concat([pd.Series(["x", 1])], axis=1)` preserve heterogeneous values under pandas `object` dtype
- **Our impl:** Constructor inference now uses the existing `Utf8` storage bucket for pandas-style object columns while preserving heterogeneous `Scalar` payloads in order
- **Impact:** `Series::from_values` and `DataFrame::from_series` now match pandas for mixed string/numeric constructor inputs
- **Resolution:** ACCEPTED - parity achieved and covered by live-oracle plus fixture-backed tests
- **Tests affected:** `live_oracle_series_constructor_mixed_utf8_numeric_reports_object_values`, `live_oracle_dataframe_from_series_mixed_utf8_numeric_matches_object_values`, `series_constructor_utf8_numeric_object_strict`, `dataframe_from_series_utf8_numeric_object_strict`
- **Review date:** 2026-04-15

### DISC-013: Series + Series union alignment does not sort the result index
- **Reference:** Pandas `Series.add(other)` (and `series + other` operator) on differently-indexed Series performs an outer-join alignment that returns a sorted result index by default.
- **Our impl:** RESOLVED - unique-label Series arithmetic now uses a sorted outer union for `+` / `-` / `*` / `/` and fill-value arithmetic, while preserving the duplicate-aware cross-product path tracked separately by DISC-014.
- **Impact:** The `FP-P2C-001 series_add_alignment_union_strict` fallback fixture has been refreshed to pandas 2.2.3 output: result index `[1, 2, 3]`, values `[NaN, NaN, 34.0]`.
- **Resolution:** FIXED in br-frankenpandas-cod1d13 by routing unique-label Series arithmetic through sorted union alignment in fp-frame instead of changing the generic fp-index discovery-order helper. NB: the listed strict test still fails today, but for a different root cause (DISC-011 nullable-Int64 dtype promotion); the sort-order issue this entry tracked is no longer present.
- **Tests affected:** `series_add_aligns_on_union_index`, `series_add_fill_sorts_unique_outer_union_index`, `FP-P2C-001/series_add_alignment_union_strict`.
- **Review date:** 2026-04-28

### DISC-014: Series + Series duplicate-label arithmetic Int64 promotion (prior WILL-FIX premise was incorrect)
- **Reference:** Pandas `Series + Series` with duplicate labels performs cross-product alignment per label. The result stays `int64` when every label matches on both sides (no unmatched pairing, so no NaN is introduced); it promotes to `float64` only when an unmatched label actually injects a NaN.
- **Our impl:** Matches pandas exactly — the duplicate-aware cross-product keeps `Int64` when no NaN is generated and promotes to `Float64` when alignment leaves a position unmatched.
- **Impact:** None. The earlier entry claimed pandas *always* promotes duplicate-label results to `Float64` even with no NaN; that is false. Verified against the pandas 2.2.3 live oracle: `Series([1,2,3], index=['a','a','b']) + Series([3,4,5], index=['a','a','b'])` returns `int64 [4,6,8]`, while a partial match (`index=['a','a']` + `index=['a','a','b']`) returns `float64` with a trailing `NaN`. The fixture `fp_p2c_001_duplicate_hardened.json` already expects `int64 [4,5]` and the conformance test `conformance_series_add_duplicate_labels` passes. The previously-proposed "always promote to Float64 when alignment *can* introduce NaN" fix would have *broken* parity for the fully-matched case and must not be implemented.
- **Resolution:** RESOLVED - no code change required; FP already matches pandas. This entry corrects the prior incorrect WILL-FIX premise, which conflated this case with the genuinely-open DISC-011 (Int64 column that actually *receives* a null). Oracle-verified 2026-06-01.
- **Tests affected:** `conformance_series::conformance_series_add_duplicate_labels` (passing).
- **Review date:** 2026-06-01

### DISC-016: RangeIndex set-operation result ordering (all four ops RESOLVED)
- **Reference:** pandas 2.2.3 `RangeIndex` set operations use different default `sort` semantics:
  - `union` / `difference` / `symmetric_difference`: `sort=None` — result is ascending-sorted EXCEPT when an operand is empty or the two operands are value-equal, where the surviving operand passes through unchanged (order preserved).
  - `intersection`: `sort=False` — result is ascending EXCEPT when BOTH operands are descending (`step < 0`), where it is descending. (Verified vs pandas 2.2.3 over 150k random pairs.)
- **Our impl:** RESOLVED for all four. Previously fp returned every set op in self/discovery order (matching pandas only where the affine fast paths happened to already be in pandas' order), diverging for both-non-empty operands with descending or non-aligned (interleaving) lattices — union/difference/symmetric_difference for ~any descending/interleaved input, and intersection for self-descending ∩ other-ascending (≈23.5% of multi-element intersections). fp now normalizes operands to their ascending equivalent so the fast paths and fallbacks produce pandas' order, sorts the interleaving union/symmetric_difference fallbacks, and routes both-descending intersection through the operands as-is (self-order yields descending). Empty-operand and value-equal passthrough preserved; lazy affine / two-affine-run backing retained for the aligned common case (only reordered cases materialize).
- **Impact:** all four ops now bit-match pandas across a 263-case randomized differential (descending, non-aligned, empty, value-equal, disjoint, subset), plus 150k-pair Python sweeps confirming the ordering rules.
- **Resolution:** FIXED (DustySummit, br-frankenpandas). union/difference/symmetric_difference in commit 88e2a8487; intersection ordering in the follow-up commit.
- **Tests affected:** `range_index_set_ops_match_pandas_2_2_3_differential_dustysummit` (data-driven from `testdata_rangeset_pandas_cases.rs`, all four ops); `range_index_set_ops_use_direct_values_b7nxg`, `range_index_set_ops_closed_form_membership_preserves_order_iatnc`, `range_index_set_ops_return_affine_spans_iatnc` (refreshed to pandas-correct ordering).
- **Review date:** 2026-07-23

### DISC-017: RangeIndex.slice_locs / searchsorted reject monotonic-decreasing (step < 0) ranges
- **Reference:** pandas 2.2.3 permits `RangeIndex(10,0,-1).slice_locs(8,3)` → `(2, 8)` and `RangeIndex(10,0,-1).searchsorted(5)` → `0` on descending ranges.
- **Our impl:** ACCEPTED divergence — fp returns an explicit `InvalidArgument` error for `slice_locs` and `searchsorted` when `step < 0` ("requires a monotonic[ally-]increasing RangeIndex"), rather than replicating pandas' behavior. get_loc / get_indexer / get_indexer_non_unique / reindex (direction-agnostic value→position lookups) DO work correctly on descending ranges; only the two ordering-boundary ops refuse.
- **Impact:** Rationale for refusing rather than matching: pandas' `searchsorted` on a descending index is a numpy artifact — numpy `searchsorted` assumes an ascending array, so `desc.searchsorted(5) = 0` is a meaningless insertion point, not a correct one. pandas' descending `slice_locs` inherits the same ascending-assuming `get_slice_bound`, producing quirky edge results (e.g. `[7].slice_locs(8,-9) = (1,0)` — an EMPTY slice for a value that is in `[-9, 8]`). A randomized 40k-pair differential found no simple closed form matching pandas' descending slice_locs (≈7.6% of a naive `value<=start` / `value<end` closed form diverged, all in boundary/single-element cases). fp's explicit error avoids silently reproducing these numpy quirks; callers needing a descending slice can reverse the index first.
- **Resolution:** ACCEPTED (DustySummit, 2026-07-23). Revisit only if a use case needs pandas-bug-compatible descending `slice_locs`; it would require replicating pandas' `get_slice_bound` direction handling, not a closed form.
- **Tests affected:** none (fp's error path is covered by existing RangeIndex tests; no descending slice_locs/searchsorted parity test is asserted).
- **Review date:** 2026-07-23

## Rules

1. Every divergence gets a sequential ID (DISC-NNN)
2. Must state whether ACCEPTED, INVESTIGATING, or WILL-FIX
3. Must list affected test cases
4. Must include review date
5. Tests for ACCEPTED divergences use XFAIL markers where applicable
