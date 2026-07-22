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

## 2026-06-18/19 - br-frankenpandas-uza04.202 - Generic groupby std/var counters

- Status: implemented, gauntlet-measured against pandas 2.2.3 on 2026-06-19.
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
- Gauntlet guard runs:
  - RCH build: `cargo build --profile release-perf -p fp-groupby --bin groupby-bench`
    on worker `hz2`, exit 0.
  - Local timing build: same command/target dir, exit 0.
  - Focused tests:
    `groupby_var_std_utf8_keys_stream_numeric_counters_uza04202` and
    `groupby_var_std_timedelta_fallback_preserves_dtype_uza04202`, exit 0.
  - RCH Criterion guard:
    `cargo bench -p fp-conformance --bench vs_pandas -- groupby/` on worker
    `vmi1227854`, exit 0.
- UBS run: first bounded scan found one new critical from an explicit `panic!`
  in the added test; the panic was removed. Rerun
  `timeout 180s ubs crates/fp-groupby/src/lib.rs crates/fp-groupby/src/bin/groupby-bench.rs`
  exited 0 with 0 critical findings and the pre-existing broad `fp-groupby`
  warning inventory.
- Golden digests:
  - 100k `agg-var`: `13b32a1dc9da2c47`; 100k `agg-std`: `200223cc1528066e`.
  - 1M `agg-var`: `bed0cd7240248b06`; 1M `agg-std`: `520c114fa5b162b5`.
  - 2M `agg-var`: `efda4ff0d3fd5f69`; 2M `agg-std`: `970818b60f82d0cb`.
- Accepted pandas comparator rows, pinned to CPU 7, dataset construction outside
  timed loops, pandas `groupby("keys", sort=True)["values"].var/std(ddof=1)`:

  | Reducer | Rows | FP p50 | pandas p50 | Ratio vs pandas | FP CV | pandas CV | Verdict |
  |---|---:|---:|---:|---:|---:|---:|---|
  | var | 100k | 2.814 ms | 3.627 ms | 1.289x | 0.52% | 1.36% | KEEP |
  | std | 100k | 2.845 ms | 3.825 ms | 1.344x | 0.96% | 2.56% | KEEP |
  | var | 1M | 29.563 ms | 35.966 ms | 1.217x | 3.05% | 3.26% | KEEP |
  | std | 1M | 28.657 ms | 35.174 ms | 1.227x | 1.05% | 0.44% | KEEP |
  | var | 2M | 58.659 ms | 76.335 ms | 1.301x | 3.49% | 1.80% | KEEP |
  | std | 2M | 56.466 ms | 75.544 ms | 1.338x | 0.64% | 0.61% | KEEP |

  Accepted geomean: 1.285x faster than pandas.
- Dropped diagnostics:
  - 2M `std`, first pinned run: FP p50 58.569 ms, pandas p50 55.941 ms,
    0.955x, dropped because FP CV was 12.39%; superseded by accepted batched
    rerun.
  - 2M `std`, 10-iter rerun: FP p50 56.868 ms, pandas p50 85.258 ms, 1.499x,
    dropped because pandas CV was 5.92%; superseded by accepted 20-iter rerun.
- Benchmark verdict: KEEP. No accepted neutral/slower release row survived the
  CV gate after the 2M batched reruns, and every accepted realistic row is a
  material win versus pandas. No revert.
- Retry predicate: remaining groupby work should target the shared UTF8
  key-factorize/dense-group primitive or output assembly, not another
  per-reducer `Std`/`Var` vector-elimination micro-tweak.

## 2026-06-18/19 - br-frankenpandas-uza04.203 - Generic groupby median numeric vectors

- Status: implemented, gauntlet-measured against pandas 2.2.3 on 2026-06-19.
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
- Golden digest: `groupby-bench --agg agg-median --key-kind utf8 --value-kind float64
  --rows 100000 --key-cardinality 1000 --golden` emitted
  `out_rows=1000 digest=427ec92527f4043d`.
