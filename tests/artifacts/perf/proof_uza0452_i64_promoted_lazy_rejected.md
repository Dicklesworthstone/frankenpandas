# br-frankenpandas-uza04.52 rejection proof: promoted i64-as-f64 lazy lanes

## Target

- Scenario: `perf_profile outer_join 500000 20`
- Baseline commit: `732d62fa`
- Profile artifact: `perf_report_uza0452_base_outer_join_500000x20_children.txt`
- Fresh residual: dense outer materialization. Top children included
  `fp_join::build_single_key_dense_i64_outer_merge_output` and
  `fp_columnar::Column::from_f64_nullable_repeated_slices_shared`, with
  `__memset_avx2_unaligned_erms` under nullable validity materialization.

## Candidate

One lever only: keep promoted nullable outer-join lanes backed by source
`Vec<i64>` and cast to `Scalar::Float64(value as f64)` only if values are
materialized. This preserved the existing `ValidityMask` construction and
therefore did not attack the large `memset` residual directly.

The source diff was removed after measurement; no production code was retained.

## Correctness

- Baseline golden: `golden_base_uza0452_outer_join_20000.txt`
- Candidate golden: `golden_after_uza0452_i64_promoted_lazy_outer_join_20000.txt`
- Baseline SHA256:
  `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- Candidate SHA256:
  `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- `diff -u` between baseline and candidate golden outputs was empty.
- Isomorphism: row ordering, key ordering, missing-row placement, Float64
  promotion, and `Null(NullKind::NaN)` materialization were unchanged. No RNG,
  hashing, tie-breaking, or floating-point arithmetic order was introduced.

## Measurements

Baseline direct timer:

- `perf_profile_uza0452_outer_join_500000x20_base.txt`
- `59.178 ms/iter`

Candidate direct timer:

- `perf_profile_uza0452_outer_join_500000x20_i64_promoted_lazy_after.txt`
- `55.321 ms/iter`

Initial paired hyperfine:

- `hyperfine_pair_uza0452_i64_promoted_lazy_outer_join_500000x20.txt`
- Baseline: `1.150 s +/- 0.027`
- Candidate: `1.118 s +/- 0.017`
- Summary: candidate `1.03x +/- 0.03` faster

Longer confirmation hyperfine:

- `hyperfine_confirm_uza0452_i64_promoted_lazy_outer_join_500000x20.txt`
- Baseline: `1.136 s +/- 0.022`
- Candidate: `1.209 s +/- 0.138`
- Summary: baseline `1.06x +/- 0.12` faster than candidate

## Decision

Rejected. The golden proof passed, but paired confirmation did not show a
credible win and Score is below 2.0. The candidate source diff was removed.

Next primitive: lazy/null-run-aware validity storage for nullable repeated-slice
and repeat-value lanes, targeting the current profile's validity-mask `memset`
residual rather than another promoted-value casting micro-lever. Expected target
ratio for the next pass: at least 1.10x on `outer_join 500000 20` if validity
materialization can be deferred without changing the public `Column` contract.
