# br-frankenpandas-uza04.143 nullable Utf8 range join lane

Agent: LavenderStone
Target: `str_left_join` / `str_outer_join` output assembly
Decision: keep

## Profile-backed target

`br-frankenpandas-uza04.143` follows the rejected `.140` shared Utf8
optional-plan lever and the rejected `.142` typed Int64 slice route. The exact
string join target still spent time in repeated output materialization, so this
pass attacked the nullable Utf8 payload assembly itself.

The baseline release-perf example was built before this lever:

- `rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Artifact: `tests/artifacts/perf/lavender_uza04142_base_build_perf_profile.txt`

`perf record` was attempted for the baseline target, but the machine blocked
hardware profiling with `perf_event_paranoid=4`:

- Artifact: `tests/artifacts/perf/lavender_uza04142_base_perf_record_str_left_join_100000x5.txt`

## Lever

One implementation lever:

- Add `ScalarValues::LazyNullableUtf8Range`, an O(1) descriptor for
  `[null prefix] + source[start..start+len] + [null suffix]`.
- Add `Column::reindex_eager_utf8_with_nullable_range` and
  `Column::reindex_eager_utf8_with_shared_plan` for all-valid eager Utf8
  sources.
- In ordered-unique left/outer join output builders, compute one shared Utf8
  gather plan for nullable payload lanes and reuse it across matching output
  columns.

This replaces repeated `Option<usize>` materialization for the common sorted
unique overlap shape. Non-Utf8, nullable-source Utf8, all-present, duplicate,
and non-contiguous plans fall back to the previous paths.

## Isomorphism proof

- Row order is still produced by the existing `left_positions` and
  `right_positions` vectors. The matching, sort, and tie-breaking code was not
  changed.
- Column naming, suffix resolution, key coalescing, and output column ordering
  remain in the existing join builders.
- The new range representation is equivalent to the old position tape
  `[usize::MAX; null_prefix] + source_start..source_start+source_len +
  [usize::MAX; null_suffix]`.
- Missing slots still materialize as `Scalar::Null(NullKind::Null)`, identical
  to `reindex_by_positions` and `reindex_outer_join_column` for nullable join
  output.
- Numeric promotion and non-Utf8 dtype behavior stay on the old
  `reindex_outer_join_column` path.
- No floating-point arithmetic or RNG path is touched.

## Golden sha256

All candidate golden outputs match the baseline exactly.

| Scenario | Baseline | Candidate |
| --- | --- | --- |
| `str_left_join 5000` | `995b9ca366dacc4d808be3f5b99d027fd0b7d8f6adffd2ff9f91a43e92628006` | `995b9ca366dacc4d808be3f5b99d027fd0b7d8f6adffd2ff9f91a43e92628006` |
| `str_outer_join 5000` | `72546ef6396c90dd2a9a33b8da723079a5db22d449a7607ded73287b9818741c` | `72546ef6396c90dd2a9a33b8da723079a5db22d449a7607ded73287b9818741c` |
| `str_left_join 100000` | `05f9bd5ebc68650a5d606f96648f7df183027801acd0c8f180b10a345d75217a` | `05f9bd5ebc68650a5d606f96648f7df183027801acd0c8f180b10a345d75217a` |
| `str_outer_join 100000` | `86ddec7b290d4fdf124d30eee671c24ecd7cb1de3734ec3a8d339b44ef8c6883` | `86ddec7b290d4fdf124d30eee671c24ecd7cb1de3734ec3a8d339b44ef8c6883` |

Artifacts:

- `tests/artifacts/perf/lavender_uza04142_base_golden_str_left_join_5000.sha256`
- `tests/artifacts/perf/lavender_uza04142_range_candidate_golden_str_left_join_5000.sha256`
- `tests/artifacts/perf/lavender_uza04142_base_golden_str_outer_join_5000.sha256`
- `tests/artifacts/perf/lavender_uza04142_range_candidate_golden_str_outer_join_5000.sha256`
- `tests/artifacts/perf/lavender_uza04142_base_golden_str_left_join_100000.sha256`
- `tests/artifacts/perf/lavender_uza04142_range_candidate_golden_str_left_join_100000.sha256`
- `tests/artifacts/perf/lavender_uza04142_base_golden_str_outer_join_100000.sha256`
- `tests/artifacts/perf/lavender_uza04142_range_candidate_golden_str_outer_join_100000.sha256`

## Benchmark gate

Forward paired hyperfine:

- `str_left_join 100000x5`: `181.4 ms +/- 7.7` ->
  `176.8 ms +/- 7.2` (`1.03x +/- 0.06`)
- `str_outer_join 100000x5`: `228.2 ms +/- 15.2` ->
  `219.5 ms +/- 8.4` (`1.04x` wall, user CPU `219.3 ms` -> `152.9 ms`)

Reversed paired hyperfine:

- `str_left_join 100000x5`: `189.3 ms +/- 10.8` ->
  `179.7 ms +/- 8.5` (`1.05x +/- 0.08`)
- `str_outer_join 100000x5`: `222.7 ms +/- 7.3` ->
  `210.9 ms +/- 9.4` (`1.06x` wall, user CPU `220.1 ms` -> `147.7 ms`)

Artifacts:

- `tests/artifacts/perf/lavender_uza04142_range_pair_hyperfine_str_join_100000x5.txt`
- `tests/artifacts/perf/lavender_uza04142_range_pair_hyperfine_str_join_100000x5.json`
- `tests/artifacts/perf/lavender_uza04142_range_pair_reversed_hyperfine_str_join_100000x5.txt`
- `tests/artifacts/perf/lavender_uza04142_range_pair_reversed_hyperfine_str_join_100000x5.json`

## Validation

- `rch exec -- cargo check -p fp-columnar -p fp-join --lib`
- `rch exec -- cargo clippy -p fp-columnar -p fp-join --lib -- -D warnings`
- `rch exec -- cargo test -p fp-join left_join --lib`
- `rch exec -- cargo test -p fp-join outer_join --lib`
- `cargo fmt --check --package fp-columnar --package fp-join`
- `git diff --check -- crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs tests/artifacts/perf/lavender_uza04142*`
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs`

Validation artifacts:

- `tests/artifacts/perf/lavender_uza04142_candidate_range_check3_fp_columnar_join.txt`
- `tests/artifacts/perf/lavender_uza04142_candidate_range_clippy_fp_columnar_join.txt`
- `tests/artifacts/perf/lavender_uza04142_candidate_range_test_fp_join_left_join.txt`
- `tests/artifacts/perf/lavender_uza04142_candidate_range_test_fp_join_outer_join.txt`
- `tests/artifacts/perf/lavender_uza04142_range_ubs_touched_rust.txt`

## Score

Impact 2.5 * Confidence 4 / Effort 2 = 5.0.

The lever clears the `Score >= 2.0` keep gate. It is modest on wall-clock but
consistent in both command orders and materially lowers user CPU on the outer
join target, while preserving byte-for-byte golden output.