- Benchmark verdict: KEEP. Head-to-head comparator was pandas 2.2.3
  `df.groupby("keys", sort=True)["values"].median()` on the same deterministic
  1000-cardinality UTF8-key Float64/NaN-every-37th workload. Accepted CV-gated
  rows: 100k rows, FP p50 2.101 ms, pandas p50 5.527 ms, ratio **2.631x
  faster**, FP CV 1.48%, pandas CV 4.56%; 2M rows, FP p50 42.975 ms, pandas
  p50 77.171 ms, ratio **1.796x faster**, FP CV 3.21%, pandas CV 1.06%. No
  code revert.
- High-CV diagnostic recorded but not counted as release proof: 1M p50 ratio
  2.504x faster (FP CV 6.62%, pandas CV 1.08%).
- Build / bench guard: RCH `cargo build --profile release-perf -p fp-groupby
  --bin groupby-bench` passed on worker `hz2`; local clean-worktree single-binary
  build passed in the same `CARGO_TARGET_DIR`; focused conformance guard
  `cargo test -p fp-groupby groupby_median_utf8_keys_numeric_vectors_uza04203`
  passed; RCH `cargo bench -p fp-conformance --bench vs_pandas -- groupby/`
  passed on worker `vmi1227854`.
- Retry predicate if this row regresses later: revisit median clone-elision only
  if a same-host, CV-gated rerun falls below parity or profiling moves residual
  time back into the generic median fallback's `Vec<Scalar>` materialization or
  `collect_finite` allocation. Otherwise route future median gaps to shared
  grouped-key plans or per-group selection algorithms, not another wrapper-level
  vector swap.

## 2026-06-18/19 - br-frankenpandas-uza04.204 - Generic groupby nunique borrowed sets

- Status: implemented, gauntlet-measured against pandas 2.2.3 on 2026-06-19.
- Lever: route generic-key `groupby_agg(Nunique)` / `groupby_nunique` over
  per-group borrowed scalar bucket sets instead of cloning every non-missing
  value into `Vec<Scalar>` before calling `nannunique`.
- Baseline comparator: prior generic non-Int64-key nunique path, which hashes
  the same `GroupKeyRef` groups but stores cloned `Scalar` values per group
  before building `nannunique`'s distinct set. Dense Int64 key/value nunique
  keeps its existing direct seen-bitset path and still has precedence.
- Graveyard mapping: Swiss-table-style fast hash state, allocation elimination,
  and cache-aware aggregation state. The helper keeps one compact `FxHashSet`
  per group and stores borrowed scalar bucket keys, so realistic UTF-8-key
  nunique avoids object cloning and a second reducer-local set construction.
- Alien-artifact proof obligation: value buckets mirror `fp_types::nannunique`
  exactly: missing values are skipped; `-0.0` and `+0.0` normalize to one
  float bucket; NaN is missing; Bool, Int64, Float64, Utf8, Timedelta,
  Datetime, Period, and Interval buckets remain dtype-distinct. Output labels
  and ordering are unchanged because the same `GroupKeyRef`, first-source
  index, and `compare_group_labels` sort are used.
- Guard added: `groupby_nunique_utf8_keys_borrowed_sets_uza04204`, covering
  UTF-8 keys, sorted output, first-seen output, duplicate strings, all-missing
  groups, `-0.0`/`+0.0` collapse, Timedelta duplicate buckets, and dtype-distinct
  Bool/Int64/Float64 buckets.
- Bench guard added: `crates/fp-groupby/src/bin/groupby-bench.rs` now accepts
  `--agg agg-nunique` so the batch runner can target the dispatcher nunique
  path against the legacy pandas original and pre-patch `Vec<Scalar>` fallback.
