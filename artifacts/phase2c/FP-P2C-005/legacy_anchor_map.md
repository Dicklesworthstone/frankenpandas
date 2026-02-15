# FP-P2C-005 Legacy Anchor Map

Packet: `FP-P2C-005`
Subsystem: groupby sum semantics

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/groupby/groupby.py` (groupby planning and reduction surfaces)
- `legacy_pandas_code/pandas/pandas/core/series.py` (series alignment and groupby entry points)

## Extracted Behavioral Contract

1. group key encounter order is preserved when sort is disabled.
2. missing keys are skipped under default `dropna=true` semantics.
3. missing values do not contribute to sums but keys remain materialized when encountered.

## Rust Slice Implemented

- `crates/fp-groupby/src/lib.rs`: `groupby_sum` with explicit alignment and first-seen ordering
- `crates/fp-conformance/src/lib.rs`: `groupby_sum` fixture operation and packet gate coverage
- `crates/fp-conformance/oracle/pandas_oracle.py`: live oracle adapter for `groupby_sum`

## Type Inventory

- `fp_groupby::GroupByOptions`
  - fields: `dropna: bool`
- `fp_groupby::GroupByError`
  - variants: `Frame`, `Index`, `Column`
- `fp_groupby::GroupKeyRef`
  - variants: `Bool`, `Int64`, `FloatBits`, `Utf8`, `Null`
- `fp_frame::Series`
  - fields: `name: String`, `index: Index`, `column: Column`
- `fp_columnar::Column`
  - fields: `dtype: DType`, `values: Vec<Scalar>`, `validity: ValidityMask`
- `fp_index::Index`
  - fields: `labels: Vec<IndexLabel>`, `duplicate_cache: OnceCell<bool>`

## Rule Ledger

1. Entry dispatch (`groupby_sum`, line 58):
   - 1a. Delegates to `groupby_sum_with_options` with `GroupByExecutionOptions::default()` (arena enabled, budget 256 MiB).
   - 1b. `groupby_sum_with_options` (line 75) delegates to `groupby_sum_with_trace` and discards the trace.

2. Alignment gate (`groupby_sum_with_trace`, line 97):
   - 2a. if `keys.index() == values.index() && !keys.index().has_duplicates()` -> identity fast path, `aligned_storage = None`, use raw `keys.values()` / `values.values()` directly (no reindex allocation).
   - 2b. if indexes differ OR keys index has duplicates -> compute `align_union(keys.index(), values.index())`, call `validate_alignment_plan(&plan)` (may return `IndexError::InvalidAlignmentVectors`), then reindex both columns via `reindex_by_positions`.
   - 2c. After alignment, `aligned_keys_values` and `aligned_values_values` are bound as `&[Scalar]` slices from either the identity path (2a) or the reindexed columns (2b).
   - Edge: if keys and values have identical labels but keys has duplicates, the slow alignment path is taken even though the indexes are equal. This mirrors pandas dedup-sensitivity.

3. Arena budget gate (`groupby_sum_with_trace`, line 118):
   - 3a. `estimated_bytes = estimate_groupby_intermediate_bytes(input_rows)` computes `input_rows * (8 + 1 + 8 + 64)` = 81 bytes/row (line 141-146).
   - 3b. if `exec_options.use_arena && estimated_bytes <= exec_options.arena_budget_bytes` -> arena path (`groupby_sum_with_arena`).
   - 3c. if `!exec_options.use_arena` OR `estimated_bytes > arena_budget_bytes` -> global allocator path (`groupby_sum_with_global_allocator`).
   - Edge: default budget is 256 MiB, so arena is used for up to ~3.3M rows; beyond that, falls back to global allocator.

4. Dense-int probe (`try_groupby_sum_dense_int64` line 324, `try_groupby_sum_dense_int64_arena` line 381):
   - 4a. `dense_int64_range(keys, dropna)` (line 301) scans all keys:
     - 4a-i. `Scalar::Int64(v)` -> update min/max, set `saw_int_key = true`.
     - 4a-ii. `Scalar::Null(_) if dropna` -> `continue` (skip null key).
     - 4a-iii. any other variant (Float64, Bool, Utf8, Null when `!dropna`) -> `return None` (bail out of dense path entirely).
   - 4b. if `!saw_int_key` (all keys were droppable nulls) -> `return Some((Vec::new(), Vec::new()))` (empty result).
   - 4c. `span = max_key - min_key + 1` (computed as i128 to avoid overflow).
   - 4d. if `span <= 0 || span > 65_536` -> `return None` (fall through to generic map path).
   - 4e. if `usize::try_from(span)` fails -> `return None`.
   - 4f. Allocate `sums[bucket_len]` = 0.0, `seen[bucket_len]` = false, `ordering` = empty vec.
   - 4g. Per-row loop (line 345 / line 406):
     - 4g-i. `Scalar::Int64(v)` -> extract key.
     - 4g-ii. `Scalar::Null(_) if dropna` -> `continue`.
     - 4g-iii. any other variant -> `return None` (defensive bail; shouldn't occur after range check).
     - 4g-iv. Compute `bucket = (key - min_key)` as usize; if `!seen[bucket]` -> mark seen, push key to ordering (first-seen order preserved).
     - 4g-v. if `value.is_missing()` -> `continue` (null/NaN values are no-op for sum).
     - 4g-vi. if `value.to_f64()` succeeds -> `sums[bucket] += v`.
     - 4g-vii. if `value.to_f64()` fails (Utf8 or Null) -> silently skipped (no error, no accumulation).
   - 4h. Output phase: iterate ordering, emit `IndexLabel::Int64(key)` and `Scalar::Float64(sums[bucket])`.
   - Edge: arena variant (line 381) is structurally identical but allocates sums/seen/ordering in a `Bump` arena; output is copied to global-allocated vecs before returning.

5. Generic map path (`groupby_sum_with_global_allocator` line 149, `groupby_sum_with_arena` line 193):
   - 5a. Entered when dense-int probe returns `None` (non-Int64 keys, span too large, or mixed types).
   - 5b. `ordering: Vec<GroupKeyRef>`, `slot: HashMap<GroupKeyRef, (usize, f64)>` (source index + running sum).
   - 5c. Per-row loop (line 166 / line 214):
     - 5c-i. if `options.dropna && key.is_missing()` -> `continue` (skip null/NaN keys).
     - 5c-ii. `GroupKeyRef::from_scalar(key)` converts key to hashable ref (line 282):
       - `Scalar::Bool(v)` -> `GroupKeyRef::Bool(v)`.
       - `Scalar::Int64(v)` -> `GroupKeyRef::Int64(v)`.
       - `Scalar::Float64(v)` -> `GroupKeyRef::FloatBits(v.to_bits())`, BUT if `v.is_nan()` -> `GroupKeyRef::FloatBits(f64::NAN.to_bits())` (canonical NaN bits, line 287).
       - `Scalar::Utf8(v)` -> `GroupKeyRef::Utf8(&str)` (zero-copy borrow).
       - `Scalar::Null(kind)` -> `GroupKeyRef::Null(kind)` (only reachable when `dropna=false`).
     - 5c-iii. `slot.entry(key_id).or_insert_with(...)`: on first-seen key, push to ordering vec and init sum to 0.0 with `source_idx = pos`.
     - 5c-iv. if `value.is_missing()` -> `continue` (null/NaN values are no-op for sum).
     - 5c-v. if `value.to_f64()` succeeds -> `entry.1 += v` (accumulate sum).
     - 5c-vi. if `value.to_f64()` fails (Utf8 or Null kind) -> silently skipped.
   - 5d. Arena variant (line 193): `ordering` is `BumpVec<GroupKeyRef>` allocated in `Bump` arena; otherwise identical logic.

6. Result emission (`emit_groupby_result`, line 243):
   - 6a. Iterates `ordering` in first-seen key order.
   - 6b. For each key, removes `(source_idx, sum)` from slot map; uses `source_keys[source_idx]` to reconstruct the output IndexLabel.
   - 6c. Index label conversion (line 256-264):
     - 6c-i. `Scalar::Int64(v)` -> `IndexLabel::Int64(v)`.
     - 6c-ii. `Scalar::Utf8(v)` -> `IndexLabel::Utf8(v.clone())`.
     - 6c-iii. `Scalar::Bool(v)` -> `IndexLabel::Utf8(v.to_string())` (stringified: `"true"` / `"false"`).
     - 6c-iv. `Scalar::Null(NullKind::NaN | NaT | Null)` -> `IndexLabel::Utf8("<null>".to_owned())`.
     - 6c-v. `Scalar::Float64(v)` -> `IndexLabel::Utf8(v.to_string())` (stringified float).
   - 6d. Output value: `Scalar::Float64(sum)` always (even if sum is 0.0 for groups with only missing values).
   - 6e. `Column::from_values(out_values)` infers dtype (all Float64 -> DType::Float64); may return `ColumnError` via `TypeError` if dtype inference fails.
   - 6f. `Series::new("sum", Index::new(out_index), out_column)` constructs output; may return `FrameError::LengthMismatch` if index/column lengths diverge (should not occur in correct implementation).
   - Edge: the `.expect("ordering references only inserted keys")` on line 254 will panic if ordering contains a key not in slot -- this is an internal invariant, not a user-facing error.

7. GroupByOptions default (line 18-22):
   - 7a. `GroupByOptions::default()` sets `dropna = true` (pandas default).
   - 7b. When `dropna = false`, null keys are treated as a distinct group; all `NullKind` variants (`Null`, `NaN`, `NaT`) collapse to a single group via `GroupKeyRef::Null(kind)` hashing (but note: different NullKind values hash differently, so `NullKind::NaN` and `NullKind::Null` are separate groups when `dropna=false`).
   - Edge: pandas 1.x treats all NA as a single group in `dropna=False`; this implementation distinguishes `NaN` vs `Null` vs `NaT` as separate groups.

8. GroupByExecutionOptions default (line 42-49):
   - 8a. `use_arena = true`, `arena_budget_bytes = 256 * 1024 * 1024` (256 MiB).
   - 8b. Arena can be disabled by passing `GroupByExecutionOptions { use_arena: false, .. }`.

## Error Ledger

1. `GroupByError::Index(IndexError::InvalidAlignmentVectors)`:
   - Trigger: `validate_alignment_plan(&plan)` fails when `left_positions.len() != right_positions.len()` or `left_positions.len() != union_index.len()`.
   - Code: `groupby_sum_with_trace` line 101; `groupby_agg` line 482.
   - pandas equivalent: internal assertion failure (should not occur with valid inputs). In pandas: no user-visible analog; alignment is always internally consistent.

2. `GroupByError::Column(ColumnError::Type(TypeError::IncompatibleDtypes { left, right }))`:
   - Trigger: `Column::from_values(out_values)` calls `infer_dtype` which calls `common_dtype` on output scalars. If output values have incompatible dtypes, this error fires.
   - Code: `emit_groupby_result` line 268; `groupby_sum_with_global_allocator` line 157; `groupby_sum_with_arena` line 204; dense path line 157/204.
   - In practice: all output values are `Scalar::Float64`, so this should never fire for `groupby_sum`. Could theoretically fire if `Column::new` dtype coercion fails.
   - pandas equivalent: `TypeError: Cannot convert <type> to numeric`.

3. `GroupByError::Frame(FrameError::LengthMismatch { index_len, column_len })`:
   - Trigger: `Series::new("sum", Index::new(out_index), out_column)` when `out_index.len() != out_column.len()`.
   - Code: `emit_groupby_result` line 269; dense path line 158/205; `groupby_agg` line 632.
   - In practice: should not occur because out_index and out_values are built in lockstep.
   - pandas equivalent: `ValueError: Length of values ({n}) does not match length of index ({m})`.

4. `GroupByError::Column(ColumnError::Type(TypeError::*))` via `reindex_by_positions`:
   - Trigger: `keys.column().reindex_by_positions(&plan.left_positions)` or `values.column().reindex_by_positions(&plan.right_positions)` during alignment materialization. The `Column::new` inside `reindex_by_positions` calls `cast_scalar` on each value which may fail on incompatible types.
   - Code: `groupby_sum_with_trace` lines 102-106; `groupby_agg` lines 483-487.
   - pandas equivalent: internal reindexing failure; no direct user-facing analog.

5. Silent skip on `value.to_f64()` failure:
   - Trigger: a value scalar is `Scalar::Utf8(...)` -- `to_f64()` returns `Err(TypeError::NonNumericValue)`. The error is consumed by the `if let Ok(v)` pattern and the value is silently ignored (not accumulated into sum).
   - Code: generic map path line 185-187 / arena path line 233-235; dense path line 362-364 / arena dense path line 423-425.
   - pandas equivalent: pandas raises `TypeError: Cannot convert <str> to numeric` at groupby time; FrankenPandas silently skips. This is a known semantic divergence.

6. Panic on internal invariant violation:
   - Trigger: `slot.remove(key).expect("ordering references only inserted keys")` fires if ordering vec contains a key not present in slot HashMap (impossible under correct logic).
   - Code: `emit_groupby_result` line 253-254; `groupby_agg` line 537-539.
   - pandas equivalent: none (internal assertion).

## Hidden Assumptions

1. Scoped aggregate is `sum` only.
2. Group key dimensionality is single-key series.
3. Dense-int path relies on bounded key-span heuristic and falls back to generic map path otherwise.

## Undefined-Behavior Edges

1. Multi-aggregate matrix (`mean`, `count`, `min`, `max`, etc.).
2. Multi-key DataFrame groupby planner semantics.
3. Full pandas `dropna`/`observed`/categorical interaction matrix.
