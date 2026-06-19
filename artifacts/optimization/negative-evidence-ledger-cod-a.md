# cod-a Negative-Evidence Ledger

Purpose: record every cod-a optimization attempt in the new performance campaign,
including benchmark-pending candidates and dead ends, so future agents do not
retry failed levers without a concrete retry predicate.

## 2026-06-18 - br-frankenpandas-8qn9i - Series.astype Column::astype route

- Status: implemented, benchmark verdict pending batch-test.
- Lever: route `Series::astype(dtype)` through `Column::astype(dtype)` instead
  of `Column::new(dtype, self.values().to_vec())`.
- Baseline comparator: current `Series::astype` materializes every value as a
  `Scalar`, then constructs a target-typed column through constructor coercion.
  This bypasses `Column::astype` typed Int64<->Float64 fast paths and also uses
  strict constructor semantics for finite non-integer Float64->Int64.
- Graveyard mapping: vectorized execution / typed data-plane specialization.
  The canonical graveyard guidance favors profile-backed, one-lever changes and
  data-structure specialization that avoids object materialization; the
  FrankenSuite summary calls out performance claims only after proof contracts
  and conservative fallback. This lever keeps the existing scalar fallback in
  `Column::astype` as the conservative path.
- Alien-artifact proof obligation: `Series::astype` should be a metadata
  wrapper around the column cast table. `Column::astype` already defines the
  pandas cast contract for Int64, Float64, Bool, Utf8, and missing values. The
  Series wrapper preserves series name, index labels, and index name while
  changing only the value dtype conversion engine.
- Guard added: `series_astype_uses_column_cast_table_8qn9i`, covering
  Float64->Int64 truncation toward zero, series name preservation, index label
  and index-name preservation, target dtype, and non-finite Float64->Int64
  rejection.
- Cass/ledger preflight: local `cass status --json` reported a stale lexical
  index; targeted `cass search "Series.astype Column::astype typed paths"`
  returned zero hits. Existing repo ledger search found no prior cod-a
  `Series.astype` attempt. The stale cass state is recorded here rather than
  treated as proof that no prior failed attempt exists.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo check -p fp-frame`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- UBS run: `timeout 180s ubs crates/fp-frame/src/lib.rs` timed out with exit
  124 after entering the Rust scan, matching the known broad `fp-frame` scanner
  backlog/stall behavior documented in
  `artifacts/audits/fp_frame_ubs_inventory_2026-06-17.md`. No completed UBS
  finding was emitted for the touched hunk in that bounded window.
- Benchmark verdict: pending. Required follow-up comparator is a focused
  `Series.astype` Int64->Float64 and Float64->Int64 workload versus the legacy
  pandas original and a pre-patch scalar-materialization baseline, plus the
  existing astype conformance packet.
- Retry predicate if rejected: only revisit this wrapper route if same-host
  profiling shows `Series::astype` above 0.1% self-time and batch evidence
  proves the residual is scalar materialization or constructor coercion rather
  than unavoidable target column allocation.

## 2026-06-18 - br-frankenpandas-9bccl - DataFrame.dropna positional gather

- Status: implemented, benchmark verdict pending batch-test.
- Lever: make row-wise `DataFrame::dropna_with_options` and
  `DataFrame::dropna_with_threshold` collect kept row positions directly and
  call `take_rows_by_positions_unchecked`, bypassing temporary Bool `Series`
  construction, index-label materialization, and `filter_rows` label-alignment.
- Baseline comparator: previous row-wise `dropna` converted each positional
  keep decision into a Bool Series with cloned index labels, then routed through
  `filter_rows`. On duplicate indexes that route is semantically risky because
  `filter_rows` intentionally aligns by label and uses the first matching data
  position for duplicate labels; `dropna` needs row-position semantics.
- Graveyard mapping: certified rewrite pipeline plus SoA/columnar execution.
  The rewrite is accepted only because the equivalence domain is explicit:
  kept positions are exactly the true entries of the old positional mask before
  label alignment. Column data already lives in per-column storage, and
  `take_rows_by_positions_unchecked` is the established typed gather primitive.
- Alien-artifact proof obligation: row order, duplicate labels, index name,
  column order, dtype, missing-value policy (`how=any`, `how=all`, threshold,
  subset validation), and row MultiIndex projection must be preserved. The only
  intended behavior change is replacing accidental duplicate-label mask
  alignment with pandas-compatible positional `dropna` selection.
- Guard added:
  `dataframe_dropna_duplicate_index_uses_positional_rows_9bccl` and
  `dataframe_dropna_threshold_duplicate_index_uses_positional_rows_9bccl`,
  covering duplicate row labels, positional row retention, threshold/subset
  behavior, and index-name preservation.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo check -p fp-frame`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- UBS run: `timeout 180s ubs crates/fp-frame/src/lib.rs` timed out with exit
  124 after entering the Rust scan and emitted no completed finding, matching
  the known broad `fp-frame` scanner backlog/stall behavior documented in
  `artifacts/audits/fp_frame_ubs_inventory_2026-06-17.md`.
