# FP-P2D-018 Legacy Anchor Map

Packet: `FP-P2D-018`
Subsystem: DataFrame constructor parity matrix (dict/records)

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/frame.py` (`DataFrame.__init__`, constructor normalization paths)
- `legacy_pandas_code/pandas/pandas/core/internals/construction.py` (dict/records materialization, column/index reconciliation)
- `legacy_pandas_code/pandas/pandas/core/indexes/base.py` (index cardinality/error contracts)

## Extracted Behavioral Contract

1. Dict-of-columns constructor infers default RangeIndex when `index` is absent.
2. Explicit index override must satisfy constructor cardinality constraints.
3. Records constructor fills missing keys with null-like values.
4. Records constructor supports explicit column selection and expansion with all-null synthetic columns.
5. Constructor payload contract violations fail closed with explicit mismatch diagnostics.

## Rust Slice Implemented

- `crates/fp-conformance/src/lib.rs`: fixture schema + execution wiring for `dataframe_from_dict` and `dataframe_from_records`
- `crates/fp-conformance/oracle/pandas_oracle.py`: live oracle handlers for dict/records constructor operations
- `crates/fp-frame/src/lib.rs`: `DataFrame::from_dict` and `DataFrame::from_dict_with_index` constructor primitives

## Type Inventory

- `fp_conformance::FixtureOperation::{DataFrameFromDict, DataFrameFromRecords}`
- `fp_conformance::PacketFixture` fields: `dict_columns`, `records`, `column_order`, `index`, `expected_frame`, `expected_error_contains`
- `fp_types::Scalar` constructor variants: `Bool`, `Int64`, `Float64`, `Utf8`, `Null`

## Rule Ledger

1. Constructor success requires index length and all column lengths to agree.
2. `column_order` subset selection is validated against declared constructor payloads.
3. Sparse record keys are represented as explicit null values at missing row/column intersections.
4. Expected-error fixtures pass only when runtime failure text contains the declared substring.

## Hidden Assumptions

1. Packet scope uses scalar-only records (no nested arrays/objects).
2. Column ordering observability is normalized through map-backed frame representation.

## Undefined-Behavior Edges

1. MultiIndex constructor payloads.
2. pandas constructor kwargs not represented in packet schema (`dtype`, `copy`, extension-array constructors).
