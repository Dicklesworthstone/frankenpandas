# br-frankenpandas-jbyuc.1.1.1.1.1.1.1.1.1.1.1.1 direct wrapper proof

## Change

`merge_dataframes(left, right, on, JoinType::Inner)` now enters the existing
single-key inner fast path directly, instead of routing through
`merge_dataframes_on` and rebuilding the same default execution options first.

## Baseline and profile

- Baseline commit: `8fffd7d7`.
- Baseline build:
  `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111111-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Candidate build:
  `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111111-candidate rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH failed open locally for both builds; both stayed crate-scoped and used
  separate scratch target dirs.
- Baseline artifact:
  `tests/artifacts/perf/perf_report_base_jbyuc111111111111_str_inner_join_1000000x1000000.txt`

## Isomorphism proof

- Ordering preserved: yes. Row matching and output assembly still happen in
  `merge_single_key_inner_unsorted`.
- Tie-breaking unchanged: yes. Duplicate-key multiplication and left/right
  match iteration are unchanged.
- Floating-point unchanged: yes. The change only bypasses wrapper setup; value
  materialization is unchanged.
- Null/NaN unchanged: yes. Missing-key classification remains inside the shared
  join path.
- RNG unchanged: N/A. The join path is deterministic and uses no RNG.
- Error class/text preserved for missing public key columns:
  `merge_missing_key_column_errors` passed.
- Golden output SHA:
  `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`
- Golden comparison:
  `tests/artifacts/perf/golden_compare_jbyuc111111111111_direct_wrapper.txt`
  says `golden_cmp=identical`.

## Focused behavior gates

- `cargo test -p fp-join merge_missing_key_column_errors --lib -- --nocapture`
- `cargo test -p fp-join merge_column_name_conflict --lib -- --nocapture`
- `cargo test -p fp-join merge_null_nan_keys_metamorphic_unmatched_rows_do_not_disturb_inner_tn6qb8 --lib -- --nocapture`
- `cargo test -p fp-join merge_default_suffixes --lib -- --nocapture`
- `cargo test -p fp-join merge_duplicate_keys --lib -- --nocapture`
- `cargo test -p fp-join merge_identical_duplicate_keys_cross_join_values_jdupk --lib -- --nocapture`
- `cargo test -p fp-join merge_inner_wide_sparse_int64_hash_matches_generic_validated_route --lib -- --nocapture`

All focused gates passed under
`CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-jbyuc111111111111-candidate rch exec -- ...`.

## Benchmark gate

Command:
`perf_profile str_inner_join 1000000 1000000`

- Baseline-first pair:
  - Before: `640.7 ms +/- 10.4 ms`
  - After: `570.9 ms +/- 5.6 ms`
  - Ratio: `1.12x +/- 0.02`
- Reversed pair:
  - After: `605.6 ms +/- 13.2 ms`
  - Before: `865.9 ms +/- 161.6 ms`
  - Ratio: `1.43x +/- 0.27`

Score: Impact 2 x Confidence 4 / Effort 2 = 4.0. Keep.

## Validation

- `cargo fmt -p fp-join -- --check`
- `cargo check -p fp-join --all-targets`
- `cargo clippy -p fp-join --all-targets -- -D warnings`
- `ubs crates/fp-join/src/lib.rs`

Formatting, check, and clippy passed. UBS exited 1 on pre-existing file-wide
inventory/false positives; no finding targeted the new `merge_dataframes`
wrapper hunk.

## Shifted profile

After profile artifact:
`tests/artifacts/perf/perf_report_after_jbyuc111111111111_direct_wrapper_str_inner_join_1000000x1000000.txt`

Visible residuals shifted to sort-order `OnceLock` initialization, frame
construction, free/memmove traffic, and `Column::take_contiguous_range`.
The next route should attack setup/certificate reuse or contiguous output
materialization, not another public-wrapper shortcut.
