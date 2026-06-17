# br-frankenpandas-uza04.90 Pass 2 primitive selection

## Recommendation contract

### Change

Implement one safe-Rust Kendall inversion primitive in `crates/fp-frame/src/lib.rs`:
an MSB-first wavelet/rank-partition inversion counter for the complete finite
no-tie matrix path.

The Pass 3 source lever should be limited to the existing
`complete_kendall_no_tie_parallel_matrix` / `count_ordered_rank_inversions`
surface:

1. gather `right_rank_by_row[row]` in `left_order` order for one pair;
2. count inversions by stable MSB partitions over the rank sequence;
3. for each bit level, add `ones_seen` for every zero bit, because the highest
   differing bit is exactly where a previous rank is greater than the current
   rank;
4. convert the exact integer discordance count through the existing tau formula;
5. fall back to the current Fenwick helper whenever the budget/shape gate rejects.

This is not row-major multi-Fenwick batching, morsel scheduling, prevalidation
hoisting, merge-sort inversion counting, sqrt/block counters, per-worker buffer
reuse, cache-layout reshuffling, or typed Float64 extraction.

### Hotspot evidence

- Current post-.89 baseline artifacts:
  - `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_50000x1.txt`
  - `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_200000x1.txt`
  - `tests/artifacts/perf/uza0490_base_golden_sha256.txt`
- Baseline means:
  - `df_kendall 50000 1`: `135.2 ms +/- 9.5`
  - `df_kendall 200000 1`: `594.7 ms +/- 18.9`
- Golden hashes remain:
  - `df_kendall 2000`: `acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1`
  - `df_kendall 5000`: `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e`
  - `df_kendall 20000`: `f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b`
- `perf record` and `perf stat` are blocked by `perf_event_paranoid=4`; see
  `uza0490_base_perf_record_df_kendall_200000x1.stderr` and
  `uza0490_base_perf_stat_df_kendall_200000x1.stderr`.
- Residual code path:
  - `complete_kendall_no_tie_parallel_matrix`
  - `Series::kendall_no_tie_fast_with_ordered_ranks`
  - `count_ordered_rank_inversions` with a u32 Fenwick tree.

### Mapped graveyard sections

Canonical docs:

- `/data/projects/alien_cs_graveyard/alien_cs_graveyard.md` §7.1
  "Succinct Data Structures": rank/select bitvectors and wavelet-style
  bit-level witnesses are the closest match. The selected primitive uses the
  same rank-by-bit decomposition but compiles it directly into an exact
  inversion counter rather than a general reusable index.
- `/data/projects/alien_cs_graveyard/alien_cs_graveyard.md` §8.1
  "Worst-Case Optimal Joins": the relevant transfer is replacing independent
  pairwise work with a sorted-relation/offline dominance view. The chosen
  version is the smallest one-lever subset: one pair still returns one exact
  dominance count, but the witness is a static rank partition instead of a
  dynamic Fenwick update stream.
- `/data/projects/alien_cs_graveyard/alien_cs_graveyard.md` §8.2
  "Vectorized Execution + Morsel-Driven Parallelism": keep the existing
  matrix-level worker/morsel boundary, but make the inner pair counter a
  sequential rank-partition kernel with predictable scans.
- `/data/projects/alien_cs_graveyard/alien_cs_graveyard.md` §8.9
  "Provenance Semirings / Lineage Tracking": use only the proof shape, not the
  runtime metadata: each matrix cell must carry an exact integer-count
  explanation against the old Fenwick comparator in focused tests.
- `/data/projects/alien_cs_graveyard/high_level_summary_of_frankensuite_planned_and_implemented_features_and_concepts.md`
  §0.15 / §0.16: apply "Surface -> Failure Class -> Alien Math -> Compiled
  Runtime Artifact" and proof-carrying fallback discipline. Runtime artifact is
  the budget-gated wavelet counter; fallback artifact is the current Fenwick
  path.
- Same summary, FrankenSQLite concept table: §8.1 and §8.2 are marked high
  impact for query-style kernels. Here the DataFrame Kendall matrix is the
  query-like kernel.
- Same summary, shared foundation crates: mirror the `franken_bench` /
  `franken_evidence` spirit with local `uza0490_*` artifacts instead of adding
  a crate.

Skill references:

- `extreme-software-optimization`: one lever, profile/baseline first,
  Score >= 2.0, golden outputs, isomorphism proof, rebenchmark.
- `alien-graveyard`: recommendation contract, EV + relevance + risk gate,
  budgeted mode, fallback trigger, repro artifact pack.
