# br-frankenpandas-uza04.126 - df_dot 4x4 full-tile microkernel unroll rejection

## Target

After the `.124` worker-private output chunk keep and `.125` BI=8 rejection,
a fresh current-main routing matrix still showed `df_dot 100000x6` as the
dominant checked lane:

- `df_dot 100000x6`: 123.108 ms/iter
- next-largest checked lane, `str_sort_chain 100000x10`: 14.157 ms/iter

This pass tested a register-blocked microkernel variant that kept `BI=4,BJ=4`
but manually unrolled the dominant full-tile accumulator update.

## Isomorphism Proof

The candidate only replaced the full-tile iterator/enumerate loops with
explicit accumulator statements. For every output cell `C[i][j]`, the inner
loop still visited `l = 0..k` in ascending order and performed the same
separate f64 multiply then add. Edge tiles, row-band ownership, output
assembly, column order, row order, null/NaN policy, tie behavior, and RNG state
were unchanged.

## Golden Output

Baseline and candidate matched exactly:

```text
df_dot 2000:
ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535

df_dot 5000:
04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d
```

Artifacts:

- `tests/artifacts/perf/lavender_df_dot_unroll4x4_base_golden_2000.txt`
- `tests/artifacts/perf/lavender_df_dot_unroll4x4_base_golden_5000.txt`
- `tests/artifacts/perf/lavender_df_dot_unroll4x4_after_golden_2000.txt`
- `tests/artifacts/perf/lavender_df_dot_unroll4x4_after_golden_5000.txt`
- `tests/artifacts/perf/lavender_df_dot_unroll4x4_after_golden_check.txt`

## Benchmark

Baseline build used the current-main release-perf `perf_profile` binary from a
crate-scoped RCH/fail-open build. Candidate build completed remotely on
`vmi1227854`; artifacts were retrieved and paired timing used local baseline
and candidate binaries from the same host.

Standalone baseline:

```text
df_dot 100000x6: 770.4 ms +/- 25.6 ms
```

Paired forward:

```text
baseline: 786.0 ms +/- 10.2 ms
unroll:   768.0 ms +/- 17.7 ms
```

Paired reversed:

```text
unroll:   827.2 ms +/- 17.6 ms
baseline: 779.3 ms +/- 16.5 ms
```

Forward order showed only `1.02x`; reversed order flipped to baseline `1.06x`
faster. This is below the Score>=2.0 keep threshold.

## Verdict

Rejected. The source hunk was removed before closeout.

The likely cause is that LLVM already optimizes the constant 4x4 inner loops
well enough; manual unrolling increased instruction footprint/register
pressure without a stable throughput win. Do not retry simple 4x4 statement
unrolling. The next `df_dot` route should be structurally different, such as
right-side packing tuned for wider column panels, finite-input NaN-witness
elision, or output-consumer chunk fusion after re-profiling.
