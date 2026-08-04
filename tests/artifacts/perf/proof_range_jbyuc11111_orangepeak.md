# br-frankenpandas-jbyuc.1.1.1.1.1 proof

Lever: ordered-unique contiguous UTF-8 inner joins now carry a single
`ContiguousRanges` position plan into output construction. The output builder
passes `PositionSelection::ContiguousRange` to `Column::take_contiguous_range`,
so the benchmark-shaped overlap avoids allocating left/right `Vec<usize>` and
avoids rescanning those vectors once per output column.

Baseline build:

- RCH command: `CARGO_TARGET_DIR=/tmp/rch_target_orangepeak_range_before rch exec -j -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH worker: `vmi1227854`
- Retrieved binary: `/tmp/rch_target_orangepeak_range_before/release-perf/examples/perf_profile`

After build:

- RCH command: `CARGO_TARGET_DIR=/tmp/rch_target_orangepeak_range_after rch exec -j -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH worker: `vmi1293453`
- Retrieved binary: `/tmp/rch_target_orangepeak_range_after/release-perf/examples/perf_profile`

Benchmark evidence:

- Baseline hyperfine after the RCH build: `247.245 ms mean +/- 10.851 ms`
- After hyperfine: `159.800 ms mean +/- 5.508 ms`
- Paired A/B hyperfine: `239.945 ms -> 163.490 ms` mean, `1.47x +/- 0.08`
- Direct harness: `0.362 -> 0.201 ms/iter`

Golden proof:

- Before SHA: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`
- After SHA: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`
- `cmp` of SHA files passed.

Isomorphism proof:

- Ordering: `ContiguousRanges { left_start, right_start, len }` expands to the
  same `left_start..left_start + len` and `right_start..right_start + len`
  sequences the old `Vec<usize>` plan emitted.
- Tie-breaking: this path is only used after
  `as_strictly_increasing_utf8_contiguous`; duplicate or unsorted keys fall
  back to the existing hash route.
- Null behavior: the path requires all-valid contiguous UTF-8 keys. Payload
  null semantics are delegated to `Column::take_contiguous_range`, which falls
  back to `take_positions` for nullable/generic columns.
- Floating point: all-valid Float64 payload ranges become
  `LazyAllValidFloat64Slice` views over the same source bytes; `to_bits` is
  unchanged for finite values, `-0.0`, infinities, and any valid payload bits.
- RNG: no RNG or randomized ordering is introduced.

Validation:

- `cargo fmt --check -p fp-columnar -p fp-join`: pass.
- `git diff --check`: pass.
- `cargo test -p fp-columnar take_contiguous_range_uses_typed_views_without_positions -- --nocapture`: pass on RCH worker `vmi1149989`.
- `cargo test -p fp-join ordered_unique_utf8_bulk_window_returns_range_plan_jbyuc11111 -- --nocapture`: pass on RCH worker `vmi1149989`.
- `cargo test -p fp-join ordered_unique_utf8 -- --nocapture`: pass; RCH selected a root worker with topology-preflight syntax error and failed open locally.
- `cargo check -p fp-columnar -p fp-join --all-targets`: pass on RCH worker `vmi1227854`.
- `cargo clippy -p fp-columnar -p fp-join --all-targets -- -D warnings`: pass; RCH twice selected root workers with the same topology-preflight syntax error and failed open locally.
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs`: exit 1 from pre-existing false positives in `fp-join` dtype/key equality checks; UBS build, fmt, clippy, check, and tests were clean.

Post-change profile:

- Artifact: `tests/artifacts/perf/post_range_reprofile_jbyuc11111.txt`
- Top user-space residual: `__memcmp_avx2_movbe` at `17.83%`.
- `Column::take_positions` no longer appears as the top residual in the flat
  report.

Score:

- Impact: 3
- Confidence: 2
- Effort: 1
- Score: `3 * 2 / 1 = 6.0`
