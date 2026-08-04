# br-frankenpandas-uza04.50 Pass 2 Primitive Selection

Timestamp: 2026-06-11T19:32:00Z
Target: homogeneous Float64 `Column::from_values` constructor residual exposed
by `filter_bool 100000 1000`.

## Hotspot Evidence

Pass 1 exact-gate profile shows:

- `filter_bool 100000 1000`: `43.1 ms +/- 2.3 ms`.
- `<fp_columnar::Column>::new`: 10.41% self.
- `<fp_columnar::ColumnData>::from_scalars`: 5.42% self.
- `fp_types::infer_dtype`: 2.86% self.
- Call stacks place these under `perf_profile::build_numeric_frame` via
  `Column::from_values(Vec<Scalar::Float64>)`.

The steady-state `filter_bool 100000 20000` profile does not support an affine
output `Column::new` target, so this pass deliberately targets the real public
constructor path and leaves the period-2 verifier residual for follow-up.

## Selected Primitive

Typed homogeneous Float64 constructor proof.

If `Column::from_values` receives a non-empty vector whose first value is
`Scalar::Float64` and every value is `Scalar::Float64`, it can compile the
scalar-vector proof into the existing `Column::from_f64_values(Vec<f64>)`
artifact directly. This skips:

- `infer_dtype` over the scalar vector,
- `Column::new` same-dtype validation and missing-normalization scans,
- `ValidityMask::from_values` over `Scalar`,
- `ColumnData::from_scalars`.

Mixed, null-bearing, empty, integer, bool, string, datetime, timedelta, period,
interval, and sparse inputs fall back to the current path.

## Graveyard / Artifact Mapping

- `alien_cs_graveyard.md` §8.2 Vectorized Execution: column-at-a-time typed
  buffers should carry proof of dtype homogeneity instead of reinterpreting rows
  through scalar enums.
- `alien_cs_graveyard.md` §8.9 Provenance Semirings / Lineage: keep compact
  lineage/proof metadata instead of inflating row annotations; here the proof is
  the homogeneous variant scan.
- `extreme-software-optimization` memory/allocation catalog: avoid duplicate
  scans and scalar materialization when a typed buffer already represents the
  same values.

## Score

Impact 3 x Confidence 4 / Effort 2 = 6.0.

This clears the Score >= 2.0 gate because the exact benchmark's constructor
rows are directly targeted, the implementation is local to `fp-columnar`, and
the fallback preserves every non-Float64 or mixed path.

## Proof Obligations

- Ordering preserved: output row `i` materializes the same f64 payload from
  input position `i`.
- Floating-point bits preserved: collect the raw `f64` payloads without
  arithmetic or canonicalization; `-0.0`, infinities, and NaN payloads are not
  recomputed.
- NaN/null semantics preserved: `Scalar::Float64(NaN)` remains a Float64
  payload and is marked invalid by `Column::from_f64_values`, matching
  `Column::new -> ValidityMask::from_values`.
- Mixed and null-bearing semantics preserved: any non-`Scalar::Float64` value
  falls back to `infer_dtype + Column::new`, so generic `Null` and object-bucket
  behavior stays unchanged.
- Empty input preserved: empty vector falls back to current `DType::Null`
  inference.
- Tie-breaking and RNG: N/A.

## Fallback / Rejection Trigger

Reject and remove the source hunk if:

- `filter_bool` golden SHA changes for 1000 or 100000 rows,
- focused `fp-columnar` tests/check/clippy fail because of the hunk,
- paired hyperfine for `filter_bool 100000 1000` does not show a real win,
- or any constructor test shows changed dtype, validity, scalar materialization,
  `-0.0`, infinity, NaN, empty, mixed, or null behavior.
