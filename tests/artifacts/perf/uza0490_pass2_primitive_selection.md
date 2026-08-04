# br-frankenpandas-uza04.90 Pass 2 Primitive Selection

Date: 2026-06-13
Worktree: `/data/projects/.scratch/frankenpandas-uza0489-orangepeak-20260613`
Scope: evidence/artifact only; no runtime source edits.

## 1. Measured Hotspot And Baseline

Target residual after `.89`:
`complete_kendall_no_tie_parallel_matrix` ->
`Series::kendall_no_tie_fast_with_ordered_ranks` ->
`Series::count_ordered_rank_inversions`.

Pass 1 baseline witness:
`tests/artifacts/perf/proof_uza0490_pass1_baseline_profile_witness.md`.

Current baseline:

| Workload | Mean | Median | p95 | Golden status |
| --- | ---: | ---: | ---: | --- |
| `df_kendall 50000 1` | `139.704 ms` | `135.217 ms` | `177.645 ms` | unchanged |
| `df_kendall 200000 1` | `589.332 ms` | `588.837 ms` | `606.867 ms` | unchanged |

Current golden SHA256 values:

```text
df_kendall 2000:  acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1
df_kendall 5000:  031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e
df_kendall 20000: f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b
```

Kernel sampling is blocked on this host by `perf_event_paranoid=4`, so this pass
uses the pass 1 timing/resource witness plus the routed residual function path
instead of a fresh `perf report` percentage.

## 2. Rejected And Non-Repeat Families

Do not repeat these families for `.90`:

- Row-major multi-Fenwick batching: rejected in `.88`; it reduced outer traversals
  but increased active Fenwick working set and regressed both 50k and 200k gates.
- Morsel-size scheduling: `.88` morsel 4 -> 8 was noise/neutral after reversed
  confirmation; scheduling alone does not change the inversion primitive.
- Merge-sort inversion counting: `0dm7c` showed bit-identical goldens but slower
  200k timing than the current Fenwick path.
- Sqrt/block counters: rejected analytically in `0dm7c`; scanning hundreds of
  block operations per row is worse than the cache-resident Fenwick walk.
- Per-pair validation removal: `proof_0dm7c_rejected_prevalidated_kendall.md`
  showed no keep-worthy delta and a 200k regression.
- Per-worker buffer/cache-layout reuse: `0dm7c` closeout says the current u32
  Fenwick state is cache-resident; cache layout alone is not the lever.
- Bit-parallel row-pair bitset matrix: previous Kendall bitset-matrix attempt
  preserved the golden SHA but regressed `df_kendall 512 20` from about `41.8 ms`
  to `143.0 ms`; reject any construction with `O(cols * rows^2)` setup.

## 3. Selected Candidate Contract

Change:
replace the hot exact no-tie inversion counter with a word-blocked dynamic rank
bitset helper for trusted permutation rank witnesses. Keep the current Fenwick
counter as fallback for general or untrusted paths.

Candidate:

```text
seen_words: Vec<u64> with one bit per y-rank
block_counts: Fenwick tree over 64-rank word counts

for row in x_order:
    rank = y_rank_by_row[row]
    block = rank >> 6
    offset = rank & 63
    lower_blocks = block_counts.prefix_sum_exclusive(block)
    mask = if offset == 63 { !0_u64 } else { (1_u64 << (offset + 1)) - 1 }
    in_word = popcount(seen_words[block] & mask)
    rank_or_lower = lower_blocks + in_word
    inversions += seen - rank_or_lower
    set seen_words[block].bit(offset)
    block_counts.add(block, 1)
```

Mapped graveyard source:
`/data/projects/alien_cs_graveyard/alien_cs_graveyard.md` section 7.1,
Succinct Data Structures. The relevant primitive is the rank/select bitvector
pattern: raw `Vec<u64>` words, prefix rank decomposition, and `popcount` for
intra-word rank. This candidate is dynamic, so the inter-word prefix is still a
small Fenwick tree, but the rank universe drops from `n` ranks to `ceil(n / 64)`
blocks.

Proof-carrying artifact source:
`/data/projects/alien_cs_graveyard/high_level_summary_of_frankensuite_planned_and_implemented_features_and_concepts.md`
sections 0.2/0.16/claim-policy rules. The implementation pass must attach
baseline, golden checksums, isomorphism proof, paired benchmark delta, and
fallback behavior.

Priority tier:
A. This is a narrow internal hot-path primitive with deterministic fallback and
no public API change.

Adoption wedge:
add a private helper under the existing complete/finite/no-tie Kendall matrix
fast path. Do not alter scalar Kendall, tie handling, NaN/null handling, or the
general fallback.

Ecosystem scan:
the current workspace manifests have no existing `bitvec`, succinct, roaring,
or bitmap dependency. Do not add a dependency for this pass; the primitive is
small enough to implement with `Vec<u64>`, `Vec<u32>`, and `u64::count_ones`.

Budgeted mode:
per pair, allocate or reuse `ceil(n / 64)` words plus `ceil(n / 64) + 1`
block-Fenwick counters. For `n = 200000`, that is about 3125 words and 3126
counters instead of `n + 1` Fenwick counters. On length overflow, out-of-range
rank, duplicate rank bit, or a configured scratch-size budget breach, return
`None` and use the current Fenwick/general path.