- `alien-artifact-coding`: behavior-preservation proof, assumptions ledger,
  failure modes, and a concrete compiled artifact rather than a named math idea.
- `alien-artifact-coding/references/15-PROOF-OBLIGATIONS-LIBRARY.md`:
  rewrite-soundness obligation for the wavelet counter against the Fenwick
  reference.

### EV score and relevance score

EV model from the applied graveyard skill:

`EV = Impact * Confidence * Reuse / (Effort * AdoptionFriction)`

- Impact: 3.0. It attacks the residual integer inversion wall without changing
  pandas-visible behavior. Expected upside is lower branch/random-access cost,
  not a guaranteed asymptotic drop.
- Confidence: 2.5. The integer proof is straightforward; the constants are
  uncertain because merge-sort already lost to the u32 Fenwick.
- Reuse: 2.0. The helper is reusable for no-tie Kendall pair cells and tests,
  but not broadly useful outside rank-domain inversion counts.
- Effort: 3.0. One helper plus matrix-path switch and focused tests.
- Adoption friction: 1.0. It lives behind the existing complete/no-tie gate and
  can fall back to Fenwick.

`EV = 3.0 * 2.5 * 2.0 / (3.0 * 1.0) = 5.0`

Extreme-optimization score:

`Impact * Confidence / Effort = 3.0 * 2.5 / 3.0 = 2.5`

Relevance:

- Symptom fit: 4.0 / 5. Directly targets residual Kendall rank inversion.
- Architecture fit: 4.0 / 5. One `fp-frame/src/lib.rs` lever, no API change.
- Project fit: 4.0 / 5. DataFrame correlation matrix remains columnar and
  safe Rust.
- Proof readiness: 5.0 / 5. Fenwick is an exact reference and goldens exist.
- Operability: 4.0 / 5. Deterministic fallback and rollback are simple.

Weighted relevance:

`0.30*4.0 + 0.25*4.0 + 0.20*4.0 + 0.15*5.0 + 0.10*4.0 = 4.15 / 5`

Priority tier: A for a bounded Pass 3 experiment. Not S, because the constants
may miss the benchmark gate and the current Fenwick path is already correct.

### Chosen primitive and rejected alternatives

Chosen:

- **Budgeted MSB wavelet/rank-partition inversion counter.**
  - Exact for arbitrary no-tie rank permutations.
  - Safe Rust: `Vec<u32>` gather buffer, `Vec<u32>` scratch buffer,
    `u64` discordance accumulator.
  - Uses only stable integer partition and `u32::BITS - leading_zeros` bit
    selection; no unsafe, SIMD, third-party crates, or floating-point changes.
  - The old Fenwick remains the baseline comparator and fallback.

Rejected for Pass 3:

- **Row-major multi-Fenwick batching:** explicitly forbidden and already
  regressed in `.88`.
- **Morsel-size scheduling:** explicitly forbidden and measured neutral/noisy.
- **Prevalidation removal/hoist:** explicitly forbidden and rejected by
  `0dm7c`.
- **Merge-sort inversion counting:** explicitly forbidden and measured slower.
- **Sqrt/block counters:** explicitly forbidden and analytically worse than the
  L3-resident u32 Fenwick on this workload.
- **Per-worker buffer reuse / cache layout reshuffles:** explicitly forbidden;
  they do not reduce rank-comparison work.
- **Typed Float64 extraction:** already kept in `.89`.
- **Full row-pair bitset/signature matrix:** rejected because exact row-pair
  materialization is `O(cols * rows^2)` and previously regressed hard.
- **General persistent wavelet tree per column pair:** more code and memory than
  the selected one-shot wavelet partition; no extra proof value for a single
  total inversion count.
- **Full CDQ/offline dominance across all right columns:** plausible but too
  much implementation surface for the next one-lever pass, and it risks
  reintroducing merge-sort-like copy volume under a different name.

### Budgeted mode and fallback trigger

Runtime budget for the candidate path:

- Only enter from `complete_kendall_no_tie_parallel_matrix` after the existing
  complete finite no-tie gates have already produced `order_refs` and
  `rank_refs`.
- Use the wavelet counter only when:
  - `row_count >= 4096`;
  - `row_count <= (1 << 22)`;
  - `max_rank < row_count`;
  - scratch allocation for `perm` + `tmp` succeeds;
  - bit width is at most `usize::BITS - 1` and the inversion accumulator stays
    in `u64`.
