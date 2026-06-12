# REJECTED — cache-friendly inversion counter for df_kendall (0dm7c premise refuted)

## Target
`df_kendall` (32-col Kendall correlation matrix) = ~617 ms/iter at n=200k — the
single largest perf number in the project and **autonomous** (integer inversion
counting, NOT FMA-gated, unlike corr/cov/spearman). The matrix path
(`complete_kendall_no_tie_parallel_matrix` → `kendall_no_tie_fast_with_ordered_ranks`
→ `count_ordered_rank_inversions`) counts discordant pairs with a u32 Fenwick/BIT.
Bead 0dm7c hypothesised a "random-access Fenwick wall" (cache misses) and a
cache-friendly replacement worth ≥2×.

## Lever attempted: merge-sort inversion count (cache-friendly, sequential)
Gather the y-ranks into x-sorted order once, then count strict inversions with a
sequential merge sort (the integer mirror of the existing `count_f64_inversions`)
instead of the random-access BIT.

- **Bit-identical (verified):** ranks are a permutation, so strict inversions of
  the gathered sequence == the Fenwick's `seen − (#seen ≤ rank)` total. Golden
  shas UNCHANGED — n=2000 `acf366c2…`, n=5000 `031978ba…`, n=20000 `f164fa86…`.
- **REGRESSED:** df_kendall 200k min-of-4 **685 ms (merge-sort) vs 617 ms
  (Fenwick) — ~1.11× SLOWER.** Reverted. (Reconfirms the prior
  corr-join-perf-rejected-levers note "merge-sort-inversions slower than Fenwick"
  — now with hard numbers.)

## Root cause: 0dm7c's cache-wall premise is WRONG
The u32 Fenwick is `vec![0u32; n+1]` ≈ **800 KB at n=200k** (already halved from
u64 by br-frankenpandas-322yd). 800 KB is **L3-resident** on any modern CPU (L3 ≥
8 MB), so its O(log n) "random" updates/queries are ~L3 hits (~12 cyc), not
main-memory misses. There is no cache wall to exploit:

- Merge sort does O(n·log n) ≈ 3.6M sequential **writes** per pair — far more
  total work than the Fenwick's ~2·log₂n ≈ 36 L3-hit accesses per row. Sequential
  ≠ free; the larger write volume loses.
- A √-decomposition (2-level bucketed, L1-resident) would do ~2√n ≈ 900 L1 ops
  per row vs the Fenwick's ~36 L3-hit ops — analytically ~900 cyc vs ~432 cyc,
  also slower.

Conclusion: the kendall inversion count is **compute-bound at the FENWICK's
O(n·log n) with already-cache-resident state**, not cache-layout-bound. No
data-structure swap beats it. The only remaining lever is algorithmic reduction
of the work itself (fewer than O(n·log n) per pair, or cross-pair sharing) — a
fundamentally different, much harder swing, not a cache-friendly counter.

## Recommendation
Mark 0dm7c's "cache-friendly inversion counter" approach as **not viable** (BIT is
L3-resident). The autonomous kendall win, if any, must come from reducing the
per-pair O(n·log n) or amortizing across the 496 pairs — file as a separate
algorithmic bead if pursued. Meanwhile the largest addressable corr/cov/spearman
block (~900 ms) remains gated on the **jawxr FMA build-decision (orchestrator
sign-off)**, which is the highest-leverage unblock.