- Cass/ledger preflight: local cass search returned zero hits for the broad
  frankenpandas groupby rejected/slower query; repo ledger search showed prior
  cod-a generic groupby counter keeps and the rejected all-singleton
  string-groupby shortcut. This attempt avoids the shortcut family and targets
  only reducer-local value cloning/set construction under generic keys.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo check -p fp-groupby`.
- Golden digest: `groupby-bench --agg agg-nunique --key-kind utf8 --value-kind float64
  --rows 100000 --key-cardinality 1000 --golden` emitted
  `out_rows=1000 digest=8de75a99b4172941`.
- Benchmark verdict: KEEP. Head-to-head comparator was pandas 2.2.3
  `df.groupby("keys", sort=True)["values"].nunique(dropna=True)` on the same
  deterministic 1000-cardinality UTF8-key Float64/NaN-every-37th workload.
  Accepted CV-gated row: 2M rows, FP p50 53.117 ms, pandas p50 153.747 ms,
  ratio **2.895x faster**, FP CV 2.68%, pandas CV 0.95%. No code revert.
- High-CV diagnostics recorded but not counted as release proof: 100k p50 ratio
  1.762x faster (FP CV 12.91%, pandas CV 13.55%); 1M pinned-CPU rerun p50
  ratio 3.089x faster (FP CV 5.52%, pandas CV 6.35%).
- Build / bench guard: RCH `cargo build --profile release-perf -p fp-groupby
  --bin groupby-bench` passed on worker `vmi1149989`; local clean-worktree
  single-binary build passed in the same `CARGO_TARGET_DIR`; RCH `cargo bench
  -p fp-conformance --bench vs_pandas -- groupby/` passed on worker
  `vmi1227854`; focused conformance guard
  `cargo test -p fp-groupby groupby_nunique_utf8_keys_borrowed_sets_uza04204`
  passed.
- Retry predicate if this row regresses later: revisit only if a same-host,
  CV-gated rerun falls below parity or profiling moves residual time back into
  reducer-local value materialization / `nannunique` set construction. Otherwise
  route future Utf8 groupby gaps to shared key factorization or lower-level
  scalar-bucket primitives, not another wrapper-level nunique clone-elision.

## 2026-06-18 - br-frankenpandas-2qb1i - Generic groupby Float64 sum/prod counters

- Status: implemented, gauntlet-measured against pandas 2.2.3 on 2026-06-19.
- Lever: route generic-key `groupby_agg(Sum|Prod)` over Float64 value columns
  through per-group streaming f64 accumulators instead of cloning every
  non-missing Float64 into `Vec<Scalar>` before calling `nansum`/`nanprod`.
- Baseline comparator: prior generic non-Int64-key Float64 sum/prod path,
  which hashes the same `GroupKeyRef` groups but stores cloned Float64 scalar
  values per group before reducing. The older `br-frankenpandas-uza04.193`
  fast path deliberately accepted only Int64/Bool; this attempt targets the
  explicitly untouched Float64 lane.
- Graveyard mapping: vectorized execution, cache-aware aggregation state, and
  allocation elimination. The helper keeps only two f64 registers plus the
  first-source index per group, removing reducer-local object vectors from a
  realistic numeric aggregation workload.
- Alien-artifact proof obligation: group identity, first-seen order, sorted
  order, and output label reconstruction are unchanged because the same
  `GroupKeyRef`, source index, and `sort_group_ordering_by` comparator are
  used. Numeric values are folded left-to-right in row order with identical
  f64 `+`/`*` identities to `nansum`/`nanprod`; missing values and NaN are
  skipped by `Scalar::is_missing`. Timedelta, string, and mixed-object values
  decline the fast path and retain fallback semantics.
- Guard added: `groupby_agg_sum_prod_float64_utf8_keys_stream_counters_2qb1i`,
  covering UTF-8 keys, sorted output, first-seen output, null/NaN skip,
  all-missing groups, and Float64 sum/prod identities; and
  `groupby_agg_sum_prod_timedelta_fallback_preserved_2qb1i`, covering
  Timedelta sum/prod fallback behavior.
- Bench guard added: `crates/fp-groupby/src/bin/groupby-bench.rs` now accepts
  `--value-kind float64`, so batch can target
  `--agg agg-sum/agg-prod --key-kind utf8 --value-kind float64` against the
  legacy pandas original and pre-patch `Vec<Scalar>` fallback.
- Cass/ledger preflight: local cass search returned zero hits for
  `frankenpandas groupby sum float negative ledger`. Repo ledger search showed
  the prior Int64/Bool `sum/prod` keep and rejected hash/open-address families;
  this attempt avoids hash-table retuning and only removes the still-live
  Float64 value-vector fallback.
- Validation run: passed
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo check -p fp-groupby`
  on 2026-06-18; only pre-existing workspace manifest license/license-file
  warnings were emitted.
