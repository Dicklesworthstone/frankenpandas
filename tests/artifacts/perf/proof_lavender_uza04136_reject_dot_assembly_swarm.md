# br-frankenpandas-uza04.136 reject: dot no-NaN assembly swarm bypass

## Target

Fresh post-`f00fdb65` routing kept `df_dot` as the top measured hotspot:
`df_dot 100000x5 = 110.949 ms/iter` in
`tests/artifacts/perf/lavender_next_f00fdb65_routing_matrix.txt`.

## Candidate

Rejected source candidate: after the existing per-band `column_has_nan` witness
proved the whole dot output was NaN-free, build the cheap output chunk
descriptors serially instead of spawning the second assembly worker swarm.

Arithmetic was unchanged during the candidate:

- each output cell kept the existing `l = 0..k` ascending f64 fold;
- SIMD multiply/add accumulators were unchanged;
- per-cell `value.is_nan()` detection during compute was unchanged;
- only the post-compute output-column assembly scheduler changed.

The source hunk was removed after measurement. This commit keeps evidence only.

## Golden Proof

Baseline:

- `df_dot golden 2000`:
  `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
- `df_dot golden 5000`:
  `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

Candidate:

- `df_dot golden 2000`:
  `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
- `df_dot golden 5000`:
  `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

Ordering, labels, tie behavior, RNG, and f64 fold order were not changed by the
candidate hunk.

## Timing

Baseline standalone via `rch exec -- hyperfine`:

- `df_dot 100000x5`: `609.4 ms +/- 9.4 ms`

Paired forward:

- baseline: `684.4 ms +/- 56.3 ms`
- candidate: `596.5 ms +/- 13.8 ms`
- ratio: `1.15x +/- 0.10`

Paired reversed:

- candidate: `615.4 ms +/- 14.6 ms`
- baseline: `636.9 ms +/- 24.7 ms`
- ratio: `1.03x +/- 0.05`

Confirm pair:

- baseline: `632.7 ms +/- 59.1 ms`
- candidate: `604.3 ms +/- 13.9 ms`
- ratio: `1.05x +/- 0.10`

## Decision

Rejected. The candidate is behavior-preserving, but the timing evidence is too
noisy and marginal to clear Score >= 2.0. Do not retry this assembly-thread
bypass as a standalone lever; any future dot work should attack a deeper kernel,
layout, or algorithmic primitive.
