# FP-P2C-003 Legacy Anchor Map

Packet: `FP-P2C-003`
Subsystem: Series arithmetic + mixed-label alignment

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/series.py` (aligned binary arithmetic semantics)
- `legacy_pandas_code/pandas/pandas/core/indexes/base.py` (index union behavior and duplicate-label handling)

## Extracted Behavioral Contract

1. Alignment is label-driven and deterministic for union materialization.
2. Non-overlapping labels become missing values in arithmetic outputs.
3. Hardened duplicate-label path is explicit and auditable; strict mode remains fail-closed.

## Rust Slice Implemented

- `crates/fp-frame/src/lib.rs`: `Series::add_with_policy`
- `crates/fp-index/src/lib.rs`: `align_union`
- `crates/fp-conformance/src/lib.rs`: packetized differential fixture execution

## Type Inventory

- `fp_frame::Series`
  - fields: `name: String`, `index: Index`, `column: Column`
- `fp_index::Index`
  - fields: `labels: Vec<IndexLabel>`, `duplicate_cache: OnceCell<bool>`
- `fp_index::AlignmentPlan`
  - fields: `union_index: Index`, `left_positions: Vec<Option<usize>>`, `right_positions: Vec<Option<usize>>`
- `fp_columnar::Column`
  - fields: `dtype: DType`, `values: Vec<Scalar>`, `validity: ValidityMask`

## Rule Ledger

1. Duplicate-label gate (`binary_op_with_policy`, fp-frame line 114):
   - 1a. if `self.index.has_duplicates() || other.index.has_duplicates()` -> call `policy.decide_unknown_feature("index_alignment", ...)` which logs to evidence ledger.
   - 1b. if 1a fires AND `matches!(policy.mode, RuntimeMode::Strict)` -> return `Err(FrameError::DuplicateIndexUnsupported)` (line 121).
   - 1c. if 1a fires AND mode=Hardened -> `decide_unknown_feature` records the decision, but execution continues past the gate (line 122-123 fall through).
   - 1d. if neither index has duplicates -> skip the entire block (no decision record emitted), proceed to rule 2.
   - Edge: `has_duplicates()` is memoized via `OnceCell` (line 172 in fp-index). First call is O(n) via `detect_duplicates` (HashMap insertion test, line 125-133); subsequent calls are O(1).
   - Edge: duplicate detection uses `HashMap<&IndexLabel, ()>` with `insert().is_some()` early-return, so the check short-circuits on first duplicate found.

2. Deterministic union alignment (`align_union`, fp-index line 519):
   - 2a. Build `left_positions_map = left.position_map_first_ref()` (HashMap<&IndexLabel, usize>, first-occurrence semantics, line 520).
   - 2b. Build `right_positions_map = right.position_map_first_ref()` (same first-occurrence semantics, line 521).
   - 2c. Initialize `union_labels` with `left.labels.iter().cloned()` -- preserves all left labels including duplicates, in left order (line 523-524).
   - 2d. For each label in `right.labels`: if `!left_positions_map.contains_key(&label)` -> append to `union_labels` (line 525-529). Right-only labels appear in their original right-side order.
   - 2e. Map each union label to left position: `left_positions[i] = left_positions_map.get(union_labels[i]).copied()` -> `Some(pos)` for labels in left, `None` for right-only labels (line 531-534).
   - 2f. Map each union label to right position: `right_positions[i] = right_positions_map.get(union_labels[i]).copied()` -> `Some(pos)` for labels in right, `None` for left-only labels (line 536-539).
   - Edge: if left is empty, union = right labels in right order (2c produces nothing, 2d appends all of right).
   - Edge: if right is empty, union = left labels in left order (2d appends nothing).
   - Edge: if both empty, union is empty, both position vectors are empty.
   - Edge: duplicate labels in left are ALL preserved in union (since 2c clones all); however position_map_first_ref uses `entry().or_insert()` (line 241-246), so `left_positions[i]` for a duplicate label always maps to the FIRST occurrence position, not the actual position of the duplicate. This means duplicate left labels will all share the same source position in the reindexed column.
   - Edge: right duplicate labels only appear in union if the label is entirely absent from left. If the label exists in left, all right duplicates are suppressed by the `contains_key` check (line 526).

3. Alignment plan validation (`validate_alignment_plan`, fp-index line 548):
   - 3a. if `plan.left_positions.len() != plan.right_positions.len()` -> return `Err(IndexError::InvalidAlignmentVectors)` (line 549-550).
   - 3b. if `plan.left_positions.len() != plan.union_index.len()` -> return `Err(IndexError::InvalidAlignmentVectors)` (line 550).
   - 3c. Otherwise return `Ok(())` -- no bounds checking on position values themselves (line 555).

4. Column reindexing (`reindex_by_positions`, fp-columnar line 551):
   - 4a. For each slot `Some(idx)`: if `idx < self.values.len()` -> clone `self.values[idx]`; if `idx >= self.values.len()` (out of bounds) -> inject `Scalar::missing_for_dtype(self.dtype)` via `unwrap_or_else` (line 555-559).
   - 4b. For each slot `None` -> inject `Scalar::missing_for_dtype(self.dtype)` (line 560).
   - 4c. Construct result via `Self::new(self.dtype, values)` which recomputes ValidityMask from the new values (line 564).
   - Edge: the dtype of the output column is always the same as the source column's dtype. Missing marker type depends on dtype:
     - `Float64` -> `Scalar::Null(NullKind::NaN)` (pandas convention: float missing is NaN).
     - `Int64` -> `Scalar::Null(NullKind::Null)`.
     - `Bool` -> `Scalar::Null(NullKind::Null)`.
     - `Utf8` -> `Scalar::Null(NullKind::Null)`.
     - `Null` -> `Scalar::Null(NullKind::Null)`.

5. Runtime admission gate (`binary_op_with_policy`, fp-frame line 131):
   - 5a. `action = policy.decide_join_admission(plan.union_index.len(), ledger)` evaluates cardinality against policy thresholds.
   - 5b. if `matches!(action, DecisionAction::Reject)` -> return `Err(FrameError::CompatibilityRejected("runtime policy rejected alignment admission"))` (line 132-135).
   - 5c. if `action == DecisionAction::Allow` -> proceed to arithmetic (line 136 fall-through).
   - 5d. if `action == DecisionAction::Repair` -> also proceeds (Repair is not Reject, so the `matches!` on line 132 does not fire); repair is advisory only.
   - Edge: `decide_join_admission` (fp-runtime line 206): in Strict mode, uses Bayes decision theory with LossMatrix. In Hardened mode with `hardened_join_row_cap`, if `estimated_rows > cap` -> forces `DecisionAction::Repair` (runtime line 242-244). If `estimated_rows <= cap` -> Bayes argmin typically yields `Allow`.
   - Edge: the decision record is always appended to the evidence ledger regardless of outcome (runtime line 247).

6. Arithmetic execution (`binary_numeric`, fp-columnar line 667):
   - 6a. if `self.len() != right.len()` -> return `Err(ColumnError::LengthMismatch { left, right })` (line 668-673). This should never fire after valid alignment but is a defensive check.
   - 6b. Compute `out_dtype = common_dtype(self.dtype, right.dtype)?` (line 675). Error if types are incompatible (e.g., Utf8 + Int64).
   - 6c. if `matches!(out_dtype, DType::Bool)` -> promote `out_dtype = DType::Int64` (line 676-678). Bool is treated as integer for arithmetic.
   - 6d. if `matches!(op, ArithmeticOp::Div)` -> force `out_dtype = DType::Float64` (line 679-681). Division always produces floats, matching pandas semantics.
   - 6e. Try vectorized path `try_vectorized_binary(right, op, out_dtype)` (line 684):
     - 6e-i. if `out_dtype == Float64`: build `ColumnData::Float64` from both sides, compute `nan_aware_validity()` masks, call `vectorized_binary_f64` (line 581-616). NaN-vs-Null distinction preserved in output: if either input is NaN at position i, output is `Scalar::Null(NullKind::NaN)`, else `Scalar::missing_for_dtype(out_dtype)` (line 603-609).
     - 6e-ii. if `out_dtype == Int64 && op != Div`: both columns must actually be `DType::Int64` (line 620-621). Calls `vectorized_binary_i64` which uses `wrapping_add/sub/mul` (line 419-421). Division returns `None` to force scalar fallback (line 412-413).
     - 6e-iii. otherwise (Bool, Utf8, mixed types not matching fast path) -> return `None` to signal scalar fallback (line 646).
   - 6f. Scalar fallback path (line 688-724):
     - 6f-i. For each element pair `(left, right)`: if `left.is_missing() || right.is_missing()` -> result is `Scalar::Null(NullKind::NaN)` if either is NaN, else `Scalar::missing_for_dtype(out_dtype)` (line 694-700).
     - 6f-ii. Both valid: convert to f64 via `to_f64()`, apply operation, check if result fits Int64 representation when `out_dtype == Int64` (line 711-717). If result is finite, equals its truncation, and fits i64 range -> produce `Scalar::Int64`, else `Scalar::Float64`.
   - 6g. Final column construction: `Self::new(out_dtype, values)` recomputes ValidityMask (line 724).
   - Edge: division by zero produces `f64::INFINITY` or `f64::NEG_INFINITY` (IEEE 754), not an error.
   - Edge: `0.0 / 0.0` produces `NaN`, which is a valid Float64 output.

7. Output Series naming (`binary_op_with_policy`, fp-frame line 140-151):
   - 7a. if `self.name == other.name` -> output name = `self.name.clone()` (line 148).
   - 7b. if `self.name != other.name` -> output name = `format!("{}{op_symbol}{}", self.name, other.name)` (line 150).
   - 7c. `op_symbol` match arm (line 140-145): `ArithmeticOp::Add` -> `"+"`, `Sub` -> `"-"`, `Mul` -> `"*"`, `Div` -> `"/"`.

8. Output Series construction (`binary_op_with_policy`, fp-frame line 153):
   - 8a. `Series::new(out_name, plan.union_index, column)` validates that union index length matches the result column length.
   - 8b. if lengths mismatch -> return `Err(FrameError::LengthMismatch { ... })`. This is a structural invariant failure that should never occur if alignment is correct.

9. Convenience wrappers (`add`, `sub`, `mul`, `div`; fp-frame lines 165-210):
   - 9a. `add(&self, other)` -> calls `binary_op_with_policy` with `RuntimePolicy::strict()` and a fresh `EvidenceLedger`. This means the default (no-policy) path is strict mode, which will reject duplicate labels (line 167).
   - 9b. `add_with_policy`, `sub_with_policy`, `mul_with_policy`, `div_with_policy` -> each delegates to `binary_op_with_policy` with the respective `ArithmeticOp` variant. All share identical alignment/gate logic.

10. `position_map_first_ref` first-match semantics (fp-index line 241):
    - 10a. Uses `HashMap::entry(label).or_insert(idx)` -- only records the FIRST position for each label (line 245).
    - 10b. Subsequent occurrences of the same label are silently ignored.
    - Edge: this is the core duplicate-label behavior for alignment. When duplicates exist, alignment always maps to the first occurrence. This matches pandas `get_indexer` default behavior but diverges from the full pandas duplicate expansion (cartesian product for duplicates), which is excluded from this packet's scope.

## Error Ledger

1. `FrameError::DuplicateIndexUnsupported`:
   - Variant: unit struct (no fields).
   - Message string: `"duplicate index labels are unsupported in strict mode for MVP slice"` (fp-frame line 18-19).
   - Trigger: `binary_op_with_policy` line 121: `(self.index.has_duplicates() || other.index.has_duplicates()) && matches!(policy.mode, RuntimeMode::Strict)`.
   - Exact condition: either left OR right index has at least one label appearing more than once, AND the policy mode is `Strict`.
   - pandas equivalent: `ValueError: cannot reindex on an axis with duplicate labels` (when `allow_duplicates=False`).
   - Note: in Hardened mode, this error is never raised; execution continues past the gate.

2. `FrameError::CompatibilityRejected(String)`:
   - Variant: tuple struct wrapping a `String` message.
   - Message string: `"runtime policy rejected alignment admission"` (fp-frame line 134).
   - Trigger: `binary_op_with_policy` line 132-135: `matches!(action, DecisionAction::Reject)` after `policy.decide_join_admission(plan.union_index.len(), ledger)`.
   - Exact condition: the Bayesian decision engine (fp-runtime `decide` function line 272) determines that the estimated row count exceeds acceptable risk thresholds under the current loss matrix. In Strict mode with `fail_closed_unknown_features=true`, the prior is set low (0.25) and evidence heavily penalizes unknown features. In Hardened mode with a cap, large row counts are forced to `Repair` (not `Reject`), so this error primarily fires in Strict mode for large union cardinalities.
   - pandas equivalent: no direct equivalent (FrankenPandas safety guard).

3. `FrameError::LengthMismatch { index_len: usize, column_len: usize }`:
   - Variant: struct with named fields `index_len` and `column_len`.
   - Message string: `"index length ({index_len}) does not match column length ({column_len})"` (fp-frame line 16-17).
   - Trigger locations:
     - `Series::new` line 37-42: if `index.len() != column.len()`.
     - `DataFrame::new` line 624-631: if any column length != index length.
     - `DataFrame::from_dict` line 687-690: if any value vector length != first vector length.
     - `DataFrame::from_dict_with_index` line 723-726: if any value vector length != index_labels length.
     - `DataFrame::with_column` line 869-873: if new column length != DataFrame row count.
   - pandas equivalent: `ValueError: Length of values ({n}) does not match length of index ({m})`.

4. `IndexError::InvalidAlignmentVectors`:
   - Variant: unit struct (no fields).
   - Message string: `"alignment vectors must have equal lengths"` (fp-index line 440-441).
   - Trigger: `validate_alignment_plan` line 549-553: if `left_positions.len() != right_positions.len()` OR `left_positions.len() != union_index.len()`.
   - Exact condition: any of the three lengths (left_positions, right_positions, union_index.labels) differ from each other.
   - pandas equivalent: internal assertion (should not occur in correct implementation; signals a bug in alignment logic).
   - Propagation: wrapped via `#[error(transparent)]` and `#[from] IndexError` in `FrameError::Index` (fp-frame line 24-25).

