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

1. Duplicate detection:
   - `has_duplicates` caches result; semantics are first-hit duplicate detection.
2. Position semantics:
   - `position_map_first` and `position_map_first_ref` preserve first occurrence for duplicate labels.
3. Union semantics:
   - output union starts with all left labels, then appends only right labels missing on left.
4. Plan validation:
   - alignment vectors must match union index length or plan is invalid.

## Error Ledger

- `IndexError::InvalidAlignmentVectors` when `left_positions`/`right_positions` lengths diverge from union index.
- Upstream fail-closed behavior for unsupported duplicate/indexing surfaces in strict mode.

## Hidden Assumptions

1. Packet scope assumes flat single-level index only.
2. Null/NaN marker handling is delegated to higher layers; index packet treats labels as opaque atoms.
3. First-occurrence selection is acceptable for scoped duplicate handling while full pandas matrix is deferred.

## Undefined-Behavior Edges

1. Full `get_indexer` family behavior matrix.
2. `MultiIndex` tuple semantics and hierarchical lookup.
3. Partial-string/timezone-sensitive index slicing semantics.
