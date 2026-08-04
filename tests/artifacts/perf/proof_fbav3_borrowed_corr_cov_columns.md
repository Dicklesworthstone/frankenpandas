# br-frankenpandas-fbav3 - borrowed Float64 corr/cov input columns

Timestamp: 2026-06-13T12:20:20Z
Agent: LavenderStone
Commit base: 1046942f

## Target

`df_corr` / `df_cov` on complete Float64 frames still spent avoidable time
copying every numeric input column into a fresh `Vec<f64>` before entering the
existing pairwise Gram path.

## Lever

Change `pairwise_numeric_column_values` to return `Cow<[f64]>`.

- Typed contiguous Float64 columns are borrowed directly.
- Bool, Int64, Timedelta64, nullable, or otherwise coerced columns still use the
  previous owned `Vec<f64>` materialization path.
- The complete-column moment loops, NaN scan, Gram fold, output ordering, and
  finalize arithmetic are unchanged.

## Isomorphism

- Ordering: `numeric_cols` collection and result column layout are unchanged.
- Tie-breaking: not applicable to Pearson corr/cov.
- Floating point: for typed Float64 columns, the borrowed slice is the exact same
  sequence that the prior `to_vec()` copied; every `sum`, `sum2`, and Gram cell
  still folds the same values in the same loop order.
- Missing/null/NaN: non-Float64 and coerced paths still materialize owned values
  with the same `Timedelta64` / `to_f64().unwrap_or(NaN)` behavior; the complete
  path still requires no `NaN` in any column.
- RNG: not used.

## Golden Verification

Same-host comparison on `vmi1227854`, baseline binary from current
`origin/main` (`1046942f`) vs the rebased candidate:

- `df_corr 5000`: `d7d4c83538939ca2d83ecc13d0f0376d90cd4f9176eaa9009a126e058231cb07`
- `df_cov 5000`: `f3766dadd4b2d36dda67aa9690a0fc3a17708fa8001fcee0c9ccb1895a452e89`
- `cmp`: byte-identical for both outputs.

## Benchmark

Host: `vmi1227854`

Command shape:

```text
hyperfine --warmup 1 --runs 9 \
  "<baseline perf_profile> df_corr 200000 3" \
  "<candidate perf_profile> df_corr 200000 3"

hyperfine --warmup 1 --runs 9 \
  "<baseline perf_profile> df_cov 200000 3" \
  "<candidate perf_profile> df_cov 200000 3"
```

Results:

- `df_corr 200000 x3`: `381.4 ms +/- 40.9` -> `267.6 ms +/- 16.9`, candidate
  `1.43x +/- 0.18` faster.
- `df_cov 200000 x3`: `415.5 ms +/- 34.6` -> `297.3 ms +/- 44.0`, candidate
  `1.40x +/- 0.24` faster.

Score: keep. Impact is high for both corr and cov; confidence is high because
goldens are byte-identical and the timing was same-host paired A/B against the
current-main baseline; effort is small and localized to the extraction boundary.

## Gates

- `cargo check -p fp-frame --all-targets` via RCH on `vmi1227854`: pass.
- `cargo fmt --check -p fp-frame`: pass.
- `dataframe_corr_basic`: pass.
- `dataframe_cov_basic`: pass.
- `corr_golden_basic` filter: 4 tests pass.
- `cov_golden_basic` filter: 3 tests pass.
- `dataframe_cov_includes_bool_and_timedelta_columns`: pass.
- `perf stat`: blocked by `perf_event_paranoid=4`.
- `cargo clippy -p fp-frame --all-targets -- -D warnings`: blocked by existing
  unrelated test-only lints tracked in `br-frankenpandas-scowx`
  (`type_complexity` at `crates/fp-frame/src/lib.rs:83099`, `83290`,
  `83582`, `83798`, `83914`, `84052`; `useless_vec` at `84422`).
- `ubs crates/fp-frame/src/lib.rs`: completed, but exits nonzero on broad
  pre-existing whole-file inventory tracked in `br-frankenpandas-yavyk`
  (`583` critical, `48203` warning, `3947` info); UBS internal
  fmt/clippy/check/test sub-gates were clean.
- Workspace `cargo fmt --check`: blocked by unrelated pre-existing formatter
  drift outside the touched fp-frame code.

## Route Next

Re-profile after this commit. The typed producer/consumer boundary was a real
win; the next primitive should be chosen from the shifted profile rather than
from the rejected FMA/transpose/tile-widen family.
