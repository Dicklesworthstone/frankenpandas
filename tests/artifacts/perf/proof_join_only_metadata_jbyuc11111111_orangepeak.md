# br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1 Proof

## Target

Post-cached-parallelism `str_inner_join 1000000 5000` still had process-wide
profiling samples in `perf_profile::build_str_join_frame` and first-use UTF8
certificates, even though the benchmark loop already builds frames outside the
timed loop. A longer `1000000x50000` profile exposed a real repeated library
residual underneath: every tiny ordered-UTF8 inner join rebuilt hash-based column
name metadata before output assembly.

## Lever

In `build_single_key_inner_merge_output_with_selections`:

- cache `DataFrame::column_names()` once per side;
- use direct linear membership scans for schemas with at most eight total
  columns;
- retain the existing hashed lookup behavior for wider schemas.

This removes per-call `HashSet`/`HashMap` allocation and hashing from the narrow
join-only path while preserving the wider-frame lookup strategy.

## Isomorphism

- Ordering and tie-breaking: unchanged. The same position plan, output spec
  order, and insertion order are used.
- Suffix behavior: unchanged. Overlap names are still sorted before
  `ensure_merge_suffixes_for_overlaps`, and duplicate output names still go
  through `insert_merged_output_column`.
- Null/NaN/FP bits: unchanged. Column data and `take_contiguous_range` are not
  modified.
- RNG: not touched.
- Floating-point: not touched.
- Golden output SHA for `perf_profile golden str_inner_join 1000000` is
  `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`
  before and after.

## Benchmarks

Baseline build:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc11111111-before \
  rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

After build:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc11111111-after \
  rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

RCH failed open locally for both builds.

Original bead workload:

- `str_inner_join 1000000 5000`: 142.6 ms +/- 7.4 ms before vs
  138.8 ms +/- 4.3 ms after, 1.03x +/- 0.06.

Longer join-only sampling workload:

- `str_inner_join 1000000 50000`, pair 1: 200.2 ms +/- 7.6 ms before vs
  183.9 ms +/- 5.3 ms after, 1.09x +/- 0.05.
- `str_inner_join 1000000 50000`, pair 2: 200.7 ms +/- 6.6 ms before vs
  181.2 ms +/- 3.9 ms after, 1.11x +/- 0.04.

## Profile Shift

Before `1000000x50000` profile:

- `perf_profile::build_str_join_frame`: 15.74% self.
- UTF8 lower-hex certificate initialization: 8.12% self.
- `fp_join::insert_merged_output_column`: 3.66% self.
- `HashMap<&str, ()>::insert`/hash metadata and `RandomState::hash_one`
  were visible in the repeated join path.

After `1000000x50000` profile:

- `perf_profile::build_str_join_frame`: 24.22% self, confirming the process-wide
  setup artifact still dominates sampling.
- UTF8 lower-hex certificate initialization: 9.95% self.
- `fp_join::build_single_key_inner_merge_output_with_selections`: 2.55% self.
- `Column::take_contiguous_range`: 2.54% self.
- The hash metadata symbols no longer appear as top residuals.

## Validation

- `cargo test -p fp-join ordered_unique_utf8 --lib -- --nocapture`
- `cargo test -p fp-join merge_column_name_conflict --lib -- --nocapture`
- `cargo fmt -p fp-join -- --check`
- `cargo check -p fp-join --all-targets`
- `cargo clippy -p fp-join --all-targets -- -D warnings`

All Cargo commands were run through `rch exec`; RCH failed open locally.

`ubs crates/fp-join/src/lib.rs` exited 1 on pre-existing file-wide findings,
including known false-positive secret-comparison reports for dtype/key equality
and broad legacy unwrap/test inventories. The report does not identify the new
lookup helper or changed output-builder ranges in its sampled findings.

## Score

Impact `2.5` x Confidence `3` / Effort `2` = `3.75`, so this clears the
required `>= 2.0` keep threshold.

## Next Route

The next profile-backed primitive should not tune the removed metadata path.
The shifted evidence points at either a profiler-isolated harness for
setup-free sampling or a deeper `Column::take_contiguous_range`/UTF8 arc-view
primitive for ordered string joins.
