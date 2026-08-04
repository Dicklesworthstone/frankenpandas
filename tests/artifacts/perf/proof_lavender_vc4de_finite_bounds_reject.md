# br-frankenpandas-vc4de finite-bound dot witness rejection

## Target

- Bead: `br-frankenpandas-vc4de`
- Scenario: `df_dot 100000x5`
- Post-`.132` routing: `tests/artifacts/perf/lavender_post_uza04132_routing_matrix.txt`
  measured `df_dot` at `145.900 ms/iter`, above `csv_parse_dates_dt_year`
  at `70.070 ms/iter` and `str_outer_join` at `41.709 ms/iter`.

## Candidate

Attempted one structurally different primitive:

- Cache finite/max-absolute bounds on immutable all-valid Float64 columns.
- In `DataFrame::dot`, skip per-output NaN witness updates only when the cached
  input bounds prove `k * max_abs(A) * max_abs(B) <= f64::MAX`.
- Fallback preserved the existing per-output NaN witness path when any input was
  non-finite, nullable, non-Float64, or not covered by the cached backing.

This was intended to avoid the rejected full-input-scan family by carrying the
witness on typed column metadata and amortizing it across repeated `dot` calls.

## Behavior

Baseline goldens:

```text
ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535  tests/artifacts/perf/lavender_vc4de_base_golden_df_dot_2000.txt
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d  tests/artifacts/perf/lavender_vc4de_base_golden_df_dot_5000.txt
```

Candidate goldens:

```text
ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535  tests/artifacts/perf/lavender_vc4de_after_golden_df_dot_2000.txt
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d  tests/artifacts/perf/lavender_vc4de_after_golden_df_dot_5000.txt
```

`cmp` returned `GOLDEN_CMP_OK` for both sizes.

Isomorphism:

- Output row/column order unchanged.
- Each output cell kept the same `l=0..k` f64 fold order.
- No tie-breaking or RNG involved.
- Fallback retained existing NaN/null semantics for any unproven input.

## Timing

Baseline:

- Internal: `167.933 ms/iter`
- Hyperfine: `666.1 ms +/- 10.7 ms`

Candidate:

- Internal: `160.986 ms/iter`

Paired forward:

- Base: `678.8 ms +/- 22.4 ms`
- After: `651.0 ms +/- 39.1 ms`
- Speedup: `1.04x +/- 0.07`

Paired reversed:

- After: `637.2 ms +/- 16.0 ms`
- Base: `693.2 ms +/- 25.6 ms`
- Speedup: `1.09x +/- 0.05`

## Decision

Rejected. The candidate preserved behavior but did not clear Score>=2.0. The
runtime source hunk was removed after measurement.

Do not retry this cached finite/max-absolute metadata witness for `df_dot`.
The next route should target a different primitive, such as a deeper blocked
GEMM memory-layout change or a different high-scoring post-reprofile hotspot.
