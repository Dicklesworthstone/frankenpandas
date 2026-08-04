# br-frankenpandas-1wc0n proof: expanding cov/corr online bivariate sweep

## Target

`Expanding::cov` and `Expanding::corr` re-folded every prefix with two-pass
means and covariance/variance sums, making the expanding path O(n^2). The
accepted lever in `7082c772` replaces both with one add-only bivariate Welford
sweep that maintains `mean_a`, `mean_b`, `M2a`, `M2b`, and `Cab`.

## Isomorphism

- Row order, index labels, and source axis naming are unchanged.
- Pair membership is unchanged: a pair is admitted only when both values are
  present and numeric.
- Emission gating is unchanged: `min_periods.max(2)`.
- Undefined correlation for zero variance is unchanged: `M2a == 0` or
  `M2b == 0` emits NaN.
- No RNG, tie-breaking, or ordering-sensitive user-visible behavior is added.
- Floating-point summation order changes from per-prefix two-pass to online
  Welford. This surface is tolerance-tested and has no `assert_text_golden`
  fixtures; rolling cov/corr goldens are not touched.

## Benchmark Evidence

Artifact: `tests/artifacts/perf/lavender_1wc0n_bench_expanding_cov.txt`

- `expanding cov`: 108.782 ms -> 0.569 ms, 191.14x faster.
- `expanding corr`: 184.010 ms -> 0.602 ms, 305.85x faster.

Score: Impact 5 x Confidence 5 / Effort 1 = 25.0, accepted.

## Validation

- `rch exec -- cargo build -p fp-frame --profile release-perf --example bench_expanding_cov`
  passed via local fallback.
- `rch exec -- cargo test -p fp-frame expanding_cov_corr_online_matches_naive_reference -- --nocapture`
  passed.
- `rch exec -- cargo test -p fp-frame expanding_cov -- --nocapture` passed
  3 tests including the new isomorphism cross-check.
- `rch exec -- cargo test -p fp-frame expanding_corr -- --nocapture` passed
  remotely on `vmi1227854`.
- `rch exec -- cargo check -p fp-frame --all-targets` passed.
- `rch exec -- cargo clippy -p fp-frame --all-targets -- -D warnings` passed.
- `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs
  crates/fp-frame/examples/bench_expanding_cov.rs` passed.
- `ubs --only=rust crates/fp-frame/src/lib.rs
  crates/fp-frame/examples/bench_expanding_cov.rs` was retried with a 180s
  timeout and timed out with `UBS_EXIT=124`, matching the known
  `br-frankenpandas-yavyk` broad scanner stall.

## Artifact Checksums

Checksums are recorded in
`tests/artifacts/perf/lavender_1wc0n_artifact_sha256.txt`.

Primary benchmark output sha256:
`c999db609e4ebe5ccc18e088a936b8707e72fb34ef02e319b3008f10bc70ae04`.

Primary isomorphism test output sha256:
`4fc91fdaa9cc3af9074a828706f164bde665fdaeaf7e4737bae17f4fc94eb950`.
