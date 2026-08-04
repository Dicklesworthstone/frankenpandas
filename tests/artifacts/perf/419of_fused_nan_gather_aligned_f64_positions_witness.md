# br-frankenpandas-419of — fused NaN-tracked gather in position-based aligned_binary_f64

## Lever (one)
`Column::aligned_binary_f64` (the different-index/reindex Float64 alignment path,
fp-frame series add ~4199) paid up to four memory passes: two full input-column
`nan_aware_validity` pre-scans + the gather/apply main loop + the
`from_f64_values` output re-scan. Replaced the two pre-scans with an inline gate
at the GATHERED positions only — `self.validity.get(i) && !lsrc[i].is_nan()`,
which equals `nan_aware_validity(self).get(i)` (the validity bit covers Null
slots; the `is_nan` term covers a still-valid `Float64(NaN)`) — and stamp the
all-valid output via `from_f64_values_all_valid_unchecked` (no re-scan; `data`
proven NaN-free when the loop keeps `all_valid`).

## Isomorphism proof
- Per-element validity decision identical: `validity.get(i) && !lsrc[i].is_nan()`
  reproduces `nan_aware_validity(self).get(i)` for both the cached-f64 case
  (same data scanned) and the from-scalars case (`Float64(NaN)` → NaN datum →
  excluded; `Null(NaN)` → validity bit already false). Same for the right side.
- Bounds: `validity.len() == lsrc.len()`, and `validity.get(i)` short-circuits
  (false when out of range), so `lsrc[i]` stays in bounds — matching the old
  `lvalid.get(i)` OOB-as-invalid behaviour.
- `all_valid` ⟹ every emitted value non-NaN ⟹ `data` NaN-free, so the unchecked
  all-valid stamp == the old `from_f64_values(data)`. Generated-NaN / gappy
  outputs still route to `from_f64_values_with_validity`, unchanged.
- ordering / positions / Null(NaN)-vs-Float64(NaN) / validity / f64 bits /
  fallback: preserved. No unsafe.

## Golden (byte-identical before==after)
`bench_aligned_f64_positions` digests f64 bits + per-row validity for all 7 ops,
n=100000, FULL 1:1 positions and a GAPPY variant (every 7th right slot None), all
14 identical before and after:
add full `ce1f207031ac262e` gappy `2f464eac09ec94e8`; sub `a4d7b3136b8d1285`/`e4a76b97f42130c3`;
mul `06348b3433a34fe8`/`248cb564a17d7532`; div `72b9e6b279eecef3`/`06a96b2c973664ea`;
mod `a4d7af5e77c2a75f`/`9726068cc99dc0e6`; pow `c8bcf87d9f2bf83c`/`75acd12736e50ea9`;
floordiv `23f86c13ec6d2f98`/`396cf58a4d254bc3`.
Cross-check: the FULL-position digests equal the same-index (9houf) goldens, and
`aligned_binary_f64_same_positions_matches_identity_alignment` stays green.

## Benchmark (n=100000, op-only loop)
- min-of-5 internal per-iter (add): before ~0.667 ms → after ~0.555 ms = **1.20×**.
- hyperfine -N paired (600 iters, incl. one-time frame build): forward 1.23×,
  reversed 1.13×. Ordering-independent, reproducible.

## Honest scope / next target
This path is GATHER-bound: the `Option<usize>`-indexed main loop (random reads +
per-element word bit-setting) dominates, so removing the two input scans captures
only ~17% of the cost (1.20×, not the 3.2× the contiguous same-index 9houf path
got). The remaining headroom is the gather itself — e.g. routing
contiguous/affine position runs to the slice kernel, or a branchless validity
fold — which is the deeper lever to attack next, not more scan trimming.

## Gates
- fp-columnar lib 404/0 (incl. identity-alignment + same_positions==general
  NaN/inf locks).
- fp-frame arithmetic integration 59/0; clippy -D warnings clean; fmt clean.
