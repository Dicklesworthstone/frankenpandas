# FP-P2C-003 Legacy Anchor Map

Packet: `FP-P2C-003`
Subsystem: Series arithmetic + mixed-label alignment

## Legacy Anchors

- `legacy_pandas_code/pandas/pandas/core/series.py` (aligned binary arithmetic semantics)
- `legacy_pandas_code/pandas/pandas/core/indexes/base.py` (index union behavior and duplicate-label handling)

## Extracted Behavioral Contract

1. Alignment is label-driven and deterministic for union materialization.
2. Non-overlapping labels become missing values in arithmetic outputs.
3. Hardened duplicate-label path is explicit and auditable; strict mode remains fail-closed.

## Rust Slice Implemented

- `crates/fp-frame/src/lib.rs`: `Series::add_with_policy`
- `crates/fp-index/src/lib.rs`: `align_union`
- `crates/fp-conformance/src/lib.rs`: packetized differential fixture execution

## Type Inventory

- `fp_frame::Series`
  - fields: `name: String`, `index: Index`, `column: Column`
- `fp_index::Index`
  - fields: `labels: Vec<IndexLabel>`, `duplicate_cache: OnceCell<bool>`
- `fp_index::AlignmentPlan`
  - fields: `union_index: Index`, `left_positions: Vec<Option<usize>>`, `right_positions: Vec<Option<usize>>`
- `fp_columnar::Column`
  - fields: `dtype: DType`, `values: Vec<Scalar>`, `validity: ValidityMask`

## Rule Ledger

1. Mixed-label arithmetic aligns on deterministic union before any numeric operation.
2. Non-overlap label slots are materialized as missing markers prior to arithmetic.
3. Duplicate-label behavior is mode-gated:
   - strict mode fail-closed,
   - hardened mode bounded repair with evidence logging.
4. Numeric operation relies on shared missing propagation contract from `Column::binary_numeric`.

## Error Ledger

- `FrameError::DuplicateIndexUnsupported` for strict duplicate-label unsupported paths.
- `FrameError::CompatibilityRejected` when runtime admission denies workload.
- `IndexError::InvalidAlignmentVectors` for invalid alignment plans.
- `ColumnError` propagation for reindex/coercion/arithmetic failures.

## Hidden Assumptions

1. Packet scope excludes full broadcast matrix and focuses on pairwise aligned arithmetic.
2. Label-domain heterogeneity is limited to current `IndexLabel` variants.
3. Hardened repairs are expected to remain rare and auditable.

## Undefined-Behavior Edges

1. Full pandas duplicate-label semantics (beyond allowlisted paths).
2. Advanced broadcast behavior across mixed dimensionality.
3. Complex dtype coercion corners not represented in scoped fixtures.
