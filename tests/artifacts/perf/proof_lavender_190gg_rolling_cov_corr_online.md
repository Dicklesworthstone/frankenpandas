# br-frankenpandas-190gg rolling cov/corr online proof

Agent: LavenderStone
Date: 2026-06-15
Lever: replace rolling pairwise cov/corr per-window refold with one sliding
safe-Rust pairwise raw-moment state.

## Profile-backed target

`br-frankenpandas-190gg` identified `Rolling::cov(other)` and
`Rolling::corr(other)` as O(n*w) paths. The old implementation aligned `other`
once, then rebuilt a `Vec<(f64, f64)>` for every output window and performed
two-pass centered statistics.

## Baseline

Build:
`tests/artifacts/perf/lavender_190gg_base_build_bench_rolling_cov_corr.txt`

Benchmark:
`tests/artifacts/perf/lavender_190gg_base_bench_rolling_cov_corr.txt`

```text
n=100000 window=100
rolling cov  OLD    14.622 ms -> NEW   119.288 ms = 0.12x
rolling corr OLD    15.071 ms -> NEW   128.172 ms = 0.12x
```

In this harness, `OLD` is an inline direct-array refold reference. The public
API baseline is the `NEW` column.

## Candidate

Implementation summary:

- Materialize aligned pairwise numeric values once as `Option<(f64, f64)>`.
- Slide one `RollingPairwiseMomentState` over `window_bounds` with monotonic
  add/remove pointers.
- Track raw sums for x, y, x*x, y*y, and x*y.
- Track exact distinct x/y values so constant-window covariance stays exactly
  `0.0` and constant-window correlation stays missing.
- Clamp correlations within `1e-12` of +/-1 to the exact sign to preserve the
  existing perfect-correlation display goldens and guard raw-sum rounding.

Candidate build:
`tests/artifacts/perf/lavender_190gg_candidate_build_bench_rolling_cov_corr_after_clamp.txt`

Candidate benchmark:
`tests/artifacts/perf/lavender_190gg_candidate_bench_rolling_cov_corr_after_clamp.txt`

```text
n=100000 window=100
rolling cov  OLD    14.879 ms -> NEW    26.003 ms = 0.57x
rolling corr OLD    15.044 ms -> NEW    25.252 ms = 0.60x
```

Public API delta: rolling cov `119.288 ms -> 26.003 ms` (`4.59x`), rolling
corr `128.172 ms -> 25.252 ms` (`5.08x`).

Score: Impact 4 * Confidence 5 / Effort 2 = 10.0. Kept.

## Isomorphism

- Alignment: `other` is still left-aligned to `self.series` before pair
  extraction.
- Missing/null/NaN: the same `Series::pairwise_numeric_value` gate is used once
  per aligned row; any invalid side drops the pair.
- Ordering: output order, source index labels, index name, and series name are
  preserved.
- Tie-breaking/RNG: no tie-breaker or random behavior is involved.
- Floating point: formula changes from repeated centered two-pass folds to raw
  sliding sums. Focused tests compare against the naive centered reference with
  tight tolerance; exact perfect-correlation goldens are preserved through the
  +/-1 clamp.
- Constant windows: exact distinct ledgers preserve covariance `0.0` and
  correlation missing output for constant x/y windows.

## Validation

- `rch exec -- cargo test -p fp-frame rolling_cov_corr -- --nocapture` passed:
  `tests/artifacts/perf/lavender_190gg_test_rolling_cov_corr_after_clamp.txt`.
- `rch exec -- cargo test -p fp-frame rolling_dataframe -- --nocapture` passed:
  `tests/artifacts/perf/lavender_190gg_test_rolling_dataframe_goldens_after_clamp.txt`.
- `rch exec -- cargo check -p fp-frame --all-targets` passed:
  `tests/artifacts/perf/lavender_190gg_cargo_check_fp_frame.txt`.
- `rch exec -- cargo clippy -p fp-frame --all-targets -- -D warnings` passed:
  `tests/artifacts/perf/lavender_190gg_clippy_fp_frame.txt`.
- `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs
  crates/fp-frame/examples/bench_rolling_cov_corr.rs` passed:
  `tests/artifacts/perf/lavender_190gg_candidate_rustfmt_check.txt`.
- `ubs --only=rust crates/fp-frame/src/lib.rs
  crates/fp-frame/examples/bench_rolling_cov_corr.rs` timed out after 180s on
  the large touched source file and recorded `UBS_EXIT=124`:
  `tests/artifacts/perf/lavender_190gg_ubs_touched.txt`.

## SHA256

See `tests/artifacts/perf/lavender_190gg_artifact_sha256.txt`.
