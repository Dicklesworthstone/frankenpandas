# FP-P2C-002 Legacy Anchor Map

Packet: `FP-P2C-002`
Subsystem: Index model and indexer semantics

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/indexes/base.py` (`Index`, `ensure_index`, `_validate_join_method`)

## Rust Slice Implemented

- `crates/fp-index/src/lib.rs`: `IndexLabel`, `Index`, `align_union`, duplicate detection
- `crates/fp-frame/src/lib.rs`: strict/hardened duplicate-index compatibility gate

## Deferred

- full `get_indexer` method matrix
- `MultiIndex` tuple semantics
- full error-string parity against pandas

## Type Inventory

- `fp_index::IndexLabel`
  - variants: `Int64(i64)`, `Utf8(String)`
- `fp_index::Index`
  - fields: `labels: Vec<IndexLabel>`, `duplicate_cache: OnceCell<bool>`
- `fp_index::AlignmentPlan`
  - fields: `union_index: Index`, `left_positions: Vec<Option<usize>>`, `right_positions: Vec<Option<usize>>`
- `fp_index::IndexError`
  - variants: `InvalidAlignmentVectors`

## Rule Ledger

1. Duplicate detection (`has_duplicates`, line 171):
   - 1a. `OnceCell::get_or_init` path -- if cache is populated, returns `*cached_bool` without recomputing (line 172-175).
   - 1b. Cold-path calls `detect_duplicates` (line 125-133): iterates labels with `HashMap<&IndexLabel, ()>`, returns `true` on first `insert` collision (line 128-129).
   - 1c. Empty index: loop body never executes, `detect_duplicates` returns `false` (line 132).
   - 1d. No-duplicate index: full scan completes, returns `false` (line 132).

2. Position map semantics (`position_map_first`, line 233; `position_map_first_ref`, line 241):
   - 2a. Both use `HashMap::entry().or_insert(idx)` -- first occurrence wins for duplicate labels (lines 236, 244).
   - 2b. Capacity pre-allocated to `self.labels.len()` (lines 234, 242), which over-allocates when duplicates exist.
   - 2c. Return type difference: `position_map_first` clones keys into owned `HashMap<IndexLabel, usize>` (line 233); `position_map_first_ref` borrows keys as `HashMap<&IndexLabel, usize>` (line 241).

3. Adaptive position lookup (`position`, line 196):
   - 3a. `SortOrder::AscendingInt64` arm (line 198):
     - 3a-i. if needle is `Int64(target)` -> binary search over labels comparing `Int64` variants (lines 200-208), returns `Ok(pos)` as `Some(pos)` or `Err(_)` as `None`.
     - 3a-ii. if needle is `Utf8(_)` -> returns `None` immediately, type mismatch (line 210).
   - 3b. `SortOrder::AscendingUtf8` arm (line 213):
     - 3b-i. if needle is `Utf8(target)` -> binary search comparing `Utf8` variants by `str` (lines 215-224).
     - 3b-ii. if needle is `Int64(_)` -> returns `None` immediately (line 226).
   - 3c. `SortOrder::Unsorted` arm (line 228): linear scan via `iter().position()`, returns first match or `None`.
   - 3d. Binary search comparator fallback: non-matching variant arms return `Ordering::Less` (lines 205, 220), which is unreachable for homogeneous sorted indexes but prevents panic on mixed types.

4. Sort order detection (`detect_sort_order`, line 59):
   - 4a. Length <= 1 (line 60-64):
     - 4a-i. `Some(Int64(_))` or `None` (empty) -> `AscendingInt64`.
     - 4a-ii. `Some(Utf8(_))` -> `AscendingUtf8`.
   - 4b. All-Int64 check (line 68-80): if all labels are `Int64` AND `windows(2)` are strictly `a < b` -> `AscendingInt64`.
   - 4c. All-Utf8 check (line 83-95): if all labels are `Utf8` AND `windows(2)` are strictly `a < b` -> `AscendingUtf8`.
   - 4d. Fallthrough -> `Unsorted` (line 97). This covers: mixed types, non-strictly-ascending, duplicates.
   - 4e. Sort order cached via `OnceCell` in `sort_order()` (line 179-183).

5. Union alignment (`align_union`, line 519):
   - 5a. Build `left_positions_map` and `right_positions_map` via `position_map_first_ref` (lines 520-521).
   - 5b. Union label construction (lines 523-529):
     - 5b-i. Start with all left labels (cloned in order, including duplicates) (line 524).
     - 5b-ii. Append right labels NOT present in `left_positions_map` (lines 525-529).
   - 5c. Left position vector: for each union label, look up in `left_positions_map`; `Some(pos)` if found, `None` if right-only (lines 531-534).
   - 5d. Right position vector: for each union label, look up in `right_positions_map`; `Some(pos)` if found, `None` if left-only (lines 536-539).
   - 5e. Duplicate-label edge: if left has duplicates, union labels contain all duplicate entries, but `left_positions_map` maps each to first-occurrence only -- downstream reindex will replicate the first-occurrence value for all duplicate positions.

6. Inner alignment (`align_inner`, line 478):
   - 6a. Build `right_map` via `right.position_map_first_ref()` (line 479).
   - 6b. For each `(left_pos, label)` in left (line 485):
     - 6b-i. if `right_map` contains label -> emit `(Some(left_pos), Some(right_pos))` into output (lines 486-490).
     - 6b-ii. if not in `right_map` -> skip, label excluded from output.
   - 6c. Duplicate-label edge: if left has duplicate labels, each occurrence is tested independently against `right_map`, but `right_map` always returns the same first-occurrence position for right.

7. Left alignment (`align_left`, line 501):
   - 7a. Build `right_map` via `right.position_map_first_ref()` (line 502).
   - 7b. For each `(left_pos, label)` in left (line 507):
     - 7b-i. `left_positions` always gets `Some(left_pos)` (line 508).
     - 7b-ii. `right_positions` gets `right_map.get(label).copied()` -- `Some(pos)` if present, `None` if missing (line 509).
   - 7c. Output `union_index` is a clone of the left index (line 513).

8. Right alignment (`align`, `AlignMode::Right`, line 465):
   - 8a. Implemented by swapping: calls `align_left(right, left)` then swaps `left_positions` and `right_positions` (lines 466-471).

9. Plan validation (`validate_alignment_plan`, line 548):
   - 9a. if `left_positions.len() != right_positions.len()` OR `left_positions.len() != union_index.len()` -> `Err(InvalidAlignmentVectors)` (lines 549-552).
   - 9b. else -> `Ok(())` (line 555).

10. Upstream duplicate-index gating (`binary_op_with_policy`, fp-frame line 114):
    - 10a. if `self.index.has_duplicates() || other.index.has_duplicates()` (line 114):
      - 10a-i. Always logs via `policy.decide_unknown_feature(...)` (lines 115-119).
      - 10a-ii. if `policy.mode == RuntimeMode::Strict` -> `Err(FrameError::DuplicateIndexUnsupported)` (lines 120-122).
      - 10a-iii. if non-strict mode -> falls through, duplicate labels tolerated with first-occurrence semantics.
    - 10b. After alignment, `validate_alignment_plan(&plan)?` propagates `IndexError::InvalidAlignmentVectors` as `FrameError::Index(...)` (fp-frame line 126).
    - 10c. `policy.decide_join_admission(plan.union_index.len(), ledger)` (fp-frame line 131):
      - 10c-i. if `DecisionAction::Reject` -> `Err(FrameError::CompatibilityRejected("runtime policy rejected alignment admission"))` (lines 132-135).
      - 10c-ii. if `DecisionAction::Allow` -> proceeds to arithmetic.

## Error Ledger

### fp-index errors (`IndexError`, fp-index/src/lib.rs line 438-442)

1. `IndexError::InvalidAlignmentVectors` (line 441):
   - Error string: `"alignment vectors must have equal lengths"` (line 441).
   - Trigger: `validate_alignment_plan` (line 548) when `left_positions.len() != right_positions.len()` OR `left_positions.len() != union_index.len()` (line 549-551).
   - pandas equivalent: No direct pandas equivalent; pandas raises `ValueError: operands could not be broadcast together` for shape mismatches in reindexing, but this is an internal invariant check not exposed to users. In pandas `Index.reindex()`, a shape mismatch produces `ValueError: cannot reindex from a duplicate axis`.
   - Note: Under normal `align_union`/`align_inner`/`align_left` construction paths, this error is unreachable since the functions always produce vectors of matching length. It serves as a defensive post-condition check.

### fp-frame errors (`FrameError`, fp-frame/src/lib.rs line 14-26)

2. `FrameError::LengthMismatch { index_len, column_len }` (line 17):
   - Error string: `"index length ({index_len}) does not match column length ({column_len})"` (line 16).
   - Trigger locations:
     - `Series::new` (line 37-41): when `index.len() != column.len()`.
     - `DataFrame::new` (line 624-630): when any column's `len() != index.len()`.
     - `DataFrame::from_dict` (line 688): when any column's value count != inferred row count.
     - `DataFrame::from_dict_with_index` (line 724): when any column's value count != index label count.
     - `DataFrame::with_column` (line 870): when new column's `len() != self.len()`.
   - pandas equivalent: `ValueError: Length of values ({n}) does not match length of index ({m})`.

3. `FrameError::DuplicateIndexUnsupported` (line 19):
   - Error string: `"duplicate index labels are unsupported in strict mode for MVP slice"` (line 18).
   - Trigger: `binary_op_with_policy` (line 114-122) when either operand's index `has_duplicates()` returns `true` AND `policy.mode == RuntimeMode::Strict`.
   - pandas equivalent: pandas does not reject duplicate indexes for arithmetic. This is a strictness gate unique to FrankenPandas MVP scope. Closest pandas behavior is `InvalidIndexError: Reindexing only valid with uniquely valued Index objects` which pandas raises in specific `reindex` paths.

4. `FrameError::CompatibilityRejected(String)` (line 21):
   - Error string: `"compatibility gate rejected operation: {reason}"` (line 20).
   - Trigger locations:
     - `binary_op_with_policy` (fp-frame line 132-135): when `policy.decide_join_admission()` returns `DecisionAction::Reject`, with message `"runtime policy rejected alignment admission"`.
     - `concat_dataframes` (fp-frame line 589): when frame column names don't match frame[0], with message `"frame[{i}] columns do not match frame[0]"`.
     - `DataFrame::select_columns` (fp-frame line 701): when a requested column name is not found, with message `"column '{col}' not found in data"`.
     - `DataFrame::rename_column` (fp-frame line 746): when the source column name is not found, with message `"column '{name}' not found"`.
     - `DataFrame::drop_column` (fp-frame line 885): when the column to drop is not found, with message `"column '{name}' not found"`.
   - pandas equivalent: varies by trigger; closest are `KeyError: '{col}'` for missing columns, `ValueError` for concat shape mismatches.

5. `FrameError::Column(ColumnError)` (line 23):
   - Transparent passthrough from `fp_columnar::ColumnError` via `#[from]`.
   - Trigger: any `Column` operation failure (type mismatch in arithmetic, reindex errors, etc.) propagated through `?` operator.