Baseline comparator:
current `Series::count_ordered_rank_inversions`, a u32 Fenwick over the full
rank universe.

## 4. Scores

EV score:

```text
Impact = 4
Confidence = 3
Reuse = 3
Effort = 2
Adoption friction = 2

EV = (Impact * Confidence * Reuse) / (Effort * Adoption friction)
EV = (4 * 3 * 3) / (2 * 2) = 9.0
```

Extreme-optimization opportunity score:

```text
Impact * Confidence / Effort = 4 * 3 / 2 = 6.0
```

Relevance score:

| Axis | Score | Weight | Weighted |
| --- | ---: | ---: | ---: |
| Symptom fit | 5 | 0.30 | 1.50 |
| Architecture fit | 4 | 0.25 | 1.00 |
| Project fit | 4 | 0.20 | 0.80 |
| Proof readiness | 5 | 0.15 | 0.75 |
| Operability | 4 | 0.10 | 0.40 |
| Total | | | 4.45 / 5.00 |

## 5. Isomorphism Proof Plan And Fallback Trigger

Let `r_t = y_rank_by_row[x_order[t]]`. The current Fenwick path computes, at
step `t`:

```text
rank_or_lower = |{ s < t : r_s <= r_t }|
inversions += t - rank_or_lower
```

The selected helper keeps the same seen set in a bitvector. For
`block = r_t / 64` and `offset = r_t % 64`:

```text
block_counts.prefix_sum_exclusive(block) =
    |{ s < t : floor(r_s / 64) < block }|

popcount(seen_words[block] & low_mask(offset)) =
    |{ s < t : floor(r_s / 64) == block and r_s % 64 <= offset }|
```

The sum is exactly `|{ s < t : r_s <= r_t }|`, so the discordant-pair count is
identical. The final tau formula remains unchanged:

```text
(n_pairs - 2.0 * discordant) / n_pairs
```

Required proof checks for the implementation pass:

- Focused unit/property test comparing the word-blocked helper against
  `count_ordered_rank_inversions` for valid permutation rank witnesses,
  including empty, length 1, lengths around 63/64/65, and a larger deterministic
  permutation.
- Existing Kendall tests, including
  `complete_kendall_parallel_matrix_matches_serial_ordered_ranks`.
- Golden verification for `df_kendall 2000`, `5000`, and `20000` with the three
  baseline SHA256 values above.
- Paired and reversed hyperfine gates for `df_kendall 50000 1` and
  `df_kendall 200000 1`.

Fallback trigger:

- Any non-Kendall method.
- Any incomplete, non-finite, tied, null, or non-numeric column path.
- Missing `order` or `rank_by_row` witness.
- Length mismatch between `x_order` and `y_rank_by_row`.
- Out-of-range row/rank or duplicate rank bit observed by the helper.
- Any future evidence that the helper fails the paired/reversed benchmark gate.

Fallback action:
use the existing `count_ordered_rank_inversions`/general Kendall path.

## 6. Expected Before/After Target

The candidate reduces each rank query/update from a full-universe Fenwick over
`n` counters to a block Fenwick over `ceil(n / 64)` counters plus one intra-word
`popcount`. At `n = 200000`, this changes the prefix/update tree height from
about 18 to about 12 and shrinks the per-pair mutable counter footprint from
about 800 KB to about 37 KB.

Expected target:

| Workload | Baseline mean | Target mean | Required result |
| --- | ---: | ---: | --- |
| `df_kendall 50000 1` | `139.704 ms` | `<= 122 ms` | at least 1.14x faster |
| `df_kendall 200000 1` | `589.332 ms` | `<= 505 ms` | at least 1.16x faster |

Keep gate:
paired and reversed runs must both favor the candidate, p95 must not regress
materially, and all listed goldens must stay byte-identical.

Reject gate:
if the 200k gate is neutral or slower, remove the source hunk and route away
from rank-counter micro-structure toward a truly cross-pair or sub-`n log n`
Kendall primitive.

## 7. Why This Is Distinct From Prior Rejections

This is not row-major multi-Fenwick batching:
it changes the per-pair rank primitive and does not process several `(i, j)`
pairs through multiple active full Fenwick trees.

This is not morsel-size scheduling:
worker granularity and pair order stay unchanged.

This is not merge-sort inversion counting:
it remains an online seen-rank prefix counter and keeps the same one-pass
ordered-rank sweep.

This is not a sqrt/block counter:
there is no linear block scan or `O(sqrt n)` decomposition. Full lower blocks
are answered by a Fenwick prefix over 64-rank words, and the current block is
answered by one word `popcount`.

This is not per-worker buffer/cache-layout reuse:
the main effect is a different rank/select-style operation count and universe,
not preserving the same full Fenwick accesses in a warmer allocation.

This is not the rejected row-pair bitset matrix:
it never builds pairwise row relation matrices and has no `O(cols * rows^2)`
construction. It stores only the dynamic seen set for the current exact
inversion sweep.

## Recommendation

Select the word-blocked dynamic rank bitset as the `.90` implementation
candidate. It is the only candidate in this pass with a direct isomorphism to
the exact inversion count, a narrow fallback boundary, and a plausible
Score >= 2.0 route against the current `df_kendall` baseline.
