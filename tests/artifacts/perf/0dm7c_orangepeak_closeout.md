# br-frankenpandas-0dm7c closeout

## Verdict

Rejected the cache-friendly inversion-counter family for
`DataFrame.corr(method="kendall")`.

This bead targeted the premise that `count_ordered_rank_inversions` was slow
because the Fenwick tree's random rank walks were cache-miss bound. Current-head
evidence and the prior rejection artifact both point the other way: at the
diagnostic sizes used here, the u32 Fenwick array is cache-resident and the
remaining wall is algorithmic work, not memory layout.

No production source change is retained for this bead.

## Current-head witness

RCH build:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-0dm7c-base
RUSTFLAGS='-C force-frame-pointers=yes'
rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Worker: `vmi1227854`.

Golden outputs:

```text
df_kendall 2000:  acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1
df_kendall 5000:  031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e
df_kendall 20000: f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b
```

Current-head hyperfine:

```text
df_kendall 50000 1:  154.5 ms +/- 5.4 ms
df_kendall 200000 1: 698.3 ms +/- 6.6 ms
```

`perf record` is blocked on this host by `perf_event_paranoid=4`; stderr is
retained at
`tests/artifacts/perf/0dm7c_orangepeak_current_perf_df_kendall_200000x1.stderr`.
The existing profile-backed rejection artifact remains the direct hotspot
witness:
`tests/artifacts/perf/0dm7c_rejected_kendall_inversion_cache_layout.md`.

## Why this bead closes

Rejected or non-repeat families:

- Per-worker Fenwick buffer reuse: neutral; calloc already gives cheap zeroed
  pages and explicit reset adds work.
- Merge-sort inversion counting: bit-identical but slower at the 200k shape.
- Sqrt/block counters: analytically worse because they trade about 36
  cache-resident Fenwick accesses per row for hundreds of block operations.
- Rank-cache layout shuffles: same operation count, no route to Score >= 2.0.

The correct next target is not a more cache-friendly Fenwick. It is an exact
all-pairs Kendall primitive that shares work across the 32 rank columns or
reduces the per-pair `O(n log n)` inversion work.

## Next primitive card

Follow-up target:
`br-frankenpandas-uza04.87`, batched all-pairs Kendall rank-signature or
offline dominance counting.

Graveyard mapping:

- `alien_cs_graveyard.md` section 7.1, succinct rank/select structures:
  rank/select and compact rank witnesses are the useful abstraction, but only
  if made static/batched rather than rebuilding a dynamic Fenwick per pair.
- `alien_cs_graveyard.md` section 8.2, vectorized execution:
  process multiple rank columns as a batch/morsel so per-row dispatch and
  x-order traversal are shared across many target columns.
- `alien_cs_graveyard.md` section 8.17, AMAC/coroutine interleaving:
  only applicable if a profile proves LLC stalls for batched independent rank
  lookups; otherwise skip it because the current Fenwick state fits cache.
- `high_level_summary...md` section 0.16, proof-carrying artifacts:
  any precomputed rank-signature table or static dominance witness needs a
  SHA-256 proof artifact and conservative fallback to the current Fenwick path.

Proof obligations:

- Exact discordance counts for every `(i, j)` pair.
- Same matrix order, diagonal, and symmetry.
- Same no-tie finite fast-path eligibility.
- Same NaN, null, tie, and non-numeric fallback behavior.
- Same `f64` output bits for `tau = 1 - 2 * discordant / denom`.

Score gate for the follow-up:
target at least `1.5x` on `df_kendall 50000 1` and `df_kendall 200000 1`,
with paired/reversed timing and unchanged golden hashes above.
