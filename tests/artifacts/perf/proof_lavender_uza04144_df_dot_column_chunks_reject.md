# br-frankenpandas-uza04.144 df_dot per-column band chunks rejection

Agent: LavenderStone
Target: `df_dot 100000x5`
Decision: reject; no runtime source retained

## Profile-backed target

Fresh current-main routing after `a33fc2e0` ranked `df_dot` as the largest
measured target:

- `df_dot 100000x1`: `313.602 ms/iter`
- `df_corr 100000x1`: `107.116 ms/iter`
- `str_outer_join 100000x5`: `40.077 ms/iter`

Routing artifact:

- `tests/artifacts/perf/lavender_next_a33fc2e0_profile_matrix.txt`

`perf record` was attempted for `df_dot 100000x1`, but this host blocks perf
events with `perf_event_paranoid=4`:

- `tests/artifacts/perf/lavender_uza04144_base_perf_record_df_dot_100000x1.txt`

## Candidate lever

One deeper output-layout lever was tested:

- Replace each worker's zero-filled shared row-band tape
  (`vec![0.0; bw * n]`) with per-output-column `Vec<f64>` chunks.
- The compute loop still used the same 4x4 SIMD micro-kernel and the same
  `l = 0..k` accumulation order for every output cell.
- Each column chunk received values in ascending row order via `Vec::push`, so
  the candidate avoided zero-filling the 205MB band tape before overwriting it.

This was the next output-layout primitive after earlier `df_dot` routes kept
row-banded compute, parallel output assembly, worker-private all-valid chunks,
and SIMD lanes. It deliberately did not repeat worker-cap, BI/BJ width, or
micro-kernel unroll tweaks.

## Isomorphism proof

- Row order: unchanged. Values were pushed for each column in increasing row
  tile order and increasing row-within-tile order.
- Column order: unchanged. `build_col(j)` still assembled the same output
  column `j` from sorted worker bands.
- Floating point: unchanged. Each cell still performed `acc += a * b` for
  `l = 0..k` in ascending order; no horizontal reduction, FMA, or reassociation
  was introduced.
- NaN/null semantics: unchanged. The same per-column `column_has_nan` witness
  selected either all-valid chunk adoption or the existing `Column::from_f64_values`
  fallback.
- RNG and tie-breaking: not used by this path.

Golden outputs matched baseline exactly:

- `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
- `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

Artifacts:

- `tests/artifacts/perf/lavender_uza04144_base_golden_df_dot_2000.sha256`
- `tests/artifacts/perf/lavender_uza04144_candidate_golden_df_dot_2000.sha256`
- `tests/artifacts/perf/lavender_uza04144_base_golden_df_dot_5000.sha256`
- `tests/artifacts/perf/lavender_uza04144_candidate_golden_df_dot_5000.sha256`

## Benchmark gate

Baseline-only hyperfine:

- `df_dot 100000x5`: `630.3 ms +/- 12.7`

Forward paired hyperfine:

- Baseline: `629.4 ms +/- 15.4`
- Candidate: `652.3 ms +/- 13.0`
- Baseline ran `1.04x +/- 0.03` faster.

Reversed paired hyperfine:

- Candidate: `653.0 ms +/- 13.4`
- Baseline: `623.2 ms +/- 18.2`
- Baseline ran `1.05x +/- 0.04` faster.

Artifacts:

- `tests/artifacts/perf/lavender_uza04144_base_hyperfine_df_dot_100000x5.txt`
- `tests/artifacts/perf/lavender_uza04144_pair_hyperfine_df_dot_100000x5.txt`
- `tests/artifacts/perf/lavender_uza04144_pair_reversed_hyperfine_df_dot_100000x5.txt`

## Validation

- `rch exec -- cargo check -p fp-frame --lib` passed for the candidate after
  removing an unused loop index warning.
- Candidate release-perf build passed remotely on `vmi1227854`.

Artifacts:

- `tests/artifacts/perf/lavender_uza04144_candidate_check2_fp_frame.txt`
- `tests/artifacts/perf/lavender_uza04144_candidate_build_perf_profile.txt`

## Decision

Rejected. The candidate preserved behavior but consistently regressed wall time
and increased system time, likely because per-column `Vec` fanout and many
small chunk allocations outweighed the removed zero-fill.

Score: negative impact, below `Score >= 2.0`.

The runtime source hunk was removed before commit. Do not repeat the per-column
worker-private chunk fanout route. The next `df_dot` route should use a
different primitive, such as a fused final-column chunk kernel that avoids both
the shared row-band tape and per-column `Vec` fanout, or a phase-timer-guided
cache-blocked compute change that does not add allocation pressure.
