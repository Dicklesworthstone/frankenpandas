# br-frankenpandas-xt3dd proof: multi-key Int64 dense groupby aggregation

## Target

- Bead: `br-frankenpandas-xt3dd`
- Scenario: `groupby_agg_multi_int2 100000 20`
- Local-only override: `RCH_REQUIRE_REMOTE=0`; no `rch exec` was used.
- Profile-backed hotspot: post-`gowx2` local matrix had `groupby_agg_multi
  100000 20` at `0.583 ms/iter` for string keys, while the new multi-key Int64
  focused baseline exposed the redundant `build_groups` path at
  `9.881 ms/iter` internally and `186.4 ms +/- 12.6 ms` under standalone
  hyperfine.

## Change

Add a direct mixed-radix dense grouping witness for all-valid multi-column
Int64 `DataFrameGroupBy::agg_list` Float64 moment reductions. The new helper
derives:

- `gid_per_row`
- `ngroups`
- output `order`
- per-gid key tuples for flat labels and row `MultiIndex`

The existing `build_groups` path remains the fallback for non-Int64 keys,
empty inputs, nullable keys, high-span products, `as_index=false`, and non-moment
aggregation paths.

## Opportunity score

- Impact: 5
- Confidence: 5
- Effort: 2
- Score: `5 * 5 / 2 = 12.5`

## Isomorphism proof

- Ordering preserved: yes. Group ids are assigned in row first-seen order,
  matching the existing dense `build_groups` mixed-radix branch. For
  `sort=true`, output gids are sorted by `Vec<i64>` lexicographic order, which
  is identical to `composite_key_cmp` for all-Int64 key tuples.
- Tie-breaking unchanged: yes. Duplicate key tuples share the first assigned
  gid exactly as before; no randomized or hash-iteration ordering is introduced.
- Floating-point preserved: yes. The change reuses the existing
  `moments_by_pair` accumulator, so row scan order, per-group sum/count pass,
  variance/std second pass, ddof behavior, and singleton `NaN` output are
  unchanged.
- RNG preserved: N/A.
- Labels preserved: yes. Flat labels are the same comma-joined Int64 strings as
  `group_key_label`; row `MultiIndex` levels and names are emitted from the
  same key tuple values as `group_keys_as_row_multiindex`.
- Fallback behavior preserved: yes. Any case outside the all-valid bounded-span
  multi-Int64 typed Float64 moments gate falls back to the previous path.

## Golden output

- Baseline golden: `tests/artifacts/perf/lavender_xt3dd_base_golden_groupby_agg_multi_int2_5000.txt`
- Candidate golden: `tests/artifacts/perf/lavender_xt3dd_candidate_golden_groupby_agg_multi_int2_5000.txt`
- SHA256: `a4a683548faa65c8dacfa36caa966b8141dec65629d16ed86a0c8ce0478ce7ee`
- Diff artifact: `tests/artifacts/perf/lavender_xt3dd_golden_diff.txt`
- Diff lines: `0`

## Benchmarks

Baseline binary:
`/data/projects/.scratch/lavender_xt3dd_bins/perf_profile_base_78952a1`

Candidate binary:
`/data/projects/.scratch/cargo-target-lavender-post-gowx2-local/release-perf/examples/perf_profile`

Forward paired hyperfine:

- Baseline: `238.4 ms +/- 24.3 ms`
- Candidate: `75.7 ms +/- 4.3 ms`
- Speedup: `3.15x +/- 0.37`
- Artifact: `tests/artifacts/perf/lavender_xt3dd_pair_hyperfine_groupby_agg_multi_int2_100000x20.txt`

Reversed paired hyperfine:

- Candidate: `72.8 ms +/- 8.7 ms`
- Baseline: `191.2 ms +/- 14.3 ms`
- Speedup: `2.63x +/- 0.37`
- Artifact: `tests/artifacts/perf/lavender_xt3dd_pair_reversed_hyperfine_groupby_agg_multi_int2_100000x20.txt`

## Validation

- `cargo check -j 1 -p fp-frame --lib`
- `cargo clippy -j 1 -p fp-frame --lib -- -D warnings`
- `cargo check -j 1 -p fp-conformance --example perf_profile`
- `cargo test -j 1 -p fp-frame dataframe_groupby_agg_list_multi_int64_keeps_multiindex --lib`
- `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs crates/fp-conformance/examples/perf_profile.rs`
- `git diff --check`

Workspace `cargo fmt --check` remains blocked by unrelated formatting drift in
`crates/fp-bench/src/main.rs` and `crates/fp-index/examples/*`; touched-file
rustfmt passed. UBS on the touched Rust files timed out after 180s with exit
`124`, matching prior large-file UBS behavior.
