# br-frankenpandas-yq96z shared repeat-run descriptor rejection

## Target

Current `main` after `86d01221` and the Beads claim commit was profiled by
timing because `perf stat` is blocked by `perf_event_paranoid=4`. The live
fanout target has shifted from outer join to one-sided repeated-key joins:

```text
left_join  500000x20: 707.6 ms +/- 24.7
outer_join 500000x20:  75.6 ms +/-  7.9
right_join 500000x20: 640.1 ms +/- 26.4
```

System profiling attempt:

```text
perf_stat_status=255
perf_event_paranoid setting is 4
```

## Rejected Lever

In `build_single_key_dense_i64_left_merge_output` and
`build_single_key_dense_i64_right_merge_output`, the candidate shared the
kept-side run-length descriptor with `Column::from_i64_repeat_values_run_lengths`
instead of rebuilding `(value, run_len)` tuples for every kept-side output
column.

No runtime source was retained after the score gate failed.

## Isomorphism Proof

- Probe order was unchanged: the candidate consumed the existing left/right
  `plan` in the same order.
- Run lengths were unchanged: the shared descriptor was exactly
  `plan.iter().map(run_len)`.
- Values were unchanged: per-column `run_values` were gathered from the same
  source positions that the original tuple-runs used.
- Null placement was unchanged: nullable opposite-side lanes and their
  `usize::MAX` segment sentinel were untouched.
- Output ordering, tie handling, dtype/null semantics, floating-point behavior,
  and RNG were unchanged.

Golden SHA256 comparison against baseline matched byte-identically for
`left_join`, `outer_join`, `right_join`, and `inner_join` at `n=2000` and
`n=20000`; see:

- `tests/artifacts/perf/yq96z_base_golden_hash_rows.txt`
- `tests/artifacts/perf/yq96z_candidate_golden_hash_rows.txt`

## Benchmark Gate

Paired hyperfine, same local runner:

```text
left_join  500000x20: 712.9 ms +/- 31.0 -> 718.8 ms +/- 17.8 (0.99x, regression)
right_join 500000x20: 653.3 ms +/- 23.0 -> 645.8 ms +/- 20.5 (1.01x, neutral)
outer_join 500000x20:  81.7 ms +/-  8.7 ->  76.1 ms +/-  4.7 (1.07x, non-target/noisy)
```

Score: Impact 0.5 x Confidence 4 / Effort 1 = 2.0 nominal, but the target
`left_join` regressed and `right_join` was neutral. Rejected by the keep rule.

## Verdict

REJECTED. Source hunk removed.

Do not retry the shared-run descriptor or tuple-allocation micro-family for
this bead. The next swing is a deeper primitive: add nullable Int64
dense-cycle source-position columns for one-sided left/right joins so the hot
path can skip the 500k-row segment descriptor and bucket-order tape entirely.
Target ratio: at least 1.25x on `left_join 500000x20` and `right_join
500000x20`, with the same golden SHA set.
