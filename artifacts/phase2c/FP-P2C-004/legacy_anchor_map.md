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

1. Right-side lookup map is built by label with positional vectors.
2. For each left label:
   - if right matches exist, emit cross-product rows (`left_pos x each right_pos`),
   - if no match and join type is left, emit row with missing right slot.
3. Output ordering is left-driven and stable by scan order.
4. `reindex_by_positions` materializes missing right values for unmatched left rows.

## Error Ledger

- `JoinError::Frame` propagation from series/index contract failures.
- `JoinError::Column` propagation from output column materialization failures.
- Unknown join modes are fail-closed at API boundary (only typed `Inner`/`Left` accepted).

## Hidden Assumptions

1. Packet scope is indexed series join core, not full DataFrame merge planner.
2. Join mode surface is intentionally constrained to `Inner`/`Left`.
3. Cardinality expansion from duplicates is accepted as scoped behavior, not abuse mitigation.

## Undefined-Behavior Edges

1. Full multi-column DataFrame merge behavior matrix.
2. Non-scoped join mode semantics and sort option matrix.
3. Advanced null-key equivalence rules beyond scoped fixtures.
