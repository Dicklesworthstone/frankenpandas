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

1. Alignment strategy:
   - fast path: if indexes equal and duplicate-free, skip re-alignment,
   - otherwise align keys/values via `align_union`.
2. Dense-int optimization path:
   - if all non-dropped keys are `Int64` and key span <= 65_536, use dense bucket aggregation.
3. Group materialization:
   - first-seen key order defines output ordering.
4. Missing policy:
   - if `dropna=true`, missing keys are skipped,
   - missing values are additive no-op in sums.

## Error Ledger

- `GroupByError::Frame` propagation for series contract violations.
- `GroupByError::Index` propagation for invalid alignment plan.
- `GroupByError::Column` propagation for output/reindex column failures.

## Hidden Assumptions

1. Scoped aggregate is `sum` only.
2. Group key dimensionality is single-key series.
3. Dense-int path relies on bounded key-span heuristic and falls back to generic map path otherwise.

## Undefined-Behavior Edges

1. Multi-aggregate matrix (`mean`, `count`, `min`, `max`, etc.).
2. Multi-key DataFrame groupby planner semantics.
3. Full pandas `dropna`/`observed`/categorical interaction matrix.
