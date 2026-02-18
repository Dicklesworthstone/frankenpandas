# FP-P2D-028 Legacy Anchor Map

Packet: `FP-P2D-028`  
Subsystem: DataFrame column-wise concat (`axis=1`) parity

## Pandas Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/reshape/concat.py` (`concat`, `_Concatenator`, `sort=False` axis handling)
- `legacy_pandas_code/pandas/pandas/core/frame.py` (DataFrame concat materialization + label alignment expectations)

## FrankenPandas Anchors

- `crates/fp-frame/src/lib.rs`: `concat_dataframes_with_axis` + axis=1 alignment/null-fill path
- `crates/fp-conformance/src/lib.rs`: fixture parsing (`concat_axis`) + packet execution/differential pathways
- `crates/fp-conformance/oracle/pandas_oracle.py`: oracle parity bridge for `dataframe_concat` axis dispatch
- `crates/fp-conformance/fixtures/packets/fp_p2d_028_*`: axis=1 fixture matrix + fail-closed diagnostics

## Behavioral Commitments

1. Axis=1 concat aligns on the union of index labels with deterministic ordering.
2. Missing row positions materialize nulls rather than silently dropping labels.
3. Duplicate output columns and duplicate index inputs are explicit compatibility-gate errors.

## Open Gaps

1. MultiIndex axis=1 concat semantics.
2. Duplicate-column-preserving mode (pandas can represent duplicate column labels).
3. Join-mode switches (`inner`/`outer`) for axis=1 concat beyond current outer-only scope.

