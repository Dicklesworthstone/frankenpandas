# br-frankenpandas-g2veb online rolling/expanding moments proof

Agent: LavenderStone
Date: 2026-06-15
Lever: replace rolling/expanding skew and kurtosis refold paths with one
safe-Rust online raw-moment state.

Merge note: origin/main independently landed a smaller fused-scan g2veb lever
while this pass was validating. This proof keeps the online evidence under
`lavender_g2veb_online_*` names and leaves the fused evidence artifacts at
their original paths.

## Profile-backed target

`br-frankenpandas-g2veb` identified `Rolling::skew`, `Rolling::kurt`,
`Expanding::skew`, and `Expanding::kurt` as O(n*w)/O(n^2) moment paths. The
old implementation refolded each complete rolling window or expanding prefix
through `apply_rolling` / `apply_expanding`.

## Baseline

Build:
`tests/artifacts/perf/lavender_g2veb_online_base_build_bench_window_moments.txt`

Benchmark:
`tests/artifacts/perf/lavender_g2veb_online_base_bench_window_moments.txt`

Baseline output:

```text
rolling_n=100000 expanding_n=8000 window=50
rolling skew  OLD    10.125 ms -> NEW    13.481 ms = 0.75x
rolling kurt  OLD     9.011 ms -> NEW     9.734 ms = 0.93x
expanding skew OLD    68.041 ms -> NEW    68.730 ms = 0.99x
expanding kurt OLD    67.856 ms -> NEW    69.257 ms = 0.98x
```

At baseline, the "NEW" column still called the historical implementation.

## Candidate

Implementation summary:

- Added `RollingMomentState` with count, raw sums through order four, and an
  exact distinct-value ledger for constant-window semantics.
- `Rolling::skew` and `Rolling::kurt` now slide one state over window bounds.
- `Expanding::skew` and `Expanding::kurt` now append into one state per column.
- Missing values remain skipped through the existing `Scalar::is_missing`
  gate before numeric conversion.

Candidate build:
`tests/artifacts/perf/lavender_g2veb_online_candidate_build_bench_window_moments.txt`

Candidate benchmark:
`tests/artifacts/perf/lavender_g2veb_online_candidate_bench_window_moments.txt`

Candidate output:

```text
rolling_n=100000 expanding_n=8000 window=50
rolling skew  OLD    10.032 ms -> NEW     9.316 ms = 1.08x
rolling kurt  OLD     8.926 ms -> NEW     6.451 ms = 1.38x
expanding skew OLD    70.597 ms -> NEW     0.667 ms = 105.91x
expanding kurt OLD    73.673 ms -> NEW     0.599 ms = 122.96x
```

Score: Impact 5 * Confidence 5 / Effort 2 = 12.5. Kept.

## Isomorphism

- Ordering: row order, column order, Series/DataFrame names, and index labels
  are preserved by constructing output from the same input positions.
- Tie-breaking: no ordering or grouping tie-breaker is introduced.
- Floating point: accumulation changes from repeated refold to sliding/
  append-only raw sums. The formulas for skew/kurtosis are unchanged. Text
  goldens for rolling/expanding skew were updated only for last-digit display
  reassociation; pandas 2.2.3 was used to verify the fixture shape.
- Null/NaN: existing missing handling is preserved through `Scalar::is_missing`;
  valid non-missing values still pass through `to_f64()`.
- RNG: not applicable.
- Constant windows/prefixes: exact distinct-value accounting preserves the
  established zero-skew and negative-three-kurtosis outputs for constant data.

## Validation

- `rch exec -- cargo test -p fp-frame skew -- --nocapture` passed:
  `tests/artifacts/perf/lavender_g2veb_test_skew_filter_after_golden.txt`.
- `rch exec -- cargo test -p fp-frame kurt -- --nocapture` passed:
  `tests/artifacts/perf/lavender_g2veb_test_kurt_filter.txt`.
- `rch exec -- cargo check -p fp-frame --all-targets` passed:
  `tests/artifacts/perf/lavender_g2veb_online_cargo_check_fp_frame.txt`.
- `rch exec -- cargo clippy -p fp-frame --all-targets -- -D warnings` passed:
  `tests/artifacts/perf/lavender_g2veb_clippy_fp_frame.txt`.
- Post-merge `rch exec -- cargo check -p fp-frame --all-targets` passed:
  `tests/artifacts/perf/lavender_g2veb_merge_cargo_check_fp_frame.txt`.
- Post-merge `rch exec -- cargo clippy -p fp-frame --all-targets --
  -D warnings` passed:
  `tests/artifacts/perf/lavender_g2veb_merge_clippy_fp_frame.txt`.
- `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs
  crates/fp-frame/examples/bench_window_moments.rs` passed:
  `tests/artifacts/perf/lavender_g2veb_rustfmt_check_final.txt`.
- The same rustfmt check passed after resolving the concurrent origin/main
  g2veb merge:
  `tests/artifacts/perf/lavender_g2veb_merge_rustfmt_check.txt`.
- `ubs --only=rust crates/fp-frame/src/lib.rs
  crates/fp-frame/examples/bench_window_moments.rs` timed out after 180s on
  the large touched source file and recorded `UBS_EXIT=124`:
  `tests/artifacts/perf/lavender_g2veb_online_ubs_touched.txt`.

## Golden SHA256

See `tests/artifacts/perf/lavender_g2veb_artifact_sha256.txt`.
