# br-frankenpandas-uza04.148 rejection proof

Agent: LavenderStone
Date: 2026-06-17
Target: `df_dot 100000 3`
Candidate lever: reusable `Float64DotInputPlan` for lazy all-valid finite dot columns.

## Profile-backed target

Post-`br-frankenpandas-uza04.147` local release-perf routing matrix:

- Artifact: `tests/artifacts/perf/lavender_post_uza04147_local_profile_matrix.txt`
- `df_dot 100000 3`: `76.188 ms/iter`
- This was the largest non-rejected residual after the `csv_parse_dates_dt_year`
  keep and after skipping the already-rejected `value_counts_nan50` lane.

## Baseline

- Baseline source: `b4558a9a` (source-identical to `ba0b4573` for df-dot)
- Baseline binary: `/data/projects/.scratch/lavender_uza04148_base_perf_profile`
- Baseline-only hyperfine artifact:
  `tests/artifacts/perf/lavender_uza04148_base_hyperfine_df_dot_100000x3.json`
- Baseline-only mean: `224.3 ms +/- 10.2 ms`

## Behavior proof

Golden command:

```text
perf_profile golden df_dot 5000
```

Baseline SHA256:

```text
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d
```

Candidate SHA256:

```text
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d
```

`diff -u` verified the baseline and dedicated-candidate golden files were
byte-identical.

Isomorphism notes:

- Row order, output column order, and output shape are unchanged.
- The candidate changed only construction/sharing of lazy left-hand dot inputs.
- Floating-point accumulation order remained the existing materialization
  left-fold over source columns.
- Null, NaN, infinite, dtype, and alignment fallback gates were not changed.
- No RNG or nondeterministic tie-breaking is involved.

## Paired benchmark

Forward order:

- Baseline: `236.5 ms +/- 6.8 ms`
- Candidate: `236.4 ms +/- 10.9 ms`
- Ratio: `1.00x +/- 0.05`

Reversed order:

- Candidate: `233.9 ms +/- 7.4 ms`
- Baseline: `237.6 ms +/- 10.5 ms`
- Ratio: `1.02x +/- 0.06`

Final dedicated artifacts:

- Build: `tests/artifacts/perf/lavender_uza04148_candidate_build_perf_profile_dedicated_target.txt`
- Candidate golden:
  `tests/artifacts/perf/lavender_uza04148_candidate_dedicated_golden_df_dot_5000.txt`
- Golden SHA:
  `tests/artifacts/perf/lavender_uza04148_candidate_dedicated_golden_df_dot_5000.sha256`
- Golden diff:
  `tests/artifacts/perf/lavender_uza04148_candidate_dedicated_golden_df_dot_5000.diff`
- Forward hyperfine:
  `tests/artifacts/perf/lavender_uza04148_pair_dedicated_hyperfine_df_dot_100000x3.json`
- Reversed hyperfine:
  `tests/artifacts/perf/lavender_uza04148_pair_dedicated_reversed_hyperfine_df_dot_100000x3.json`

Dedicated-target confirmation:

- Forward: baseline `236.5 ms +/- 6.8 ms`, candidate
  `236.4 ms +/- 10.9 ms`, ratio `1.00x +/- 0.05`
- Reversed: candidate `233.9 ms +/- 7.4 ms`, baseline
  `237.6 ms +/- 10.5 ms`, ratio `1.02x +/- 0.06`

## Decision

Rejected. The observed improvement is within noise and does not clear the
Score>=2.0 keep threshold.

Score: Impact 0.5 * Confidence 2.0 / Effort 2.0 = `0.5`.

Source changes were removed after measurement. The next route should be a
deeper df-dot primitive instead of another shared-plan micro-lever, such as a
phase-timer-backed materialization/drop avoidance path, fused terminal
consumer, or a fundamentally different blocked output layout.
