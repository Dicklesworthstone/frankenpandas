## br-frankenpandas-uza04.129 - df_dot packed-panel/reuse rejection

Date: 2026-06-14
Agent: LavenderStone

### Baseline

- Build: `CARGO_TARGET_DIR=.rch-target-lavenderstone-uza04129-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH worker: `vmi1227854`
- Hyperfine: `df_dot 100000 6` mean `806.7 ms +/- 32.3 ms`
- Internal harness: `df_dot 100000 6` `117.161 ms/iter`
- Goldens:
  - `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
  - `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`
- `perf stat -d` was blocked by `perf_event_paranoid=4`.

### Candidate 1: A slice-table dispatch hoist

- Lever: replace hot-loop `Cow<[f64]>` deref with a prebuilt `&[f64]` slice table.
- Behavior: `df_dot 2000` and `df_dot 5000` goldens matched byte-for-byte.
- Internal harness: `118.696 ms/iter` versus `117.161 ms/iter` baseline.
- Verdict: rejected; source hunk removed.

### Candidate 2: B-panel outer loop order

- Lever: process each packed `B` panel across the row band before moving to the next panel.
- Behavior: `df_dot 2000` and `df_dot 5000` goldens matched byte-for-byte.
- Internal harness: `229.618 ms/iter` versus `117.161 ms/iter` baseline.
- Verdict: rejected; source hunk removed.

### Isomorphism

- Ordering preserved: yes for both candidates; output row/column assembly was unchanged.
- Tie-breaking: not applicable.
- Floating-point: each cell kept independent `l=0..k` ascending `acc += a*b` fold.
- RNG: none.
- Golden outputs: baseline/candidate byte comparisons passed for both candidates.

### Routing

Do not repeat dot metadata-deref or loop-order micro-levers. The next pass should either attack a fundamentally different dot primitive with a larger structural change, or reprofile and switch to the next measured non-dot hotspot.
