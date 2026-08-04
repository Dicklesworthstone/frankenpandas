# br-frankenpandas-gvxsn - df_dot BJ=8 compute-kernel rejection

LavenderStone, 2026-06-14.

## Profile-backed target

Current `main` after `br-frankenpandas-uza04.124` was re-profiled with
`tests/artifacts/perf/lavender_next_profile_matrix_current_v3.txt`.
`df_dot 100000 5` was the dominant measured lane at `123.154 ms/iter`
inside the harness, above `left_join` (`15.063 ms/iter`), CSV lanes
(`~12 ms/iter`), and `str_duplicated` (`8.394 ms/iter`).

## Baseline

- Commit: `0f2e89991a44633e19718a7da3952e23e411a1c5`.
- Baseline binary: `.rch-target-lavenderstone-next/release-perf/examples/perf_profile`.
- Build note: `rch exec` failed open locally because no workers were admissible.
- Baseline hyperfine, `df_dot 100000 5`: `679.2 ms +/- 10.6 ms`.
- Golden SHA:
  - `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
  - `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

## Candidate

One compute-only lever: widen the `DataFrame::dot` packed B column block from
`BJ=4` to `BJ=8`, preserving each output cell's `l=0..k` ascending f64
accumulation order and the same output storage path.

## Isomorphism

- Golden SHAs matched exactly for `df_dot 2000` and `df_dot 5000`.
- `cmp -s` passed for both baseline-vs-after golden outputs.
- Row/column ordering was unchanged.
- Floating-point order per cell was unchanged; each accumulator still folds
  `l=0..k` in order.
- Tie-breaking and RNG behavior are not involved.

## Bench Gate

- Paired forward:
  - baseline: `667.6 ms +/- 12.8 ms`
  - candidate: `979.4 ms +/- 17.7 ms`
  - baseline was `1.47x +/- 0.04` faster.
- Paired reversed:
  - candidate: `996.6 ms +/- 18.6 ms`
  - baseline: `663.0 ms +/- 14.5 ms`
  - baseline was `1.50x +/- 0.04` faster.

Verdict: reject. The source hunk was removed. The likely reason is register
pressure/spills from 32 live accumulators outweighing any column-block overhead
reduction. Next route should avoid wider accumulator tiles and instead attack a
different primitive, such as k-panel packing/reuse or a row-panel scheduler that
reduces per-band thread/system overhead without changing per-cell accumulation.
