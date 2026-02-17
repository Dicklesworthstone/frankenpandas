# FP-P2D-020 Legacy Anchor Map

Packet: `FP-P2D-020`
Subsystem: DataFrame constructor scalar + dict-of-series parity matrix

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/frame.py` (`DataFrame.__init__` scalar and dict-of-series constructor branches)
- `legacy_pandas_code/pandas/pandas/core/indexes/base.py` (index alignment/reindex semantics)
- `legacy_pandas_code/pandas/pandas/core/internals/construction.py` (constructor data normalization and column projection behavior)

## Extracted Behavioral Contract

1. Scalar constructor requires explicit shape drivers (`index` + `columns`) and broadcasts scalar to every cell.
2. Dict-of-series constructor aligns values by union index when no explicit index is provided.
3. Explicit `index` re-targets rows with null fill for unseen labels.
4. Explicit `columns` projects and expands columns; missing columns become all-null.
5. Duplicate series names in dict input resolve deterministically to the last payload.
6. Missing required payloads fail closed.

## Rust Slice Implemented

- `crates/fp-conformance/src/lib.rs`: `FixtureOperation::{DataFrameConstructorScalar, DataFrameConstructorDictOfSeries}` execution + differential wiring
- `crates/fp-conformance/oracle/pandas_oracle.py`: pandas-backed scalar and dict-of-series constructor handlers
- `crates/fp-frame/src/lib.rs`: frame materialization and index/column projection primitives used by constructor replay

## Type Inventory

- `fp_conformance::FixtureOperation::{DataFrameConstructorScalar, DataFrameConstructorDictOfSeries}`
- `fp_conformance::PacketFixture` fields: `fill_value`, `index`, `column_order`, `left`, `right`, `groupby_keys`, `expected_frame`, `expected_error_contains`
- `fp_types::Scalar` constructor payload types: `Int64`, `Utf8`, `Null`

## Rule Ledger

1. Scalar broadcast replicates `fill_value` across `len(index) * len(column_order)` cells.
2. Dict-of-series constructor respects index labels, not positional offsets.
3. Missing labels/columns always map to deterministic null values.
4. Expected-error fixtures pass only when runtime failure text contains configured substring.

## Hidden Assumptions

1. Dict-of-series payload names are unique unless duplicate-name overwrite behavior is under test.
2. Current Rust projection helper resolves duplicate source index labels via first-position semantics.

## Undefined-Behavior Edges

1. Duplicate target column labels in constructor `column_order`.
2. MultiIndex constructor inputs and extension-array constructor payloads.
