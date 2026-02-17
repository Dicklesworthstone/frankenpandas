# FP-P2D-017 Legacy Anchor Map

Packet: `FP-P2D-017`
Subsystem: constructor + dtype-coercion parity closure

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/series.py` (`Series.__init__`, dtype inference/coercion)
- `legacy_pandas_code/pandas/pandas/core/frame.py` (`DataFrame` constructor from Series inputs)
- `legacy_pandas_code/pandas/pandas/core/internals/construction.py` (constructor normalization and type coercion)

## Extracted Behavioral Contract

1. Series constructors preserve index labels and coerce values deterministically to common dtype.
2. Incompatible dtype mixes (e.g. utf8 with numeric) fail closed.
3. DataFrame-from-series constructors align by union index with deterministic label ordering.
4. Missing positions created by alignment become dtype-appropriate null values.
5. Duplicate column names resolve deterministically in insertion order semantics.

## Rust Slice Implemented

- `crates/fp-frame/src/lib.rs`: `Series::from_values`, `DataFrame::from_series`
- `crates/fp-conformance/src/lib.rs`: `series_constructor` + `dataframe_from_series` operations in fixture and differential harness
- `crates/fp-conformance/oracle/pandas_oracle.py`: pandas constructor oracle handlers

## Type Inventory

- `fp_conformance::FixtureOperation`: `SeriesConstructor`, `DataFrameFromSeries`, `ColumnDtypeCheck`
- `fp_frame::Series`, `fp_frame::DataFrame`
- `fp_types::Scalar` and `DType` coercion lattice (`Bool`, `Int64`, `Float64`, `Utf8`, `Null`)

## Rule Ledger

1. Constructor parity is checked on full index/value frame/series outputs, not only dtype metadata.
2. Dtype check fixtures enforce canonical coercion outcomes.
3. Error fixtures require explicit fail-closed messages.
4. Multi-series constructors use `left/right/groupby_keys` aggregate payload path.

## Hidden Assumptions

1. `dataframe_from_series` payload ordering is `left`, then `right`, then `groupby_keys` sequence.
2. Column-name collisions currently resolve by last insert into `BTreeMap`.

## Undefined-Behavior Edges

1. Full pandas constructor kwargs matrix (`dtype`, `copy`, nested dict/list forms) not yet modeled.
2. Extension dtypes and categorical constructors remain outside this packet.
