# FP-P2D-021 Legacy Anchor Map

Packet: `FP-P2D-021`
Subsystem: DataFrame constructor list-like / 2D-array parity matrix

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/frame.py` (`DataFrame.__init__` list-like array constructor paths)
- `legacy_pandas_code/pandas/pandas/core/internals/construction.py` (2D array coercion, ragged-row handling, and column assignment behavior)
- `legacy_pandas_code/pandas/pandas/core/indexes/base.py` (default/explicit index behavior)

## Extracted Behavioral Contract

1. `DataFrame(matrix_rows)` assigns default range index and positional default columns.
2. Explicit `columns` controls shape projection and allows expansion when rows are short.
3. Explicit `index` is valid only when index cardinality equals row count.
4. Ragged rows are null-padded when explicit/default column surfaces permit.
5. Malformed payloads fail closed.

## Rust Slice Implemented

- `crates/fp-conformance/src/lib.rs`: `FixtureOperation::DataFrameConstructorListLike` execution + differential wiring
- `crates/fp-conformance/oracle/pandas_oracle.py`: pandas-backed list-like constructor handler + dispatch aliases
- `crates/fp-frame/src/lib.rs`: `DataFrame::from_dict_with_index` constructor primitives used for materialization

## Type Inventory

- `fp_conformance::FixtureOperation::DataFrameConstructorListLike`
- `fp_conformance::PacketFixture` fields: `matrix_rows`, `index`, `column_order`, `expected_frame`, `expected_error_contains`
- `fp_types::Scalar`: `Int64`, `Float64`, `Utf8`, `Null`

## Rule Ledger

1. Default positional columns are emitted as string labels (`"0"`, `"1"`, ...).
2. Short rows fill missing cells with deterministic null values.
3. Explicit column sets wider than row payloads are null-padded.
4. Index cardinality mismatch is a hard constructor error.

## Hidden Assumptions

1. Input `matrix_rows` values are scalar-only and pre-normalized to fixture scalar types.
2. Current Rust constructor path does not preserve any non-string positional column label types.

## Undefined-Behavior Edges

1. Duplicate labels inside explicit `column_order`.
2. Extension-array and nested object list-like constructor payloads.
