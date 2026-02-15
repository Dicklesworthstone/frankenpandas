# FP-P2C-001 Legacy Anchor Map

Packet: `FP-P2C-001`
Subsystem: DataFrame/Series construction + alignment

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/frame.py` (`DataFrame`, `_from_nested_dict`, `_reindex_for_setitem`)
- `legacy_pandas_code/pandas/pandas/core/series.py` (`Series` construction and aligned binary arithmetic)

## Extracted Behavioral Contract

1. Index alignment is label-driven and must materialize a deterministic union before arithmetic.
2. Missing labels introduce missing values, not dropped rows.
3. Duplicate index handling is compatibility-critical and must be mode-gated.

## Rust Slice Implemented

- `crates/fp-frame/src/lib.rs`: `Series::from_values`, `Series::add_with_policy`, `DataFrame::from_series`
- `crates/fp-index/src/lib.rs`: deterministic union alignment plan (`align_union`)
- `crates/fp-columnar/src/lib.rs`: reindexing and missing propagation in numeric ops

## Type Inventory

- `fp_index::IndexLabel`
  - variants: `Int64(i64)`, `Utf8(String)`
- `fp_index::Index`
  - fields: `labels: Vec<IndexLabel>`, `duplicate_cache: OnceCell<bool>`
- `fp_index::AlignmentPlan`
  - fields: `union_index: Index`, `left_positions: Vec<Option<usize>>`, `right_positions: Vec<Option<usize>>`
- `fp_frame::Series`
  - fields: `name: String`, `index: Index`, `column: Column`
- `fp_frame::DataFrame`
  - fields: `index: Index`, `columns: BTreeMap<String, Column>`
- `fp_columnar::Column`
  - fields: `dtype: DType`, `values: Vec<Scalar>`, `validity: ValidityMask`

## Rule Ledger

1. Duplicate-label gating (`binary_op_with_policy`, line 114):
   - 1a. if `left.has_duplicates() && right.has_duplicates()` and mode=strict -> reject `DuplicateIndexUnsupported`.
   - 1b. if `left.has_duplicates() && !right.has_duplicates()` and mode=strict -> reject `DuplicateIndexUnsupported`.
   - 1c. if `!left.has_duplicates() && right.has_duplicates()` and mode=strict -> reject `DuplicateIndexUnsupported`.
   - 1d. if either has duplicates and mode=hardened -> `decide_unknown_feature` logs to ledger, then continue.
   - 1e. if neither has duplicates -> proceed to rule 2 (no decision record emitted).
   - Note: `has_duplicates()` is memoized via `OnceCell` (AG-05), cost is O(n) first call, O(1) subsequent.

2. Alignment union (`align_union`, fp-index line 519):
   - 2a. Build `position_map_first_ref` for left and right (HashMap<&IndexLabel, usize>, first occurrence).
   - 2b. `union_labels = left.labels.clone()` (preserves left order including duplicates).
   - 2c. For each label in right: if NOT in left_positions_map, append to union_labels.
   - 2d. `left_positions[i] = left_positions_map.get(union_labels[i])` -> `Some(pos)` or `None`.
   - 2e. `right_positions[i] = right_positions_map.get(union_labels[i])` -> `Some(pos)` or `None`.
   - Edge: if left is empty, union = right labels in right order.
   - Edge: if right is empty, union = left labels in left order.
   - Edge: if both empty, union is empty.
   - Edge: duplicate labels in left are preserved; right duplicates only appear if the label is absent from left.

3. Plan validation (`validate_alignment_plan`, fp-index line 548):
   - 3a. if `left_positions.len() != right_positions.len()` -> `InvalidAlignmentVectors`.
   - 3b. if `left_positions.len() != union_index.len()` -> `InvalidAlignmentVectors`.
   - 3c. Otherwise pass (no additional validation on position values).

4. Alignment materialization (`reindex_by_positions`, fp-columnar line 551):
   - 4a. For each position slot `Some(idx)`: clone `values[idx]`; if idx is out of bounds, inject `missing_for_dtype`.
   - 4b. For each position slot `None`: inject `Scalar::missing_for_dtype(self.dtype)`.
   - 4c. Missing value by dtype:
     - `Float64` -> `Scalar::Null(NullKind::NaN)` (pandas convention: float missing is NaN).
     - `Int64` -> `Scalar::Null(NullKind::Null)`.
     - `Bool` -> `Scalar::Null(NullKind::Null)`.
     - `Utf8` -> `Scalar::Null(NullKind::Null)`.
     - `Null` -> `Scalar::Null(NullKind::Null)`.
   - 4d. Construct new Column with same dtype and new values.

5. Runtime admission gate (`decide_join_admission`, line 131):
   - 5a. Policy evaluates union_index.len() against admission threshold.
   - 5b. If `DecisionAction::Reject` -> return `FrameError::CompatibilityRejected`.
   - 5c. If `DecisionAction::Allow` -> proceed to arithmetic.
   - 5d. Decision is recorded in evidence ledger.

6. Arithmetic execution (`binary_numeric`, fp-columnar):
   - 6a. Compute `out_dtype = common_dtype(left.dtype, right.dtype)`.
     - Int64 + Int64 -> Int64 (for Add/Sub/Mul); Int64 / Int64 -> Float64 (for Div).
     - Int64 + Float64 -> Float64 (promotion).
     - Float64 + Float64 -> Float64.
     - Bool/Utf8 + numeric -> error (unsupported arithmetic).
   - 6b. Try vectorized path (`try_vectorized_binary`): if both columns have contiguous typed arrays of same type, operate on raw slices for SIMD auto-vectorization.
   - 6c. Scalar fallback: element-wise arithmetic with validity mask propagation.
   - 6d. Missing propagation: `invalid & invalid -> invalid`, `invalid & valid -> invalid`, `valid & invalid -> invalid`, `valid & valid -> apply op`.
   - 6e. NaN propagation: if either operand is NaN, result is NaN (for Float64).

7. Column naming (`binary_op_with_policy`, line 140-151):
   - 7a. if `self.name == other.name` -> keep `self.name`.
   - 7b. if `self.name != other.name` -> `format!("{}{op_symbol}{}", self.name, other.name)`.
   - 7c. op_symbol: Add="+", Sub="-", Mul="*", Div="/".

## Error Ledger

1. `FrameError::DuplicateIndexUnsupported`:
   - Trigger: either operand has duplicate labels AND mode=strict.
   - Code: `binary_op_with_policy` line 121.
   - pandas equivalent: `ValueError: cannot reindex on an axis with duplicate labels` (when `allow_duplicates=False`).

2. `FrameError::CompatibilityRejected(String)`:
   - Trigger: runtime policy `decide_join_admission` returns `Reject`.
   - Code: `binary_op_with_policy` line 133.
   - Message: "runtime policy rejected alignment admission".
   - pandas equivalent: no direct equivalent (FrankenPandas safety guard).

3. `FrameError::LengthMismatch { index_len, column_len }`:
   - Trigger: `Series::new` when index length != column length.
   - Code: `Series::new` constructor.
   - pandas equivalent: `ValueError: Length of values ({n}) does not match length of index ({m})`.

4. `IndexError::InvalidAlignmentVectors`:
   - Trigger: `validate_alignment_plan` when position vector lengths are inconsistent.
   - Code: `validate_alignment_plan` line 549.
   - pandas equivalent: internal assertion (should not occur in correct implementation).

5. `ColumnError::IncompatibleTypes { left, right }`:
   - Trigger: `binary_numeric` when dtypes cannot be promoted (e.g., Bool + Int64).
   - Code: `binary_numeric` via `common_dtype`.
   - pandas equivalent: `TypeError: unsupported operand type(s)`.

6. `ColumnError::LengthMismatch`:
   - Trigger: `binary_numeric` when left/right columns have different lengths (post-reindex).
   - Code: should not occur after valid alignment plan.

## Hidden Assumptions

1. Packet scope is binary Series arithmetic with explicit labels, not full DataFrame arithmetic matrix.
2. Duplicate-label semantics outside the hardened allowlist are intentionally unsupported in strict mode.
3. Missing propagation semantics rely on `Scalar::missing_for_dtype` and current dtype coercion rules.

## Undefined-Behavior Edges

1. Full pandas duplicate-label behavioral matrix beyond current gate.
2. Full broadcast semantics (`Series`-`DataFrame`, scalar broadcasting edge paths).
3. Full indexing surface (`loc`/`iloc`) interaction with arithmetic alignment.
