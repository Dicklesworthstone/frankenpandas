# uza04.97 — contiguous-source Utf8 reindex gather — REJECTED (neutral)

BlackThrush, 2026-06-13. perf_event_paranoid was briefly -1 this session
(profiling worked for str_outer_join), then flipped back to 4 (blocked again).

## Target (profile-backed, str_outer_join 200000, perf -g -F 1999)

```
21.42%  <fp_columnar::Column>::reindex_by_positions   <- top self
 6.54%  fp_join::ordered_unique_int64_outer_positions
 5.84%  __memmove_avx_unaligned_erms
 4.90%  fp_join::build_single_key_ordered_unique_outer_merge_output
 4.20%  <fp_columnar::Column>::new
```

`reindex_by_positions` feeds all 12 null-introducing Utf8 column gathers of the
outer join. The existing cmxjz branch calls `as_all_valid_str_vec()`, which for a
CONTIGUOUS source materializes a `Vec<&str>` over the whole source
(`offsets.windows` + per-row `from_utf8` revalidation), then scatter-indexes it
into a zero-capacity `new_bytes`.

## Lever (bit-identical) and result

Added a contiguous-source branch that copies each present span straight from
`bytes[offsets[idx]..offsets[idx+1]]` (validity by construction, no `from_utf8`,
no `Vec<&str>`).

- Goldens BYTE-IDENTICAL across str_outer_join / str_left_join / reindex_str /
  str_series_take at n=2000 and n=200000 (8/8 sha256 match).
- **Two-pass variant (pre-size new_bytes via a span-length pass): REGRESSED**
  str_outer_join 36.8 -> 42.8 ms (+16%), str_left_join 13.4 -> 16.0 ms (+19%).
  The extra scattered pass over `positions` costs more than the `Vec<&str>`
  elimination saves.
- **One-pass variant (avg-width reserve, no second pass): NEUTRAL**
  str_outer_join 36.0 vs 36.8 ms, str_left_join 13.1 vs 13.4 ms (within noise).

## Conclusion

The Utf8 reindex gather is memcpy/scatter-bound: the scattered span `memmove`s
dominate, and `from_utf8` + the `Vec<&str>` allocation are free relative to them.
Eliminating them is neutral. Score << 2.0 → rejected, source reverted (fp-columnar
diff empty). The branch only ever fires for null-introducing gathers (joins,
reindex-with-missing), where output >= source, so no shape benefits.

## Next swing (reporting rule)

str_outer_join cost is spread (no single >21% lever) and dominated by output
materialization — not a micro-lever target. The remaining project-wide perf
gaps are constrained, not "tapped out":
- Correlation Gram (df_corr 88ms) is bit-identical-walled: the only >=2x lever is
  FMA/reassociation, which shifts output bits and needs golden regen (jawxr,
  rejected). Bound by absolute parity, not by algorithm.
- Kendall (df_kendall 423ms) is correctly parallelized (work-stealing morsel loop,
  64 workers); the per-pair Fenwick inversion count is cache-miss-bound and has
  been ground across uza04.87-95 (all rejected).
- Highest-EV remaining: a CACHE-OBLIVIOUS / radix-partitioned inversion counter
  for the Kendall per-pair kernel that turns the Fenwick's random tree updates
  into sequential passes (the cache-miss wall, not the op count, is the cost).
  Needs the profiler stably unblocked (paranoid<=1) to validate.