5. `ColumnError::LengthMismatch { left: usize, right: usize }`:
   - Variant: struct with named fields `left` and `right`.
   - Message string: `"column length mismatch: left={left}, right={right}"` (fp-columnar line 474-475).
   - Trigger locations:
     - `binary_numeric` line 668-673: if `self.len() != right.len()` (defensive check post-alignment).
     - `binary_comparison` line 732-737: same defensive length check.
     - `filter_by_mask` line 785-790: if data column and mask column have different lengths.
   - pandas equivalent: `ValueError: Lengths must match to compare` or internal shape assertion.
   - Propagation: wrapped via `#[error(transparent)]` and `#[from] ColumnError` in `FrameError::Column` (fp-frame line 22-23).

6. `ColumnError::Type(TypeError)`:
   - Variant: transparent wrapper over `fp_types::TypeError`.
   - Message string: inherited from `TypeError` display (fp-columnar line 476-477).
   - Trigger locations:
     - `binary_numeric` line 675 via `common_dtype(self.dtype, right.dtype)?`: when dtypes have no common promotion (e.g., `Utf8` + `Int64`).
     - `binary_numeric` scalar fallback line 702-703 via `val.to_f64()?`: when a non-missing value cannot be converted to f64 (e.g., `Scalar::Utf8` in a numeric arithmetic context).
     - `fillna` line 810 via `cast_scalar(fill_value, self.dtype)?`: when the fill value cannot be cast to the column's dtype.
   - pandas equivalent: `TypeError: unsupported operand type(s)` or `TypeError: Cannot convert`.
   - Propagation: `ColumnError::Type` -> `FrameError::Column` via `From` impl.

## Hidden Assumptions

1. Packet scope excludes full broadcast matrix and focuses on pairwise aligned arithmetic.
2. Label-domain heterogeneity is limited to current `IndexLabel` variants.
3. Hardened repairs are expected to remain rare and auditable.

## Undefined-Behavior Edges

1. Full pandas duplicate-label semantics (beyond allowlisted paths).
2. Advanced broadcast behavior across mixed dimensionality.
3. Complex dtype coercion corners not represented in scoped fixtures.