- Benchmark verdict: keep. Pandas head-to-head gauntlet found no accepted
  neutral or slower rows for the current groupby cluster. The valid accepted
  rows were 3.155x-6.291x faster than pandas, with a 4.120x accepted geomean
  across the five accepted 100k/1M results. High-CV rows below are recorded as
  non-proof evidence gaps, not keep proof.
- Retry predicate if rejected: do not retry Float64 `sum/prod` scalar-clone
  elimination unless same-worker profiling shows residual self-time in fallback
  value materialization for Float64 aggregation; route remaining gaps to shared
  group-key construction, fused factorize+aggregate, or a lower-level numeric
  grouped-state primitive rather than another reducer wrapper.

### Gauntlet evidence - 2026-06-19 - pandas original comparator

- Artifact report: `artifacts/perf/cod-a-groupby-gauntlet-a7287a4d.md`.
- Raw pandas harness outputs:
  `artifacts/perf/cod-a-groupby-gauntlet-vs-pandas-a7287a4d.json` and
  `artifacts/perf/cod-a-groupby-gauntlet-vs-pandas-a7287a4d-1m.json`.
- Criterion guard output:
  `artifacts/perf/cod-a-groupby-gauntlet-criterion-a7287a4d.txt`.
- Build/profiling note: `rch exec -- cargo build --profile release-perf -p fp-bench`
  succeeded on worker `ovh-b`, but the executable was not materialized by RCH
  artifact sync; the pandas harness used a local per-crate release-perf build
  in the same `CARGO_TARGET_DIR`. `rch exec -- cargo bench -p fp-conformance
  --bench vs_pandas -- groupby/` completed on `ovh-b`.
- Revert decision: no revert. There were no accepted slower or parity rows.
  High-CV rows require a more stable rerun before they can prove a claim, but
  they do not justify reverting this optimization cluster.

| Size | Workload | Verdict | Accepted ratio | p50-implied ratio | FP p50 us | pandas p50 us | FP CV% | pandas CV% | Ledger action |
|---|---|---:|---:|---:|---:|---:|---:|---:|---|
| 100k | `groupby_sum_int64` | DROPPED_HIGH_CV | N/A | 8.095x | 199.10 | 1611.79 | 35.81 | 20.60 | Needs stable rerun; do not cite as keep proof |
| 100k | `groupby_mean_float64` | DROPPED_HIGH_CV | N/A | 3.721x | 264.13 | 982.78 | 2.60 | 13.06 | Needs stable rerun; do not cite as keep proof |
| 100k | `groupby_agg_multi` | DROPPED_HIGH_CV | N/A | 7.949x | 304.53 | 2420.78 | 33.79 | 19.10 | Needs stable rerun; do not cite as keep proof |
| 100k | `groupby_mean_str` | DROPPED_HIGH_CV | N/A | 2.008x | 1860.89 | 3736.72 | 8.69 | 4.91 | Needs stable rerun; do not cite as keep proof |
| 100k | `groupby_transform_mean` | FASTER | 4.586x | 4.586x | 234.31 | 1074.52 | 4.09 | 4.98 | Accepted win |
| 100k | `groupby_transform_mean_str` | DROPPED_HIGH_CV | N/A | 2.018x | 1760.68 | 3553.61 | 13.09 | 11.40 | Needs stable rerun; do not cite as keep proof |
| 100k | `groupby_cumcount` | DROPPED_HIGH_CV | N/A | 1.952x | 793.48 | 1548.59 | 0.97 | 13.71 | Needs stable rerun; do not cite as keep proof |
| 100k | `groupby_count` | FASTER | 3.155x | 3.155x | 197.16 | 622.01 | 2.06 | 2.22 | Accepted win |
| 1M | `groupby_sum_int64` | DROPPED_HIGH_CV | N/A | 4.752x | 2612.67 | 12415.94 | 15.47 | 1.41 | Needs stable rerun; do not cite as keep proof |
| 1M | `groupby_mean_float64` | DROPPED_HIGH_CV | N/A | 4.866x | 2748.41 | 13373.26 | 20.40 | 1.14 | Needs stable rerun; do not cite as keep proof |
| 1M | `groupby_agg_multi` | FASTER | 6.291x | 6.291x | 3212.90 | 20211.93 | 1.34 | 1.01 | Accepted win |
| 1M | `groupby_mean_str` | DROPPED_HIGH_CV | N/A | 2.291x | 18609.44 | 42642.00 | 15.93 | 7.76 | Needs stable rerun; do not cite as keep proof |
| 1M | `groupby_transform_mean` | DROPPED_HIGH_CV | N/A | 6.140x | 2889.73 | 17742.52 | 5.08 | 13.40 | Needs stable rerun; do not cite as keep proof |
| 1M | `groupby_transform_mean_str` | FASTER | 2.178x | 2.178x | 20061.76 | 43688.75 | 1.31 | 1.34 | Accepted win |
| 1M | `groupby_cumcount` | DROPPED_HIGH_CV | N/A | 4.957x | 10804.23 | 53559.56 | 7.91 | 9.48 | Needs stable rerun; do not cite as keep proof |
| 1M | `groupby_count` | FASTER | 5.988x | 5.988x | 1993.46 | 11937.48 | 2.61 | 2.13 | Accepted win |