- Benchmark verdict: pending. Required follow-up comparator is a focused
  `DataFrame.dropna` row-wise workload with duplicate-index and mostly-valid
  realistic frames versus legacy pandas original and pre-patch mask/filter_rows
  baseline.
- Retry predicate if rejected: only revisit this family if same-host profiling
  shows residual time in row-position collection or `take_rows_by_positions`;
  do not reintroduce a Bool Series or label-aligned `filter_rows` path for
  positional `dropna`.

## 2026-06-18 - br-frankenpandas-uza04.202 - Generic groupby std/var counters

- Status: implemented, benchmark verdict pending batch-test.
- Lever: route generic-key `groupby_agg(Std|Var)` over a clone-free two-pass
  numeric accumulator instead of building per-group `Vec<Scalar>` values and
  then calling `nanvar`/`nanstd`.
- Baseline comparator: dense Int64-key `Std`/`Var` already has a direct bucket
  path, and generic-key `Mean`, `Count`, `Size`, `Min`, `Max`, `First`,
  `Last`, `Sum`, and `Prod` already avoid group value vectors. Generic
  string/object-key `Std`/`Var` was the remaining common numeric reducer still
  paying row hash plus value clone plus per-group Vec allocation before the
  same two-pass finite-number formula.
- Graveyard mapping: loop fusion, cache-aware aggregation state, and
  allocation elimination. The accumulator keeps one compact `(source_idx, sum,
  count, sum_sq)` state per group, so hot realistic UTF-8-key groupby std/var
  no longer round-trips every numeric value through scattered heap vectors.
- Alien-artifact proof obligation: the first pass sums finite non-missing
  numeric values in the same per-group encounter order as the old vector path;
  the second pass accumulates squared deviations in the same order with the
  same ddof=1 boundary. Timedelta and non-numeric values decline the fast path
  so dtype-preserving and mixed-object fallback semantics stay owned by the
  existing implementation.
- Guard added:
  `groupby_var_std_utf8_keys_stream_numeric_counters_uza04202`, covering
  UTF-8 keys, null values, sorted output, first-seen output, singleton groups,
  and all-missing groups; and
  `groupby_var_std_timedelta_fallback_preserves_dtype_uza04202`, covering
  Timedelta var/std fallback to `Timedelta64`/`NaT` outputs.
- Bench guard added: `crates/fp-groupby/src/bin/groupby-bench.rs` now accepts
  `--agg agg-var` and `--agg agg-std` so the batch runner can target the
  dispatcher path against the legacy pandas original and pre-patch Vec
  fallback.
