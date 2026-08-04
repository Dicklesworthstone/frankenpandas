# br-frankenpandas-uza04.77 Pass 2 Primitive Selection

Timestamp: 2026-06-11T04:22:00Z
Target: residual `filter_bool` mask witness verification in `DataFrame::loc_bool`

## Hotspot Evidence

Pass 1 on current `origin/main` (`167ff9bf`) confirmed:

- `filter_bool 100000 1000`: `60.3 ms +/- 4.6 ms`.
- `filter_bool 100000 20000`: `0.009 ms/iter`, 270 perf samples.
- `<fp_frame::DataFrame>::loc_bool`: 46.46% self, 55.15% children.
- Annotation local samples are concentrated in the current every-other repeated
  mask verifier. The compiler already lowers the current 8-bool chunk loop into
  one unaligned 64-bit compare per 8 mask bytes.

## Selected Primitive

64-byte repeated-mask block verifier.

The current `.75` recognizer already proves the every-other mask shape. This
pass keeps the same predicate and certificate but raises the verifier grain
from 8 mask bytes to 64 mask bytes, reducing loop trips by 8x on the
profile target while preserving the existing fallback path.

## Graveyard / Artifact Mapping

- `alien_cs_graveyard.md` §8.2 Vectorized Execution: selection-vector and mask
  pipelines must stay row-isomorphic to materialized pipelines.
- `alien_cs_graveyard.md` §7.1 Succinct structures: compact bit/word-level
  evidence for selection predicates.
- `alien-artifact-coding` certified rewrite obligations: the accepted rewrite
  must be equivalent over the declared every-other mask domain and preserve the
  baseline path for nonmatching inputs.

## Score

Impact 3 x Confidence 4 / Effort 2 = 6.0.

This clears the Score >= 2.0 gate because the fresh profile directly targets
the verifier loop and the source lever is a local equivalence-preserving block
rewrite.

## Non-Repeat Boundary

- Not `.75`: `.75` added the every-other recognizer. This only changes the
  verifier grain inside the same predicate.
- Not `.76`: no constructor bypass, no `DataFrame::new_with_axes` shortcut, no
  column/index metadata normalization changes.

## Proof Obligations

- Return `{ start: 0, step: 2, len: ceil(mask.len() / 2) }` only when every
  `mask[i] == (i % 2 == 0)`.
- Return `{ start: 1, step: 2, len: floor(mask.len() / 2) }` only when every
  `mask[i] == (i % 2 == 1)`.
- Preserve current tiny-mask behavior: fewer than two selected rows fall
  through to the existing general affine builder.
- Check all tail bytes exactly; no padding reads and no out-of-slice assumptions.
- Use only safe slice equality over `[bool]`; no casts, transmutes, or bool
  representation assumptions.
- Nonmatching masks must fall back to the current general affine scan and
  materialized-position path.
- Golden SHA for `filter_bool 1000` and `filter_bool 100000` must remain
  unchanged.

## Fallback / Rejection Trigger

Reject and remove the source hunk if either command-order paired hyperfine
does not show a real win of about 1.10x or better, if confidence intervals
overlap no-win as in `.76`, or if after-profile simply moves the same work into
a `memcmp`/block-compare call without improving the benchmark.

If rejected, route the next bead away from block-size tweaks and toward a
producer-carried immutable mask witness or typed/bitpacked boolean mask
certificate accepted by `loc_bool` without rescanning ordinary slices.