- On any shape/budget/allocation failure, call the existing
  `Series::kendall_no_tie_fast_with_ordered_ranks` Fenwick path.

Benchmark fallback trigger:

- Reject and remove the source change if paired/reversed hyperfine does not show
  at least `1.15x` on both `df_kendall 50000 1` and `df_kendall 200000 1`, or
  if either golden hash changes.

### Isomorphism proof plan

- Order: unchanged. The matrix still consumes the same `left_order` and
  `right_rank_by_row` witnesses produced by `kendall_no_tie_order` and
  `kendall_rank_by_row_from_order`.
- Diagonal: unchanged. `complete_kendall_no_tie_parallel_matrix` still writes
  exact `1.0` on `mat[i * n + i]`.
- Symmetry: unchanged. The matrix still computes upper-triangle cells and
  mirrors the same `f64` value into the transpose.
- Tie + NaN + null fallback: unchanged. The candidate is reachable only after
  the existing complete/finite/no-tie gates; rejected columns still use the
  current pairwise fallback path.
- Fallback: unchanged observable behavior. Any candidate rejection calls the
  current Fenwick helper, not a new approximate path.
- f64 bits: unchanged for accepted inputs. The wavelet counter must produce the
  same `u64` discordance count as Fenwick, then reuse the existing
  `(n_pairs - 2.0 * discordant) / n_pairs` formula.
- Focused unit proof: compare the wavelet discordance count to Fenwick on
  hand-written permutations, reverse/sorted permutations, and the existing
  `complete_kendall_parallel_matrix_matches_serial_ordered_ranks` fixture.
- Golden proof: regenerate and compare `df_kendall 2000`, `5000`, and `20000`
  against `acf366c2`, `031978ba`, and `f164fa86`.

### Before/after target for p50-ish hyperfine means

Baseline:

- `df_kendall 50000 1`: mean `135.2 ms`, median `132.2 ms`.
- `df_kendall 200000 1`: mean `594.7 ms`, median `595.6 ms`.

Pass 3 target:

- `df_kendall 50000 1`: mean at or below `117 ms`.
- `df_kendall 200000 1`: mean at or below `517 ms`.

Minimum keep gate:

- at least `1.15x` paired/reversed speedup at both shapes;
- unchanged goldens;
- focused Kendall tests pass;
- no new clippy/check failures in the touched crate scope.

### Primary failure risk and countermeasure

Risk: constants kill the primitive. The current u32 Fenwick is L3-resident, and
the prior merge-sort attempt proved that sequential full-array passes can lose
despite nicer locality.

Countermeasure:

- Keep the old Fenwick helper as the reference and fallback.
- Gate by shape and scratch budget.
- Treat the wavelet counter as a single removable lever.
- Reject on the paired/reversed benchmark gate rather than stacking any second
  change on top of it.

### Repro artifacts and rollback

Existing baseline/repro artifacts:

- `tests/artifacts/perf/uza0490_base_build_perf_profile.txt`
- `tests/artifacts/perf/uza0490_base_golden_df_kendall_2000.txt`
- `tests/artifacts/perf/uza0490_base_golden_df_kendall_5000.txt`
- `tests/artifacts/perf/uza0490_base_golden_df_kendall_20000.txt`
- `tests/artifacts/perf/uza0490_base_golden_sha256.txt`
- `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_50000x1.{txt,json}`
- `tests/artifacts/perf/uza0490_base_hyperfine_df_kendall_200000x1.{txt,json}`
- `tests/artifacts/perf/uza0490_base_perf_record_df_kendall_200000x1.{stdout,stderr}`
- `tests/artifacts/perf/uza0490_base_perf_stat_df_kendall_200000x1.{stdout,stderr}`

Pass 3 should add:

- `uza0490_after_golden_*`
- `uza0490_after_hyperfine_*`
- `uza0490_pair_forward_hyperfine_*`
- `uza0490_pair_reversed_hyperfine_*`
- focused Kendall test output
- crate-scoped check/clippy/fmt/UBS outputs as feasible

Rollback:

- Since Pass 3 must be one source lever in `crates/fp-frame/src/lib.rs`, remove
  the wavelet helper and the matrix-path switch if the gate fails.
- Do not touch `.89` typed Float64 extraction or unrelated files.

## Pass 3 target summary

Implement `count_ordered_rank_inversions_wavelet` (name flexible) in
`crates/fp-frame/src/lib.rs` and route the complete/no-tie Kendall matrix path
through it under the budget gate above, with the current Fenwick helper retained
as exact fallback and test oracle.
