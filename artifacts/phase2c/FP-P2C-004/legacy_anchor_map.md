# FP-P2C-004 Legacy Anchor Map

Packet: `FP-P2C-004`
Subsystem: indexed join semantics

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/reshape/merge.py` (merge/join planning)
- `legacy_pandas_code/pandas/pandas/core/series.py` (index-driven join behavior for aligned operations)

## Extracted Behavioral Contract

1. inner joins on duplicate keys expand to cross-product cardinality.
2. left joins preserve left ordering and inject missing right values for unmatched keys.
3. hardened mode is allowed bounded continuation but strict mode remains fail-closed on unknown surfaces.

## Rust Slice Implemented

- `crates/fp-join/src/lib.rs`: `join_series` for `Inner` and `Left`
- `crates/fp-conformance/src/lib.rs`: `series_join` fixture operation and packet gate coverage

## Type Inventory

- `fp_join::JoinType`
  - variants: `Inner`, `Left`
- `fp_join::JoinedSeries`
  - fields: `index: Index`, `left_values: Column`, `right_values: Column`
- `fp_frame::Series`
  - fields: `name: String`, `index: Index`, `column: Column`
- `fp_index::Index`
  - fields: `labels: Vec<IndexLabel>`, `duplicate_cache: OnceCell<bool>`
- `fp_columnar::Column`
  - fields: `dtype: DType`, `values: Vec<Scalar>`, `validity: ValidityMask`

## Rule Ledger

1. Allocator-path selection (`join_series_with_trace`, lines 76-131):
   - 1a. `estimate_output_rows()` computes expected row count from left scan + right_map (line 99).
   - 1b. `estimate_intermediate_bytes()` multiplies `output_rows * (2*size_of::<Option<usize>>() + size_of::<IndexLabel>())` with saturating arithmetic (lines 178-184).
   - 1c. if `options.use_arena == true` AND `estimated_bytes <= options.arena_budget_bytes` -> dispatch to `join_series_with_arena` (lines 103-111).
   - 1d. else -> dispatch to `join_series_with_global_allocator` (lines 112-121).
   - Edge: `estimated_bytes` uses `saturating_mul`/`saturating_add`, so overflow clamps to `usize::MAX` and forces global-allocator fallback, never panics.

2. Right-side lookup map construction (`join_series_with_trace`, lines 82-86):
   - 2a. Builds `HashMap<&IndexLabel, Vec<usize>>` from `right.index().labels()` by enumerated position.
   - 2b. Duplicate right labels accumulate multiple positions in the same Vec entry via `or_default().push(pos)`.
   - Edge: Empty right index produces an empty HashMap; every left label will be unmatched.

3. Left-map construction guard (`join_series_with_trace`, lines 89-97):
   - 3a. if `join_type` is `Right` or `Outer` -> build analogous `HashMap<&IndexLabel, Vec<usize>>` from left index (line 89-94).
   - 3b. if `join_type` is `Inner` or `Left` -> `left_map = None` (line 96).
   - Edge: For Inner/Left scope, left_map is never allocated, saving one full index scan.

4. Output-row estimation for Inner (`estimate_output_rows`, lines 133-152):
   - 4a. For each left label, if `right_map.get(label)` returns `Some(matches)` -> contribute `matches.len()` (line 145).
   - 4b. if `right_map.get(label)` returns `None` AND join_type is `Inner` -> contribute 0 (line 147).
   - 4c. Return value: `sum(per_left_label_contributions)` (line 152).

5. Output-row estimation for Left (`estimate_output_rows`, lines 133-152):
   - 5a. For each left label, if `right_map.get(label)` returns `Some(matches)` -> contribute `matches.len()` (line 145).
   - 5b. if `right_map.get(label)` returns `None` AND join_type is `Left` -> contribute 1 (line 146).
   - 5c. Return value: `sum(per_left_label_contributions)` (line 152).

6. Inner join core loop (global allocator: lines 198-216, arena: lines 272-289):
   - 6a. Match arm: `JoinType::Inner | JoinType::Left | JoinType::Outer` (line 199/273) -- shared match arm.
   - 6b. for each `(left_pos, label)` in `left.index().labels().iter().enumerate()`:
     - 6b-i. if `right_map.get(label)` returns `Some(matches)` -> for each `right_pos` in matches: push `(label.clone(), Some(left_pos), Some(*right_pos))` to output vectors, then `continue` to next left label (lines 202-209 / 275-282).
     - 6b-ii. if no match AND `join_type` is `Left` or `Outer` -> push `(label.clone(), Some(left_pos), None)` (lines 211-215 / 284-288).
     - 6b-iii. if no match AND `join_type` is `Inner` -> skip (implicit: the `continue` on match exits early; the Left/Outer guard does not fire for Inner).
   - Edge: Cross-product expansion -- if left has `m` occurrences of label `k` and right has `n`, Inner produces `m*n` output rows for that label (each left occurrence iterates all `n` right matches).
   - Edge: Labels are cloned into `out_labels` per output row, not shared. This is O(output_rows) allocation.
   - Edge: The `continue` after the matched-branch means a label that matches right will NEVER enter the Left/Outer unmatched path, even if the match arm encompasses Left/Outer.

7. Left join unmatched-row emission (global allocator: lines 211-215, arena: lines 284-288):
   - 7a. if `matches!(join_type, JoinType::Left | JoinType::Outer)` -> emit row with `right_positions.push(None)` (line 214/287).
   - 7b. `None` in right_positions causes `reindex_by_positions` to produce `Scalar::missing_for_dtype(self.dtype)` (fp-columnar line 560).
   - Edge: The missing sentinel is dtype-aware: `Null(NullKind::Null)` for most types, `Null(NullKind::NaN)` for Float64.

8. Value materialization via `reindex_by_positions` (fp-columnar lines 551-565):
   - 8a. for each `Some(idx)` in positions -> `self.values.get(idx).cloned()`, falling back to `Scalar::missing_for_dtype` if idx is out of bounds (line 555-559).
   - 8b. for each `None` in positions -> `Scalar::missing_for_dtype(self.dtype)` (line 560).
   - 8c. Result vector is passed to `Column::new(self.dtype, values)` which may coerce types and rebuild validity mask (line 564).
   - Edge: Out-of-bounds index silently produces a missing value (via `unwrap_or_else`) rather than panicking; this is a defensive guard, not expected in normal join operation.

9. Output construction (`join_series_with_global_allocator` lines 249-256, `join_series_with_arena` lines 320-331):
   - 9a. `left_values = left.column().reindex_by_positions(&left_positions)?` -- propagates `ColumnError` on coercion failure (line 249/322).
   - 9b. `right_values = right.column().reindex_by_positions(&right_positions)?` -- propagates `ColumnError` on coercion failure (line 250/325).
   - 9c. Output `Index::new(out_labels)` constructs fresh index from collected labels (line 253/328).
   - Edge: Arena path converts `BumpVec` to slice via `.as_slice()` before passing to `reindex_by_positions` (lines 322, 325); output `Column` and `Index` are always heap-allocated regardless of arena path.

10. Output ordering invariant:
    - 10a. Inner: output rows appear in left-scan order; within each left label, right matches appear in right-index insertion order (line 203-207).
    - 10b. Left: same as Inner for matched rows; unmatched left rows appear in their original left-index position relative to surrounding matched rows (line 211-215).
    - Edge: Output is NOT sorted by label; it preserves left-scan encounter order. Duplicate labels in output are expected and valid.

## Error Ledger

1. `JoinError::Frame(FrameError)` (fp-join/src/lib.rs line 29):
   - 1a. Trigger: `FrameError::LengthMismatch { index_len, column_len }` -- raised during `Series::new()` or `Series::from_values()` if index and column lengths differ. Not triggered by join logic itself, but by upstream construction of input Series.
     - pandas equivalent: `ValueError: Length of values ({column_len}) does not match length of index ({index_len})`
     - Source: fp-frame/src/lib.rs line 16-17.
   - 1b. Trigger: `FrameError::DuplicateIndexUnsupported` -- raised if strict mode rejects duplicate index labels. Not triggered in join's current Inner/Left paths (duplicates are accepted and produce cross-product).
     - pandas equivalent: `InvalidIndexError: Reindexing only valid with uniquely valued Index objects`
     - Source: fp-frame/src/lib.rs line 18-19.
   - 1c. Trigger: `FrameError::CompatibilityRejected(String)` -- raised in `merge_dataframes` when key column `on` is missing from left or right DataFrame. Not applicable to `join_series` (series join path).
     - pandas equivalent: `KeyError: '{on}'` / `MergeError: ...`
     - Source: fp-join/src/lib.rs lines 366-375, error message: `"left DataFrame missing key column '{on}'"` / `"right DataFrame missing key column '{on}'"`.
   - 1d. Trigger: `FrameError::Column(ColumnError)` -- passthrough from column-level failures during Series construction.
     - Source: fp-frame/src/lib.rs line 22-23.
   - 1e. Trigger: `FrameError::Index(IndexError)` -- passthrough from index-level failures.
     - Source: fp-frame/src/lib.rs line 24-25.

2. `JoinError::Column(ColumnError)` (fp-join/src/lib.rs line 31):
   - 2a. Trigger: `ColumnError::LengthMismatch { left, right }` -- NOT directly triggered by join logic (join builds position vectors, not paired columns), but could propagate from `Column::new` if reindex produces values whose coerced length differs from source. In practice, `reindex_by_positions` always produces a consistent-length values vec.
     - pandas equivalent: `ValueError: Length of values does not match length of index`
     - Source: fp-columnar/src/lib.rs line 474-475.
   - 2b. Trigger: `ColumnError::Type(TypeError)` -- raised during `Column::new()` inside `reindex_by_positions` (line 564) if dtype coercion fails on the materialized values vector. This can occur if the injected `Scalar::missing_for_dtype` value cannot be cast to the column's target dtype (defensive edge; not expected under normal operation since missing sentinels are dtype-compatible).
     - pandas equivalent: `TypeError: Cannot convert ...`
     - Source: fp-columnar/src/lib.rs line 476-477, triggered via `cast_scalar_owned` in `Column::new` (line 493).

3. Error propagation path for `join_series` Inner/Left:
   - 3a. `join_series` (line 58-64) -> `join_series_with_options` (line 66-74) -> `join_series_with_trace` (line 76-131).
   - 3b. `join_series_with_trace` dispatches to `join_series_with_arena` or `join_series_with_global_allocator`, both return `Result<JoinedSeries, JoinError>`.
   - 3c. `left.column().reindex_by_positions(&left_positions)?` -- `?` converts `ColumnError` to `JoinError::Column` via `From` impl (line 249/322).
   - 3d. `right.column().reindex_by_positions(&right_positions)?` -- same conversion (line 250/325).
   - 3e. No panics in the join path for Inner/Left; all error surfaces return `Result`.
   - Edge: The `expect("left_map required for Right join")` (lines 231, 302) is unreachable for Inner/Left because `left_map` is only consumed in the `JoinType::Right` match arm, and the construction guard (rule 3b) ensures it is `Some` when Right is selected.

4. Fail-closed join mode surface:
   - 4a. `JoinType` is a closed enum with four variants: `Inner`, `Left`, `Right`, `Outer` (lines 12-17).
   - 4b. All match arms in the join core are exhaustive (`Inner | Left | Outer` and `Right`); no wildcard/default arm exists.
   - 4c. Adding a new variant to `JoinType` would produce a compile-time error at every match site, enforcing fail-closed behavior at the type level.
   - Edge: There is no runtime "unknown join type" error path; invalid modes are rejected at compile time by Rust's exhaustive pattern matching.

## Hidden Assumptions

1. Packet scope is indexed series join core, not full DataFrame merge planner.
2. Join mode surface is intentionally constrained to `Inner`/`Left`.
3. Cardinality expansion from duplicates is accepted as scoped behavior, not abuse mitigation.

## Undefined-Behavior Edges

1. Full multi-column DataFrame merge behavior matrix.
2. Non-scoped join mode semantics and sort option matrix.
3. Advanced null-key equivalence rules beyond scoped fixtures.
