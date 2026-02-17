# FP-P2D-019 Legacy Anchor Map

Packet: `FP-P2D-019`
Subsystem: DataFrame constructor kwargs parity matrix (`index`/`columns` over frame payload)

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/frame.py` (`DataFrame.__init__` frame-input constructor path)
- `legacy_pandas_code/pandas/pandas/core/generic.py` (axis reindex semantics for label-targeting)
- `legacy_pandas_code/pandas/pandas/core/indexes/base.py` (index label projection/reordering behavior)

## Extracted Behavioral Contract

1. `DataFrame(frame)` preserves source frame shape/content.
2. `DataFrame(frame, columns=[...])` performs column projection and allows missing columns via null fill.
3. `DataFrame(frame, index=[...])` performs row re-targeting with null fill for unseen labels.
4. Combined `index` + `columns` kwargs apply simultaneously and deterministically.
5. Missing mandatory constructor payloads fail closed.

## Rust Slice Implemented

- `crates/fp-conformance/src/lib.rs`: `FixtureOperation::DataFrameConstructorKwargs` execution + comparator/differential wiring
- `crates/fp-conformance/oracle/pandas_oracle.py`: pandas-backed `dataframe_constructor_kwargs` handler + dispatch
- `crates/fp-frame/src/lib.rs`: underlying frame primitives used to materialize target axis surfaces

## Type Inventory

- `fp_conformance::FixtureOperation::DataFrameConstructorKwargs`
- `fp_conformance::PacketFixture` fields: `frame`, `index`, `column_order`, `expected_frame`, `expected_error_contains`
- `fp_types::Scalar` axis-fill variants: `Int64`, `Float64`, `Utf8`, `Null`

## Rule Ledger

1. Default axis surfaces derive from source frame when kwargs are absent.
2. Targeted missing row labels map to null-filled cells for all selected columns.
3. Targeted missing columns produce all-null columns over selected index.
4. Expected-error fixtures pass only when runtime failure text contains configured substring.

## Hidden Assumptions

1. Source frame columns are represented as map-backed unique labels in this slice.
2. Duplicate source index labels resolve by first-position semantics in current runtime helper path.

## Undefined-Behavior Edges

1. Duplicate target column labels in kwargs.
2. MultiIndex constructor kwargs and nested frame subclasses.
