# br-frankenpandas-vc4de Vec-backed dot chunk rejection

## Target

- Bead: `br-frankenpandas-vc4de`
- Scenario: `df_dot 100000x5`
- Profile route: post-`br-frankenpandas-uza04.132` bead description routed to
  `df_dot`, with sampled `perf record` blocked in this environment by
  `perf_event_paranoid=4`.
- Candidate lever: keep each computed dot output band as `Arc<Vec<f64>>` inside
  chunked Float64 output columns, avoiding conversion to `Arc<[f64]>`.

## One-lever boundary

The intended lever touched only the output handoff after the existing GEMM
kernel had already computed each band. It did not change:

- A/B operand extraction order.
- 4x4 tile shape.
- Worker count or row-band partitioning.
- Per-cell `l = 0..k` floating-point multiplication/addition order.
- Output row or column order.

The source hunk was removed after the benchmark gate failed.

## Golden output

Baseline:

```text
ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535  df_dot 2000
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d  df_dot 5000
```

Candidate:

```text
ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535  df_dot 2000
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d  df_dot 5000
```

## Benchmark

Baseline artifact: `tests/artifacts/perf/lavender_vc4de_base_baseline.txt`

- Internal: `162.461 ms/iter`
- Standalone hyperfine: `671.0 ms +/- 27.4 ms`

Candidate artifact: `tests/artifacts/perf/lavender_vc4de_after_candidate.txt`

- Internal: `125.297 ms/iter`
- Standalone hyperfine: `668.0 ms +/- 32.3 ms`

Paired artifact: `tests/artifacts/perf/lavender_vc4de_pair_df_dot_100000x5.txt`

- Baseline: `717.6 ms +/- 61.6 ms`
- Candidate: `680.3 ms +/- 75.3 ms`
- Paired result: `1.05x +/- 0.15`

The paired hyperfine result is below the keep threshold and too noisy to
attribute confidently. The candidate run was also not an isolated proof because
a concurrent uncommitted `df_dot` NaN-bound witness was present in the shared
tree; the Vec-backed chunk hunk therefore has no acceptable keep evidence.

## Decision

- Impact: 1
- Confidence: 1
- Effort: 2
- Score: `1 * 1 / 2 = 0.5`

Decision: rejected. Source hunk removed. Do not retry the `Arc<Vec<f64>>`
chunk handoff family; route to a structurally different `df_dot` primitive in a
clean worktree.

## Validation

- `rch exec -- cargo check -p fp-frame --all-targets`: passed.
- `perf record`: blocked by `perf_event_paranoid=4`; artifact
  `tests/artifacts/perf/lavender_vc4de_base_perf_record_df_dot_100000x3.txt`.