6. `FrameError::Index(IndexError)` (line 25):
   - Transparent passthrough from `fp_index::IndexError` via `#[from]`.
   - Trigger: `validate_alignment_plan(&plan)?` in `binary_op_with_policy` (fp-frame line 126) converts `IndexError::InvalidAlignmentVectors` into this variant.

### Implicit non-error edges (silent degradation)

7. Duplicate labels in non-strict mode:
   - When `has_duplicates()` is true but `policy.mode != Strict`, `binary_op_with_policy` logs to `EvidenceLedger` via `decide_unknown_feature` but proceeds with first-occurrence alignment semantics. No error is raised; behavior silently picks the first matching label position (Rule 2a, 5e).

8. Type-mismatch in `position()` for sorted indexes:
   - When a needle's variant (`Int64` vs `Utf8`) does not match the sorted index's type, `position()` returns `None` silently (Rule 3a-ii, 3b-ii). No error is raised. This matches pandas behavior where `index.get_loc(mismatched_type)` raises `KeyError`, but here the `None` is propagated as a missing-label signal to alignment.

## Hidden Assumptions

1. Packet scope assumes flat single-level index only.
2. Null/NaN marker handling is delegated to higher layers; index packet treats labels as opaque atoms.
3. First-occurrence selection is acceptable for scoped duplicate handling while full pandas matrix is deferred.

## Undefined-Behavior Edges

1. Full `get_indexer` family behavior matrix.
2. `MultiIndex` tuple semantics and hierarchical lookup.
3. Partial-string/timezone-sensitive index slicing semantics.
