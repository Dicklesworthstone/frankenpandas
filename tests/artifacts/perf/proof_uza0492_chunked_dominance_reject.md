# br-frankenpandas-uza04.92 - chunked dominance counter rejection

Timestamp: 2026-06-13T11:21:16Z
Agent: LavenderStone

## Target

`df_kendall` still spends about 90% of sampled cycles in
`Series::kendall_no_tie_fast_with_ordered_ranks` after the earlier typed
extraction and word-blocked rank-bitset keeps.

This pass tested one lever only: put a chunked dominance counter before the
existing word-blocked/Fenwick fallback. The candidate preserved the old fallback
for invalid ranks and duplicate-rank rejection, but it was still a single-pair
counter wrapper rather than true cross-column dominance sharing.

## Baseline/Profile

- RCH worker: `vmi1153651`
- Baseline build artifact:
  `/data/projects/.scratch/cargo-target-lavenderstone-uza0492-base/release-perf/examples/perf_profile`
- After build artifact:
  `/data/projects/.scratch/cargo-target-lavenderstone-uza0492-after/release-perf/examples/perf_profile`
- Baseline-only hyperfine:
  - `df_kendall 50000 1`: `117.6 ms +/- 2.6 ms`
  - `df_kendall 200000 1`: `489.8 ms +/- 14.0 ms`
- Perf profile:
  - `kendall_no_tie_fast_with_ordered_ranks`: `90.26%` self
  - `complete_kendall_no_tie_parallel_matrix` worker path: `91.33%` children

## Behavior Proof

Golden outputs were byte-identical before/after:

- `df_kendall 2000`: `acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1`
- `df_kendall 5000`: `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e`
- `df_kendall 20000`: `f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b`

Isomorphism:

- Ordering preserved: yes; the candidate only changed the inversion counter for
  already-ordered complete no-tie ranks.
- Tie-breaking unchanged: yes; duplicate ranks returned `None` and fell through
  to the existing fallback chain.
- NaN/null/non-finite fallback unchanged: yes; this helper is only reached after
  the complete-finite rank path admits the inputs.
- Floating-point bits unchanged: yes; discordance counts and final tau formula
  were unchanged, and all golden files were byte-identical.
- RNG unchanged: N/A.

Focused validation:

- `rch exec -- cargo test -p fp-frame --lib kendall_chunked_dominance_counter_matches_fenwick_boundaries -- --nocapture`

## Benchmark Gate

Paired forward:

- `50000x1`: baseline `119.8 ms +/- 6.2 ms`, after `139.6 ms +/- 5.7 ms`
  - Baseline ran `1.17x +/- 0.08` faster.
- `200000x1`: baseline `497.7 ms +/- 10.1 ms`, after `569.0 ms +/- 7.2 ms`
  - Baseline ran `1.14x +/- 0.03` faster.

Paired reversed:

- `50000x1`: after `139.6 ms +/- 5.2 ms`, baseline `119.3 ms +/- 4.4 ms`
  - Baseline ran `1.17x +/- 0.06` faster.
- `200000x1`: after `543.0 ms +/- 14.3 ms`, baseline `510.0 ms +/- 16.1 ms`
  - Baseline ran `1.06x +/- 0.04` faster.

Score: Impact `0` x Confidence `5` / Effort `2` = `0.0`.

## Decision

Reject. The candidate is proof-clean but slower in both benchmark orders and
does not satisfy the intended cross-column sharing primitive. The runtime source
hunk was removed; no `fp-frame` source diff is retained.

Next route: attack a truly shared offline dominance structure across target
columns for a fixed left-column order. Do not retry chunked single-pair counters,
static one-dimensional rank/select, row-major rank-signature batching,
multi-Fenwick/bitset batching, morsel scheduling, merge-sort, sqrt/block
counters, per-pair validation removal, or cache-layout micro-tweaks.
