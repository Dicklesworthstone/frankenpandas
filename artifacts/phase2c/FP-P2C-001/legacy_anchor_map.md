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

1. Duplicate-label gating:
   - if `self.index.has_duplicates() || other.index.has_duplicates()` and mode is strict -> reject.
   - otherwise log decision and continue via hardened bounded path.
2. Alignment union:
   - `align_union` preserves left order and appends only right-unseen labels.
3. Alignment materialization:
   - `reindex_by_positions` injects dtype-specific missing values for `None` slots.
4. Arithmetic missing propagation:
   - missing on either side yields missing output;
   - NaN contamination yields NaN-missing marker.
5. Column naming:
   - same-name add keeps name; differing names emit `left+right`.

## Error Ledger

- `FrameError::LengthMismatch` when index/column lengths differ.
- `FrameError::DuplicateIndexUnsupported` for strict duplicate-label unsupported surface.
- `FrameError::CompatibilityRejected` if runtime admission rejects alignment workload.
- `IndexError::InvalidAlignmentVectors` when plan vectors are inconsistent.
- `ColumnError` propagation for reindex/arithmetic coercion failures.

## Hidden Assumptions

1. Packet scope is binary Series arithmetic with explicit labels, not full DataFrame arithmetic matrix.
2. Duplicate-label semantics outside the hardened allowlist are intentionally unsupported in strict mode.
3. Missing propagation semantics rely on `Scalar::missing_for_dtype` and current dtype coercion rules.

## Undefined-Behavior Edges

1. Full pandas duplicate-label behavioral matrix beyond current gate.
2. Full broadcast semantics (`Series`-`DataFrame`, scalar broadcasting edge paths).
3. Full indexing surface (`loc`/`iloc`) interaction with arithmetic alignment.
