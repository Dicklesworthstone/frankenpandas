# br-frankenpandas-uza04.60 periodic run-schedule rejection

Agent: OrangePeak
Date: 2026-06-09
Target: `perf_profile outer_join 500000`

## Profile-backed target

The baseline `outer_join 500000x200` perf sample kept the dense outer builder as
the active residual:

- `build_single_key_dense_i64_outer_merge_output` dominated the join output path.
- `build_single_key_dense_i64_outer_merge_output::{closure#9}` was 20.95% self
  in the sampled profile.
- Baseline golden output SHA for `outer_join 20000`:
  `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.

Baseline timings:

- Direct `outer_join 500000x20`: 20 iterations in 0.325s, 16.250 ms/iter.
- Direct `outer_join 500000x200`: 200 iterations in 2.193s, 10.967 ms/iter.
- Hyperfine `outer_join 500000x20`: 396.2 ms +/- 28.7 ms.
- Hyperfine `outer_join 500000x200`: 2.238 s +/- 0.054 s.

## Candidate

The candidate was a proof-carrying dense outer periodic run-schedule descriptor.
It certified real left/right key slices as same-period dense cycles, derived the
outer bucket schedule, and fell back to the generic dense outer route on any
mismatch, gap, period mismatch, or overflow. The intended win was to remove plan
construction and redundant plan walks, not to repeat CSR-only certification.

Isomorphism obligations while the hunk was present:

- Output bucket order remains ascending dense-key order.
- Matched buckets preserve left-major tie order and right row order.
- Left-only and right-only edge buckets stay in the same side-only positions.
- Dtype promotion, null/NaN behavior, Float64 payload bits, and RNG/hash behavior
  are unchanged because only schedule construction changed.
- Any certificate failure falls back before output materialization.

Focused helper tests and the dense outer generic-parity test passed while the
candidate hunk was present. The hunk was removed after the score gate rejected it.

## Behavior proof

Baseline golden SHA:

`453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`

Candidate golden SHA:

`453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`

`cmp` result: byte-identical.

## Score gate

Paired hyperfine:

- `outer_join 500000x20`: baseline 346.6 ms +/- 12.7 ms, candidate 328.3 ms +/- 14.1 ms, candidate 1.06x +/- 0.06.
- `outer_join 500000x200`: baseline 2.178 s +/- 0.080 s, candidate 2.115 s +/- 0.066 s, candidate 1.03x +/- 0.05.

Direct timings did not corroborate a robust win:

- `outer_join 500000x20`: baseline 16.250 ms/iter, candidate 17.054 ms/iter.
- `outer_join 500000x200`: baseline 10.967 ms/iter, candidate 11.060 ms/iter.

Score: Impact 0 x Confidence 4 / Effort 2 = 0.0.

Verdict: reject; no source code retained.

## Next route

Do not repeat these rejected families:

- `.56` cursor-free CSR construction.
- `.57` multi-lane value-tape batching.
- `.59` periodic CSR-only certification.
- `.60` periodic run-schedule certification.

The next profile-backed primitive should attack promoted left-lane value
materialization directly: a source-position-backed lazy repeat-values column or
lane-level shared row-position descriptor that lets promoted Float64 lanes read
source values at materialization time instead of eagerly collecting per-lane
`run_values` vectors. Preserve output ordering, tie-breaking, dtype promotion,
null/NaN semantics, exact Float64 bits, and RNG/hash behavior.
