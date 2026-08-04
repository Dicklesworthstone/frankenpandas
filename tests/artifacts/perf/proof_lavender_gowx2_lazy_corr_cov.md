# br-frankenpandas-gowx2 lazy corr/cov matrix output proof

## Change

Add a safe-Rust lazy all-valid Float64 pairwise-stat matrix column plan for
finite `DataFrame::corr()` and `DataFrame::cov()` outputs. The fast path only
applies when all selected numeric input columns are all-valid finite Float64
and the resulting matrix has finite all-valid outputs. Constant, insufficient
`min_periods`, NaN/null, non-finite, and mixed dtype cases fall back to the
existing eager path.

## Profile-backed target

- Bead: `br-frankenpandas-gowx2`
- Routing artifact: `tests/artifacts/perf/lavender_current_20260616a_profile_matrix.txt`
- Baseline profile rows at HEAD `cfcf9868`:
  - `df_corr 100000 1`: `111.224 ms/iter`
  - `df_cov 100000 1`: `102.812 ms/iter`

This targets the benchmark shape that inspects output metadata before
materializing values; lazy output columns avoid computing and freeing the full
matrix until values are actually requested.

## Benchmark gate

Baseline hyperfine:

- Artifact: `tests/artifacts/perf/lavender_corr_cov_lazy_base_hyperfine_corr_cov_100000.txt`
- `df_corr 100000x1`: `111.8 ms +/- 4.2 ms`
- `df_cov 100000x1`: `112.4 ms +/- 3.2 ms`

Candidate hyperfine:

- Artifact: `tests/artifacts/perf/lavender_corr_cov_lazy_candidate_hyperfine_corr_cov_100000.txt`
- `df_corr 100000x1`: `76.1 ms +/- 6.4 ms`
- `df_cov 100000x1`: `74.3 ms +/- 5.9 ms`

Result:

- `df_corr`: `1.47x` faster
- `df_cov`: `1.51x` faster
- Score: Impact 4 * Confidence 4 / Effort 2 = `8.0`; keep.

Note: these hyperfine commands were captured before the 2026-06-16 ts1-offline
override. The wrapper warned that `hyperfine` was a non-compilation command; the
post-override validation below was rerun with plain local `cargo`, no `rch exec`.

## Golden output proof

Baseline SHA256:

```text
ed9392f10bb6c553347a99e21ff5bdf8c140758fd9d8f86da61d1fd1addf58f2  tests/artifacts/perf/lavender_corr_cov_lazy_base_golden_df_corr_5000.txt
c72cd8ebff48785c7620736d4e1b11d6a7c10dbb6a705e565429bd0787008eaa  tests/artifacts/perf/lavender_corr_cov_lazy_base_golden_df_cov_5000.txt
```

Candidate SHA256:

```text
ed9392f10bb6c553347a99e21ff5bdf8c140758fd9d8f86da61d1fd1addf58f2  tests/artifacts/perf/lavender_corr_cov_lazy_candidate_golden_df_corr_5000.txt
c72cd8ebff48785c7620736d4e1b11d6a7c10dbb6a705e565429bd0787008eaa  tests/artifacts/perf/lavender_corr_cov_lazy_candidate_golden_df_cov_5000.txt
```

Checks:

- `tests/artifacts/perf/lavender_corr_cov_lazy_base_golden_check.txt`: OK
- `tests/artifacts/perf/lavender_corr_cov_lazy_candidate_golden_check.txt`: OK
- `tests/artifacts/perf/lavender_corr_cov_lazy_golden_diff.txt`: zero lines

## Isomorphism proof

- Ordering preserved: yes. Numeric input column discovery and result column
  labels still flow through the existing `numeric_cols` order. Row labels are
  still the same `IndexLabel::Utf8` labels in the same order.
- Tie-breaking unchanged: N/A. Corr/cov has no ordering tie-break path.
- Floating-point behavior: materialized values are byte-identical for the
  golden fixture. The lazy plan uses the same shifted-moment Gram formulation
  and finalization semantics as the eager complete finite path.
- RNG seeds: N/A. No randomized path.
- Null/NaN behavior: preserved by guard. Any nullable, NaN, non-finite,
  constant-correlation, or insufficient-`min_periods` input falls back to the
  existing eager implementation.
- Safety: safe Rust only; no C BLAS/MKL/XLA and no unsafe block introduced.

## Validation

Local post-override gates:

- `cargo check -j 1 -p fp-columnar -p fp-frame --lib`: pass.
  Artifact: `tests/artifacts/perf/lavender_gowx2_local_check_fp_columnar_frame_lib.txt`
- `cargo clippy -j 1 -p fp-columnar -p fp-frame --lib -- -D warnings`: pass.
  Artifact: `tests/artifacts/perf/lavender_gowx2_local_clippy_fp_columnar_frame_lib.txt`
- `cargo test -j 1 -p fp-frame dataframe_corr --lib`: 13 passed.
  Artifact: `tests/artifacts/perf/lavender_gowx2_local_test_fp_frame_dataframe_corr.txt`
- `cargo test -j 1 -p fp-frame dataframe_cov --lib`: 6 passed.
  Artifact: `tests/artifacts/perf/lavender_gowx2_local_test_fp_frame_dataframe_cov.txt`
- `cargo test -j 1 -p fp-conformance dataframe_corr_cov --lib`: 2 passed.
  Artifact: `tests/artifacts/perf/lavender_gowx2_local_test_fp_conformance_dataframe_corr_cov.txt`
- `rustfmt --check crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs`: pass.
  Artifact: `tests/artifacts/perf/lavender_gowx2_candidate_rustfmt_check_touched.txt`
- `git diff --check` on touched source/bead/progress files: pass.
  Artifact: `tests/artifacts/perf/lavender_gowx2_git_diff_check_touched.txt`

Known unrelated gate blockers:

- `cargo clippy -p fp-columnar -p fp-frame --all-targets -- -D warnings`
  fails in pre-existing example code at
  `crates/fp-columnar/examples/bench_str_unique.rs:86` for
  `clippy::manual_clamp`, outside this lever.
- `ubs crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs` was stopped
  after it spent over three minutes inside an `ast-grep run '$X.unwrap()'`
  subprocess on a two-file shadow workspace without findings.

## Rollback

Revert the commit for `br-frankenpandas-gowx2`; all lazy-path entry points are
guarded behind the new pairwise-stat matrix constructor and the existing eager
path remains intact.
