# br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1.1 - contiguous UTF8 join output keep

Agent: OrangePeak
Date: 2026-06-09
Target crate: fp-join

## Profile-backed target

Parent commit `b32eaa3718800f3ea72b5697737f95f8c27f5ef9` was re-profiled with:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc11111111-current rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
perf_profile str_inner_join 1000000 1000000
```

RCH failed open locally. The setup-diluted profile still exposed a join-only residual:

```text
_int_malloc: 10.59%
__memcmp_avx2_movbe: 9.27%
fp_join::build_single_key_inner_merge_output_with_selections: 8.76%
fp_join::utf8_span_lower_bound: 7.40%
insert_merged_output_column: 3.42%
column_name_lookup_contains: 2.58%
take_contiguous_range: 1.19%
```

Artifacts:

- `tests/artifacts/perf/perf_record_current_reprofile_join_only_jbyuc11111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/perf_report_current_reprofile_join_only_jbyuc11111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/perf_current_reprofile_join_only_jbyuc11111111_str_inner_join_1000000x1000000.data`

## Lever

One production lever was kept: specialize `build_single_key_inner_merge_output_with_selections` for the ordered UTF8 inner-join case where both selections are `PositionSelection::ContiguousRange`, the join key name is identical on both sides, the contiguous ranges have equal length, and there is no overlapping non-key column name.

This avoids the generic suffix/hash/name-resolution path for the narrow no-overlap case and builds the output columns directly from contiguous slices.

## Isomorphism proof

Ordering and tie-breaking:

- Only precomputed paired contiguous selections are consumed.
- Left columns are emitted in original left `column_order`.
- The shared right key is skipped exactly as the generic same-name key path does.
- Right non-key columns are emitted in original right `column_order`.
- Output index remains `Index::new_known_unique_int64_unit_range(0, len)`, matching the generic builder.

Suffix and duplicate-name behavior:

- If any right non-key name overlaps a left non-key name, the fast path returns `None`.
- Those cases continue through the existing generic suffix/error path.
- `merge_column_name_conflict*` tests passed after the change.

Floating point, null/NaN, and RNG:

- This lever does not inspect, compare, reorder, or recompute scalar values.
- It delegates column slicing to `take_position_selection_typed`, the same typed selection helper already used for contiguous selections.
- No random state or hash order is observable in this fast path; `BTreeMap` is still the storage map and `column_order` controls observable column order.

Golden output:

```text
76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e  tests/artifacts/perf/golden_base_jbyuc111111111_str_inner_join_1000000.txt
76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e  tests/artifacts/perf/golden_after_jbyuc111111111_str_inner_join_1000000.txt
```

Artifacts:

- `tests/artifacts/perf/golden_base_jbyuc111111111.sha256`
- `tests/artifacts/perf/golden_base_jbyuc111111111.verify.txt`
- `tests/artifacts/perf/golden_base_jbyuc111111111_str_inner_join_1000000.txt`
- `tests/artifacts/perf/golden_after_jbyuc111111111.sha256`
- `tests/artifacts/perf/golden_after_jbyuc111111111.verify.txt`
- `tests/artifacts/perf/golden_after_jbyuc111111111_str_inner_join_1000000.txt`
- `tests/artifacts/perf/golden_compare_jbyuc111111111.sha256`

## Benchmark

Same-command hyperfine pair:

```text
Before b32eaa37:
  perf_profile str_inner_join 1000000 1000000
  1.113 s +/- 0.044 s [User: 1.036 s, System: 0.075 s]

After candidate:
  perf_profile str_inner_join 1000000 1000000
  954.6 ms +/- 13.1 ms [User: 877.7 ms, System: 75.8 ms]

Speedup:
  1.17x +/- 0.05
```

Artifacts:

- `tests/artifacts/perf/hyperfine_base_jbyuc111111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/hyperfine_base_jbyuc111111111_str_inner_join_1000000x1000000.json`
- `tests/artifacts/perf/hyperfine_pair_jbyuc111111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/hyperfine_pair_jbyuc111111111_str_inner_join_1000000x1000000.json`

## Re-profile

After profile:

```text
_int_malloc: 13.93%
fp_join::utf8_span_lower_bound: 9.11%
__memcmp_avx2_movbe: 8.02%
fp_join::build_single_key_inner_merge_output_with_selections: 5.19%
fp_join::merge_dataframes_on_with_options: 4.50%
fp_join::merge_single_key_inner_unsorted: 4.34%
perf_profile::build_str_join_frame: 3.48%
BTreeMap<String, Column>::insert: 2.62%
```

The optimized output-builder symbol dropped from 8.76% to 5.19%; the next profile-backed route is output materialization allocation/BTreeMap insertion or the UTF8 lower-bound/certificate path.

Artifacts:

- `tests/artifacts/perf/perf_record_after_jbyuc111111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/perf_report_after_jbyuc111111111_str_inner_join_1000000x1000000.txt`
- `tests/artifacts/perf/perf_after_jbyuc111111111_str_inner_join_1000000x1000000.data`

## Validation

Passed:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111-test rch exec -- cargo test -p fp-join ordered_utf8_contiguous_no_overlap_output_fast_path_jbyuc111111111 --lib -- --nocapture
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111-test rch exec -- cargo test -p fp-join ordered_unique_utf8 --lib -- --nocapture
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111-test rch exec -- cargo test -p fp-join merge_column_name_conflict --lib -- --nocapture
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111-test rch exec -- cargo fmt -p fp-join -- --check
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111-test rch exec -- cargo check -p fp-join --all-targets
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111-test rch exec -- cargo clippy -p fp-join --all-targets -- -D warnings
```

UBS:

```text
ubs crates/fp-join/src/lib.rs
```

UBS exited 1 on file-wide pre-existing inventory. The final artifact reports no `panic!` macros. Remaining critical entries are four equality-comparison false positives on dtype/sentinel comparisons outside this lever.

Artifact:

- `tests/artifacts/perf/ubs_jbyuc111111111.txt`

## Score

Impact 3.5 x Confidence 4 / Effort 2 = 7.0. Keep.
