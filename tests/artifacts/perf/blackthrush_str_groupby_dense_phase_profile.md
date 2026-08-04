# str_groupby dense path — phase attribution (BlackThrush, 2026-06-13)

Dynamic profilers are unavailable in this environment (`perf_event_paranoid=4`,
no passwordless sudo, no valgrind), so attribution was obtained by temporary
env-gated `Instant` phase timers inside `DataFrameGroupBy::aggregate_str_dense`
(crates/fp-frame/src/lib.rs ~52252). The instrumentation was reverted after
measurement (lib.rs diff is clean).

## Harness

`release-perf` build of `fp-conformance` example `perf_profile`, scenario
`str_groupby_*` at n=200000. Numbers are per-call (one groupby+agg), min-of-6.

```
FP_GB_PROF str_dense nrows=200000 ng=200000 group=~11ms  out_idx=~11ms  agg(count)=~3ms  frame=~4us
                                  ^^^^^^^^^^
```

## Finding 1 — the benchmark keys are all-UNIQUE (ng == nrows)

`build_str_key_frame(n, key_cardinality)` (perf_profile.rs:329) writes each key
as `key_{mixed:016x}_{key_id:04x}`. The embedded full 64-bit `mixed` makes every
key unique, so the `key_id = mixed % cardinality` suffix is vestigial: the
`cardinality=4096` argument is defeated and `ng == n == 200000`. The `str_groupby_*`
benchmarks therefore measure the DEGENERATE "every row is its own group" path
(near_all_distinct => full MSD-argsort branch), NOT realistic K<<n grouping.

Consequence: a realistic low-cardinality string-groupby benchmark is MISSING.
With genuinely repeating keys the op hits the O(n) hash-group branch (ng=K) and
is far cheaper. The golden builder `build_str_key_frame_repeated(n, 64)` DOES
produce real grouping, so golden output and the benchmark exercise different
data shapes for the same scenario name.

## Finding 2 — cost split on the degenerate path (the real target)

For ng==n the two dominant phases are ~equal:

- **group ~11ms**: `fp_columnar::utf8_msd_argsort_bytes` over 200k ~26-byte spans.
- **out_idx ~11ms**: building the output index — the loop at lib.rs ~52378
  gathers each group's key span in SORTED order:
  `out_key_bytes.extend_from_slice(&bytes[offsets[r]..offsets[r+1]])` where
  `r = first_row[g]` is the original (unsorted) row. For ng==n that is 200k
  random reads scattered across the 5MB input buffer (cache-miss bound), into an
  `out_key_bytes` Vec allocated with ZERO capacity (realloc churn), followed by
  two `Arc::from(Vec)` copies.
- **agg ~3ms** (count); var/std ~6-9ms (two passes).

## Lever (proposed, bit-identical, ~1.8x ceiling)

Fuse the output-byte gather INTO the MSD argsort. `utf8_msd_argsort_bytes`
already visits spans in sorted order during its final stable distribution pass;
a variant that ALSO emits the concatenated sorted byte buffer + offsets makes the
out_idx phase nearly free (no scattered re-gather, no realloc churn), collapsing
~25ms -> ~14ms (~1.8x). Bit-identical: same sorted key bytes, same offsets, same
Index. Applies only to the sort=True near_all_distinct branch.

Caveats: (a) ~1.8x is BELOW the Score>=2.0 bar on its own; (b) it optimizes a
benchmark-artifact (degenerate) shape. Pre-sizing `out_key_bytes` to the total
span byte length and building the Arc backing without the intermediate Vec copy
is a smaller independent bit-identical cleanup, also sub-2.0 alone.

Recommendation: fix the benchmark builder to honor `key_cardinality` (drop the
`{mixed:016x}` prefix or bound it) so str_groupby measures realistic grouping;
then re-profile the realistic hash-group branch for the actual no-gaps target.