## 2026-07-22 - br-frankenpandas-zjx9o - DataFrame cum* one-morsel spawn threshold

- Status: **REJECT / NO-SHIP**. The candidate source and ignored timing guard
  were removed; no production change remains.
- Frontier/profile evidence: the mandated
  `python3 benches/vs_pandas_harness.py --all --sizes 10k,100k` run produced
  only four CV-valid cells out of 84. `dataframe_ops/cumsum` at 10k was a
  valid loss: FP p50 439.28 us, mean 437.11 us, CV 3.94%; pandas p50 374.11
  us, mean 373.62 us, CV 1.07%; ratio 0.852x. At 100k FP p50 was 980.14 us
  versus pandas 3628.02 us, but FP CV 8.56% made that row inadmissible.
  `strace -f -c` then attributed the small-frame fixed cost to exactly 280
  `clone3` calls across three warmups plus 25 measurements: ten scoped OS
  threads were created for every 10k x 10 cumulative operation.
- Ledger/log preflight: the Series cumsum owned-buffer and arithmetic
  parallel-threshold families are already closed. This attempt did not retry
  either: it changed only `DataFrame::apply_cum_f64`'s total-cell gate from
  16,384 to 131,072 so a 100k-cell frame stayed serial while the 1M-cell
  100k-row case remained parallel.
- Graveyard mapping and recommendation contract: Vectorized Execution plus
  Morsel-Driven Parallelism (§8.2) says scheduling overhead must be amortized
  by a cache-sized morsel. Opportunity score: impact 3, confidence 5, effort
  1, score 15. The semantic invariant was exact because only independent
  column scheduling changed; column order, prefix-fold order, dtype, null mask,
  and output values were unchanged. The disproof threshold was any candidate
  or null-control CV at or above 5%.
- Strict-remote interleaved same-binary A/B on `ovh-a`, single-operation
  samples: baseline p50 369.996 us; candidate p50 178.025 us (2.0783x);
  null ratio 1.0030. **Rejected** because baseline CV 30.939%, candidate CV
  9.484%, null-A CV 6.292%, and null-B CV 10.300% all exceeded the 5% gate.
- Strict-remote batched interleaved same-binary A/B on `hz1`, 16 operations
  per sample: baseline p50 731.915 us; candidate p50 250.651 us (2.9201x);
  null-A/null-B p50 248.214/246.251 us, null ratio 1.0080. **Rejected** because
  baseline CV 13.643%, candidate CV 13.337%, and null-A CV 11.553% exceeded
  5% (only null-B cleared at 3.136%). Directional medians are not keep proof.
- Retry-condition predicate: retry only when an isolated or pinned worker can
  run the same in-process A/B/null harness with baseline, candidate, null-A,
  and null-B all below 5% CV, while a 10k and 100k targeted public harness
  rerun also clears 5% CV and shows no 100k regression. Until that predicate
  holds, do not ship or cite the apparent 2.08-2.92x median effect.

## 2026-07-22 - groupby/read frontier measurement blocker

- The current `vs_pandas_harness.py` groupby run at 10k/100k produced no
  admissible loss: valid cells were `groupby_sum_int64` 7.41x faster at 100k
  and `groupby_mean_float64` 8.64x faster at 10k; all other groupby cells were
  high-CV. The IO run likewise had a valid `csv_write` 10k win (FP p50
  4420.82 us, pandas 91082.05 us, 20.603x, FP/pandas CV 2.38%/3.89%).
