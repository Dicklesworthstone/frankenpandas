# uza04.102 — parallel per-column outer-join assembly — KEPT (1.52x)

BlackThrush, 2026-06-13. perf_event_paranoid=4 (perf blocked); attribution via
env-gated `Instant` phase timers in `build_single_key_ordered_unique_outer_merge_output`
(reverted before the lever was written).

## Target

str_outer_join 100000 (single Int64 key, 50% overlap, 6+6 Utf8 value columns →
n=150000 output rows). Tracing the merge dispatch (the join code is heavily
path-fragmented; str_outer_join falls through the i64-fused fast paths because
its VALUE columns are Utf8) showed it resolves to
`build_single_key_ordered_unique_outer_merge_output`. Phase timers there:

```
setup ~6µs    assemble ~10.5ms   (serial per-column reindex over 13 columns)
```

The assembly looped `for name in columns { reindex_outer_join_column(col, positions) }`
SERIALLY across all 13 output columns — independent work run on one thread.

## Lever (bit-identical)

Build an ordered list of per-column build specs (KeyCoalesce / Reindex / Take),
compute each column (the O(output) gather) across workers via a work-stealing
`thread::scope`, then insert in the SAME first-seen left-then-right order.
Byte-identical: each output column is the same values in the same row order,
inserted in the same column order → `columns`/`column_order` unchanged.

## Proof

- Golden sha256 BYTE-IDENTICAL for str_outer_join / str_left_join / inner_join /
  outer_join / left_join at n=2000 and n=5000 (10/10) — confirms no other join
  path regressed.
- `cargo test -p fp-join`: 113 passed, 0 failed.
- **str_outer_join 100000: 30.962 → 20.429 ms = 1.52x** (min-of-8, rch worker).

## Honest accounting

1.52x is below the 2.0 bench bar: the assembly was ~10.5ms of str_outer_join's
~31ms; the remainder is the sequential position computation
(`ordered_unique_int64_outer_positions` building 150k-row `Option<usize>`
position vectors) plus the bench's `DataFrame::new_with_column_order` re-validation
of the 13×150k merged frame and the output drop. The lever is a genuine
serial→parallel structural fix (not a micro-tweak) and benefits the whole
outer-join family, so it is kept.

## Next swing (reporting rule)

To clear 2x on str_outer_join, the next lever is the **sequential position
computation** (`ordered_unique_int64_outer_positions`) — building the merged
left/right position vectors is an O(n) sequential merge that could be a
partitioned/parallel merge — plus avoiding the redundant frame re-validation on
an already-validated merge output. Profiler-gated (paranoid<=1) to confirm the
position-compute fraction precisely.
