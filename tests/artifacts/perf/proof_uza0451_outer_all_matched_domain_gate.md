# br-frankenpandas-uza04.51 proof: outer all-matched domain gate

## Target

- Scenario: `perf_profile outer_join 500000 20`
- Baseline binary: `/data/projects/.scratch/cargo-target-orangepeak-uza0451-base/release-perf/examples/perf_profile`
- Candidate binary: `/data/projects/.scratch/cargo-target-orangepeak-uza0451-after/release-perf/examples/perf_profile`
- Profile-backed target: the current-main profile showed `build_single_key_dense_i64_outer_all_matched_merge_output` at `5.67%` self before falling through to the real partial-overlap outer builder.

## Lever

For the dense Int64 outer all-matched fast path, compute each side's min/max key domain before building column specs or dense bucket vectors. If one side is empty, or if `left_min != right_min || left_max != right_max`, return `None` immediately and let the existing general dense outer builder handle the partial-overlap case.

This is a route gate, not an output rewrite.

## Isomorphism and golden proof

- Ordering: unchanged. Rejected all-matched candidates still route to the existing general dense outer builder, which owns bucket-order output for partial overlaps.
- Tie-breaking: unchanged. The general dense outer builder still emits left-major cross products for matched buckets and existing side-only rows for unmatched buckets.
- Floating point: unchanged. No value conversion path changed.
- RNG: not used by the scenario.
- Empty case: unchanged; both-empty input still reaches the existing empty-output branch.
- All-matched case: unchanged when both sides share the same min/max domain; existing all-matched tests still cover the valid fast path.
- Golden command: `perf_profile golden outer_join 20000`
- Baseline SHA256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- Candidate SHA256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- `cmp -s` baseline-vs-candidate golden output: pass

The min/max proof is one-way: if the domains differ, at least one edge bucket has rows from only one side, so the all-matched fast path cannot be semantically valid. Returning `None` is therefore equivalent to the prior eventual rejection, but avoids the rejected path's bucket-vector work.

## Timing evidence

Baseline-only hyperfine:

- `1.203 s +/- 0.043`

Direct internal timers:

- Baseline: `75.974 ms/iter`
- Candidate: `55.213 ms/iter`

Paired hyperfine:

- Baseline: `1.184 s +/- 0.030`
- Candidate: `1.132 s +/- 0.028`
- Ratio: candidate `1.05x +/- 0.04` faster

Perf profiles:

- Baseline event count: `5,197,550,566`; `build_single_key_dense_i64_outer_all_matched_merge_output` was `5.67%` self.
- Candidate event count: `4,492,033,735`; the all-matched outer helper is no longer a top no-children frame.
- Shifted residual: `build_single_key_dense_i64_outer_merge_output` and nullable repeated-slice construction remain the real outer-join materialization target.

## Score

- Impact: 2
- Confidence: 4
- Effort: 1
- Score: `2 * 4 / 1 = 8.0`

Decision: keep.

## Validation

- `rustfmt --check crates/fp-join/src/lib.rs`
- `cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route`
- `cargo test -p fp-join merge_outer_all_matched_dense_int64_duplicates_matches_generic_validated_route`
- `cargo test -p fp-join`
- `cargo check -p fp-join --all-targets`
- `cargo clippy -p fp-join --all-targets -- -D warnings`
- `ubs crates/fp-join/src/lib.rs` completed with pre-existing file-wide findings; no sampled finding targets the new min/max gate.
- `cargo fmt --check` failed on unrelated existing formatting drift in non-touched files; the touched file-specific rustfmt check passed.