- No fresh groupby or CSV write lever is justified. `csv_read` was an apparent
  FP win (p50 48.21 us vs 8046.00 us at 10k; 1183.69 us vs 102602.87 us at
  100k), but CV was 11.53% and 5.18%, so both rows were dropped. `csv_write`
  at 100k was also dropped (CV 10.49%/13.39%).
- Genuine blocker: all four parquet read/write cells are `INCOMPLETE` because
  the current `fp-bench` reports `unsupported io/parquet_*`; the public
  harness therefore cannot supply a measured parquet frontier. This is not a
  source-performance rejection and no speculative code was changed.
- Retry predicate: resume this vein only after fp-bench has an admitted
  parquet read/write workload (or a fixture-bound equivalent) and a pinned or
  isolated worker yields CV <5% for both sides on 10k and 100k; then profile
  the admitted hotspot and preflight its exact ledger/log family before one
  lever. Until then, route to a different measured surface rather than
  inventing a parquet result.

## 2026-07-22 - df_transpose_materialize SURFACE re-run remains inadmissible

- Rebuilt `fp-bench` at current HEAD `3ccd78f6051a09074fc6f17f5cd2ffa317d10996` with strict remote RCH on `ovh-a`; retrieved binary SHA256 `464eec7d8e9c144ea657e18b3a3df5bf38ec2d9915f72df46ca26de7a911f0b8`, size 40,654,904 bytes, timestamp `2026-07-22T19:22:25Z`.
- Re-ran `df_transpose_materialize` Float64 at 100k. FrankenPandas p50 1357.49 us, CV 0.47%; pandas p50 1573.77 us, CV 7.40%; harness verdict `DROPPED_HIGH_CV` and ratio withheld. **Rejected as evidence**, not as a source lever: the pandas control failed the 5% admissibility gate.
- Retry-condition predicate: retry only with an isolated/pinned worker and same-binary interleaved A/B/null control where both implementations and all null controls are below 5% CV (and the 100k row remains non-regressive).

## 2026-07-22 - groupby/read frontier refresh

- Fresh 10k/100k harness admitted only already-landed groupby wins: `groupby_sum_int64` 100k 5.819x (CV 2.12%/1.56%), `groupby_transform_mean` 10k 9.27x (4.30%/1.70%), `groupby_transform_mean_str` 10k 6.616x (2.78%/3.72%), `groupby_cumcount` 100k 6.943x (3.62%/4.49%), and `groupby_count` 10k 7.158x (1.03%/3.02%). Ledger/log search finds these families already accepted; no new lever is justified.
- IO remains blocked: CSV rows dropped for high CV and parquet read/write remain unsupported by fp-bench. Retry only after a new admitted, profile-backed groupby/read hotspot or parquet workload clears the <5% CV gate at both sizes.

## 2026-07-22 - fresh groupby candidate routing

- Preflight of open candidates found `br-frankenpandas-wvlfh` (RangeIndex groupby bucket construction) explicitly recorded in `docs/NEGATIVE_EVIDENCE.md` as an older code-first item with no profile, while `br-frankenpandas-0rhjk` is cc-assigned. No candidate is eligible for a KEEP without ownership and profile-first hotspot evidence.
- The requested harness rerun therefore confirms the existing blocker rather than authorizing speculative edits: all admissible groupby rows are already-landed wins; CSV is high-CV and parquet is unsupported. Retry predicate: an owned candidate must first show >0.1% self-time in a profile and an admitted 10k/100k A/B/null run with every CV <5%.

## 2026-07-22 - targeted groupby/read admission refresh

- Targeted rerun admitted `groupby_cumcount` 100k at 5.441x (FP/pandas p50 279.91/1523.04 us; CV 2.99%/1.21%) and `csv_write` 10k at 20.874x (4416.93/92198.28 us; CV 2.19%/4.96%). Both are already-landed families, so this is confirmation, not a fresh lever.
- `groupby_mean_str`/`groupby_count` 100k and CSV read 10k/100k remained high-CV; CSV write 100k remained high-CV. Retry requires a new owned profile hotspot plus <5% CV at both sizes.