- Cass/ledger preflight: `cass status --json` reported an unhealthy, stale
  index (`last_indexed_at=2026-03-12`); no cass result was trusted. Repo ledger
  search showed prior cod-a keeps for generic groupby first/last, sum/prod,
  and mean/count/min/max-style counters, and prior phantom/rejected string
  groupby/factorization swings; this attempt deliberately targets only the
  remaining generic `Std`/`Var` Vec fallback.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo check -p fp-groupby`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- UBS run: first bounded scan found one new critical from an explicit `panic!`
  in the added test; the panic was removed. Rerun
  `timeout 180s ubs crates/fp-groupby/src/lib.rs crates/fp-groupby/src/bin/groupby-bench.rs`
  exited 0 with 0 critical findings and the pre-existing broad `fp-groupby`
  warning inventory.
- Benchmark verdict: pending. Required follow-up comparator is
  `groupby-bench --agg agg-var` and `--agg agg-std` on realistic UTF-8-key
  cardinalities versus legacy pandas original and a pre-patch per-group
  `Vec<Scalar>` baseline, with golden digest unchanged.
- Retry predicate if rejected: do not retry per-group `Std`/`Var` vector
  elimination unless same-worker profiling shows residual self-time in the
  fallback Vec materialization or the hash/group lookup itself; route any
  remaining gap to a shared grouped-key primitive rather than another reducer
  micro-specialization.

## 2026-06-18 - br-frankenpandas-uza04.203 - Generic groupby median numeric vectors

- Status: implemented, benchmark verdict pending batch-test.
- Lever: route generic-key `groupby_agg(Median)` / `groupby_median` over
  per-group `Vec<f64>` numeric value vectors instead of cloning every
  non-missing value into per-group `Vec<Scalar>` and then allocating a second
  `collect_finite` `Vec<f64>` inside `nanmedian`.
- Baseline comparator: prior generic non-Int64-key median path, which hashes
  the same `GroupKeyRef` but stores cloned `Scalar` values before reducing.
  Dense Int64-key median already has a CSR/select-nth route, so this only
  targets realistic UTF-8/object-key groups that still needed median sorting.
- Graveyard mapping: vectorized execution, cache-aware aggregation state, and
  constants-aware data-structure specialization. The helper keeps the existing
  group hash/order primitive but removes object materialization from the median
  value lane.
- Alien-artifact proof obligation: group admission and output label ordering
  are unchanged because the same `GroupKeyRef`, first-source index, and
  `compare_group_labels` sort are used. Median values are the same finite
  non-missing `to_f64()` values in each group; odd/even selection uses the
  existing dense path's order-statistic rule, with NaN/negative-zero groups
  routed through a full sort fallback. Timedelta and non-numeric values decline
  the fast path, preserving dtype-specific fallback semantics.
- Guard added: `groupby_median_utf8_keys_numeric_vectors_uza04203`, covering
  UTF-8 keys, sorted output, first-seen output, null skipping, odd/even groups,
  singleton groups, and all-missing groups; and
  `groupby_median_timedelta_fallback_preserves_dtype_uza04203`, covering
  Timedelta median fallback to `Timedelta64` output.
- Bench guard added: `crates/fp-groupby/src/bin/groupby-bench.rs` now accepts
  `--agg agg-median` so the batch runner can target the dispatcher median path
  against the legacy pandas original and pre-patch `Vec<Scalar>` fallback.
- Cass/ledger preflight: local cass search returned zero hits for the broad
  frankenpandas rejected/slower query, while repo ledger search showed prior
  groupby counter keeps and a rejected all-singleton string-groupby shortcut.
  This attempt avoids the rejected shortcut family and targets only median
  value materialization under generic keys.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo check -p fp-groupby`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: pending. Required follow-up comparator is
  `groupby-bench --agg agg-median --key-kind utf8` on realistic cardinalities
  versus legacy pandas original and a pre-patch per-group `Vec<Scalar>` median
  baseline, with golden digest unchanged.
- Retry predicate if rejected: do not retry scalar-clone elimination for median
  unless same-host profiling shows residual self-time in the generic median
  fallback's `Vec<Scalar>` materialization or `collect_finite` allocation; route
  remaining median gaps to shared grouped-key plans or per-group selection
  algorithms rather than another wrapper-level vector swap.
