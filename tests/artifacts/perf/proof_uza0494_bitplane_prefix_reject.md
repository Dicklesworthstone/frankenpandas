# br-frankenpandas-uza04.94 - bitplane Kendall witness rejection

Status: rejected, no runtime source retained.

## Target

After `br-frankenpandas-uza04.93` rejected a fixed-left CDQ target-panel row
builder, this pass tested a cache-oblivious / succinct bitplane-style exact
Kendall witness:

- For one left column order, scan all target `rank_by_row` streams by descending
  rank bit.
- For each bit, group rows by the already-equal higher-bit prefix and count
  prior `1` bits when the current rank bit is `0`.
- Each row pair is counted once, at the highest differing rank bit, preserving
  exact discordance counts and the final Kendall formula.

The lever was deliberately bounded: use it as the complete no-tie matrix route
only if it beats the current word-blocked ordered-rank inversion counter.

## Baseline

Baseline binary:

`/data/projects/.scratch/cargo-target-lavenderstone-uza0494-base/release-perf/examples/perf_profile`

RCH build:

`CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza0494-base RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`

RCH worker: `vmi1153651`.

Baseline-only hyperfine:

- `df_kendall 50000x1`: `116.5 ms +/- 5.3 ms`
- `df_kendall 200000x1`: `488.7 ms +/- 18.2 ms`

Artifacts:

- `tests/artifacts/perf/uza0494_baseline_df_kendall_50000.json`
- `tests/artifacts/perf/uza0494_baseline_df_kendall_200000.json`

## Behavior Proof

Focused RCH tests passed before rejecting and removing the candidate source:

- `cargo test -p fp-frame --lib kendall_bitplane_prefix_counter_matches_word_blocks -- --nocapture`
- `cargo test -p fp-frame --lib complete_kendall_parallel_matrix_matches_serial_ordered_ranks -- --nocapture`

RCH worker: `vmi1153651`.

Golden-output SHA256 matched baseline and candidate exactly:

- `df_kendall 2000`: `acf366c266b66f8497fb55734ed1b4ec40952a7c014f43db262c7c1e625e15e1`
- `df_kendall 5000`: `031978ba431260b942dd36d9be055064a7453118a7c00888c35747644d33d99e`
- `df_kendall 20000`: `f164fa86300fbea93e46aa99a5e0f7413fa8c0baf2ee49c6a689ed92652a4c3b`

Isomorphism boundary:

- Input ordering preserved: same complete-finite/no-tie gate feeds identical
  per-column orders and rank-by-row arrays.
- Tie/NaN behavior preserved: candidate was only considered after the existing
  no-tie complete matrix prevalidation; all other cases fall back unchanged.
- Floating-point behavior preserved: discordant pair counts are exact integers;
  final formula remains `(n_pairs - 2 * discordant) / n_pairs` with the same
  diagonal `1.0`, symmetric copy, and `NaN` zero-pair guard.
- RNG behavior unchanged: no randomness introduced.
- Output ordering preserved: same column-major matrix indices and symmetry copy.

## Bench Gate

After binary:

`/data/projects/.scratch/cargo-target-lavenderstone-uza0494-after/release-perf/examples/perf_profile`

RCH build:

`CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavenderstone-uza0494-after RUSTFLAGS="-C force-frame-pointers=yes" rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`

RCH worker: `vmi1153651`.

Paired hyperfine results:

- `50000x1` forward: baseline `113.8 ms +/- 3.3 ms`, candidate `247.8 ms +/- 4.7 ms`; baseline `2.18x` faster.
- `50000x1` reversed: candidate `249.2 ms +/- 5.3 ms`, baseline `118.4 ms +/- 5.7 ms`; baseline `2.10x` faster.
- `200000x1` forward: baseline `472.2 ms +/- 8.5 ms`, candidate `1.761 s +/- 0.157 s`; baseline `3.73x` faster.
- `200000x1` reversed: candidate `1.786 s +/- 0.158 s`, baseline `479.8 ms +/- 13.0 ms`; baseline `3.72x` faster.

Artifacts:

- `tests/artifacts/perf/uza0494_pair_df_kendall_50000_forward.json`
- `tests/artifacts/perf/uza0494_pair_df_kendall_50000_reversed.json`
- `tests/artifacts/perf/uza0494_pair_df_kendall_200000_forward.json`
- `tests/artifacts/perf/uza0494_pair_df_kendall_200000_reversed.json`

## Decision

Reject. Score is below zero for the campaign gate because the candidate is a
large regression at both row counts and both benchmark orders.

The bitplane prefix witness is exact, but the per-left-column bit passes and
target-prefix scratch traffic multiply the existing compact word-block counter
work instead of eliminating it. The candidate source and focused temporary test
were removed; `crates/fp-frame/src/lib.rs` has no retained diff.

## Next Route

Do not retry bitplane-prefix scans, CDQ repartition copies, per-pair counter
rewrites, static one-dimensional rank/select, row-major rank-signature batching,
dynamic multi-Fenwick/bitset batching, morsel scheduling, merge-sort,
sqrt/block counters, validation removal, or cache-layout micro-tuning.

Next primitive should move to a fundamentally different exact all-pairs Kendall
formulation: a row-pair sign tensor / divide-and-conquer accumulation that
updates many column-pair discordance counters from packed column-order
signatures, with an early design proof that it avoids the explicit `O(n^2)`
row-pair surface while preserving exact counts.
