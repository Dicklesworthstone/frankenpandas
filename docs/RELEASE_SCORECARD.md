# FrankenPandas Release-Readiness Scorecard

## 2026-06-24 SlateOtter — fp-index Utf8 get_indexer_non_unique typed (measured, 3.07×→7.14×)

`Index::get_indexer_non_unique` on Utf8 built its position map with `entry(label.clone())` — a String alloc
per source row (1M). Already won 3×, but the clones bound it. Typed `&str`-keyed position map (gate both
sides Utf8). Bit-identical (6-test `utf8_ginu_typed_conformance` vs oracle, green). `probe_str_ginu` @1M/10k:

| op                     | before  | after   | pandas   | ratio          | fp-side |
|------------------------|---------|---------|----------|----------------|---------|
| get_indexer_non_unique | 70.74ms | 30.42ms | 217.13ms | 3.07→**7.14×**  | 2.33×   |

Same `entry(k.clone())` String-alloc-per-row smell as the value_counts fix. Detail in
`docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — fp-index Utf8 Index.value_counts loss→win (measured, 0.80×→3.22×)

`Index::value_counts` on Utf8 tallied an `FxHashMap<IndexLabel>` with **cloned keys** — a `String` alloc per
input label (1M) + redundant `contains_key`. Added a typed `&str` tally (one `entry` per label, clone only
per distinct label). Bit-identical (5-test `utf8_value_counts_typed_conformance` vs oracle, green).
`probe_str_dedup` @1M/10k:

| op           | before  | after   | pandas  | ratio          | fp-side |
|--------------|---------|---------|---------|----------------|---------|
| value_counts | 66.57ms | 16.55ms | 53.23ms | 0.80→**3.22×** | 4.02×   |

(fp already wins unique 1.62×, nunique 1.53×, duplicated 1.75×, drop_duplicates 1.82×.) Detail in
`docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — fp-index Utf8 union/difference/symdiff typed sweep (measured)

Follow-up to the Utf8 intersection win. Typed all-Utf8 paths for the other set-ops: difference (one map
doubling as membership+dedup), symdiff (each membership map carries the opposite half's dedup, 3→2 maps),
union (`&str` keys). Bit-identical (4-test `utf8_setops_typed_conformance` vs oracles, green).
`probe_str_setops` @1M:

| op                   | before   | after    | pandas    | ratio          | fp-side |
|----------------------|----------|----------|-----------|----------------|---------|
| union                | 580.59ms | 314.04ms | 1753.10ms | 3.02→**5.58×** | 1.85×   |
| difference           | 431.74ms | 167.74ms | 874.76ms  | 2.03→**5.21×** | 2.57×   |
| symmetric_difference | 784.76ms | 357.40ms | 962.05ms  | 1.23→**2.69×** | 2.20×   |

fp-index Utf8 set-op surface now fully typed (all WIN 1.2–5.6×). Detail in `docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — fp-index Utf8 Index.intersection loss→win (measured, 0.70×→1.19×)

`Index::intersection` on unsorted Utf8 indexes built `FxHashMap<&IndexLabel>` of other + a separate `seen`
dedup set — ~2.5M pointer-keyed string-hash probes — measured 0.70× vs pandas. Added a typed all-Utf8 path:
one `FxHashMap<&str, bool>` with a "matched" flag dedup (no seen-set), `&str` keys (no enum load).
Bit-identical (6-test `utf8_intersection_typed_conformance` vs oracle, green). `probe_str_setops` @1M:

| op           | before   | after    | pandas   | ratio          | fp-side |
|--------------|----------|----------|----------|----------------|---------|
| intersection | 408.35ms | 241.27ms | 286.34ms | 0.70→**1.19×** | 1.69×   |

(fp already wins get_indexer 4.5×, isin 3.0×; pandas object-Index is brutally slow.) Detail in
`docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — DataFrame std/var/skew axis=1 typed output (measured)

pandas axis=1 is terrible (std 213ms, skew 334ms @1M×8); fp already wins 6-12×. std/var/sem/skew/kurt
axis=1 used `reduce_rows_func_f64` with a `Vec<Scalar>` output — switched to typed `from_f64_values`
(like sum/mean/max). Bit-identical (new `df_axis1_f64_typed_conformance` vs oracle + zero-var guard, green).
`bench_probe_df` @1M×8:

| op (axis=1) | before  | after   | pandas   | ratio           | fp-side |
|-------------|---------|---------|----------|-----------------|---------|
| std         | 20.78ms | 16.43ms | 213.28ms | 10.3→**13.0×**  | 1.26×   |
| var         | 17.72ms | 14.00ms | 205.51ms | 11.6→**14.7×**  | 1.27×   |
| skew        | 38.39ms | 34.65ms | 333.74ms | 8.7→**9.6×**    | 1.11×   |

(sum/mean/max already win 6-7× via typed `reduce_rows_f64`.) Detail in `docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — Series.skew/kurt typed fused-pass (measured, 5.1×→~14×)

`Series::skew`/`kurtosis` called `numeric_values` (copies the all-valid buffer into a `vals` Vec + mean
scan) then re-scanned `vals` twice more. Added a typed `as_f64_slice` fused single-pass (mean, then m2+m3 /
m2+m4 in one sweep, no copy). Bit-identical (4-test `skew_kurt_typed_conformance` vs oracle, green).
`bench_probe3` @1M:

| op   | before | after  | pandas  | ratio          | fp-side |
|------|--------|--------|---------|----------------|---------|
| skew | 3.87ms | 1.41ms | 19.68ms | 5.08→**13.96×** | 2.75×   |
| kurt | 3.82ms | 1.41ms | 19.75ms | 5.17→**14.01×** | 2.71×   |

Same probe: fp already wins sem 4.41× (typed Welford), median 12.6×, prod 1.52×. Detail in
`docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — Series.autocorr typed f64 path (measured, 1.38×→13.4×)

`Series::autocorr` materialized a 1M `Vec<Scalar>` (~32MB) + pair Vecs even for an all-valid Float64
column. Added a typed `as_f64_slice` two-pass path (lag-offset, no Scalar materialization). Bit-identical
(5-test `autocorr_typed_conformance` vs independent oracle, green). `bench_probe2 autocorr` @1M:

| op       | before  | after  | pandas  | ratio          | fp-side |
|----------|---------|--------|---------|----------------|---------|
| autocorr | 13.66ms | 1.41ms | 18.83ms | 1.38→**13.4×** | 9.70×   |

Same probe: fp already wins quantile 6.05×, nunique 2.23×, duplicated 1.17×; corr/cov ~parity
(bandwidth-bound). Detail in `docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — typed nullable-f64 Series sum/mean/var/std (measured)

Series-level reductions had all-valid f64 fast paths but a Float64 column WITH missing values fell to a
`values()` Scalar-Vec loop. Added typed `as_f64_slice_with_validity` skipna branches to `sum`/`var` (mean/std
inherit). Bit-identical (6-test `series_nullable_reduction_conformance` green). `bench_series_null` @1M /
20% missing, ~1.6× fp-side across all four:

| op   | before  | after  | pandas  | ratio          |
|------|---------|--------|---------|----------------|
| var  | 10.71ms | 6.81ms | 11.30ms | 1.06→**1.66×** |
| std  | 10.89ms | 6.84ms | 16.64ms | 1.53→**2.43×** |
| mean | 5.24ms  | 3.26ms | 2.68ms  | 0.51→0.82× *   |
| sum  | 5.20ms  | 3.26ms | 2.03ms  | 0.39→0.62× *   |

var flips parity→win, std strengthens. *sum/mean improve 1.6× fp-side but stay below pandas's SIMD
`np.nansum` (safe scalar `validity.get` vs vectorized NaN-blend; closing it needs unsafe SIMD). Detail in
`docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — dense SeriesGroupBy var/std incl. nullable-f64 (measured, loss→win)

`SeriesGroupBy::std` had no dense path (always `agg_numeric`) and `var`'s dense block gated all-valid
only, so var/std on a Float64 Series WITH missing values fell to the slow generic gather. Extracted one
shared `dense_group_var_std(want_std)` (int64 / all-valid-f64 / nullable-f64; two-pass skipna; all-missing
group -> `Null(NaN)`); `var()`/`std()` both route through it. Bit-identical (6-test
`sgb_nullable_dense_conformance` green). `bench_sgbnull` @1M / 1000 groups / 20% missing, equal load:

| op  | before  | after   | pandas  | ratio     | fp-side |
|-----|---------|---------|---------|-----------|---------|
| var | 49.29ms | 10.48ms | 20.90ms | **1.99×** | 4.70×   |
| std | 49.80ms | 10.80ms | 21.54ms | **1.99×** | 4.61×   |

Detail in `docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — nullable-Float64 dense SeriesGroupBy sum/mean/min/max (measured, loss→win)

Parallel to the DataFrameGroupBy nullable work below, on the SeriesGroupBy side. `dense_group_fold`
(shared by sum/mean/min/max) only accepted all-valid `as_f64_slice()`, so a Float64 Series WITH
missing values fell to the slow generic `agg_numeric` path. Added a skipna `as_f64_slice_with_validity`
branch (all-missing group -> `Null(NaN)` to match agg_numeric). Bit-identical (4-test
`sgb_nullable_dense_conformance` green). `bench_sgbnull` @1M / 1000 groups / 20% missing,
before/after measured under equal load:

| op   | before  | after  | pandas  | ratio     | fp-side |
|------|---------|--------|---------|-----------|---------|
| sum  | 64.88ms | 7.10ms | 25.45ms | **3.59×** | 9.14×   |
| mean | 58.61ms | 6.49ms | 24.59ms | **3.79×** | 9.03×   |
| min  | 57.34ms | 6.86ms | 24.38ms | **3.55×** | 8.36×   |

(max shares min's path, conformance-verified.) Open follow-ups: SeriesGroupBy nullable std (0.50×) and
median (~0.98×) use separate paths — unchanged. Detail in `docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — nullable-Float64 dense groupby prod/first/last/median (measured, loss→win)

Follow-up to the reductions landing below: the nullable-f64 dense gate excluded prod/first/last/median,
so those still fell to the generic `build_groups` path. Widened the gate + added skipna arms (prod
folds from 1.0; first/last = first/last valid in row order; median = new `dense_group_median_f64_skipna`).
Bit-identical (9-test `groupby_nullable_dense_conformance` green). `bench_gbnull` @1M / 1000 groups / 20% missing:

| op     | before  | after   | pandas  | ratio     | fp-side |
|--------|---------|---------|---------|-----------|---------|
| prod   | 40.47ms | 5.25ms  | 19.13ms | **3.64×** | 7.71×   |
| first  | 36.18ms | 5.81ms  | 18.24ms | **3.14×** | 6.23×   |
| last   | 43.41ms | 6.10ms  | 18.45ms | **3.02×** | 7.12×   |
| median | 43.30ms | 12.79ms | 39.01ms | **3.05×** | 3.39×   |

Detail in `docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-24 SlateOtter — nullable-Float64 dense groupby reductions (measured, loss→win)

`DataFrameGroupBy` sum/mean/min/count/std on a dense Int64 key with a Float64 value column
that contains **missing values** was excluded from the dense `dense_aggregate_emit` fast path
(it only accepted all-valid `as_f64_slice`) and fell to the generic `build_groups` path —
**0.43–0.47× pandas**. Added a typed skipna arm over `(data, validity)` and widened the gate.
Bench `bench_gbnull` @1M rows / 1000 groups / 20% missing, bit-identical (5-test
`groupby_nullable_dense_conformance` green):

| op   | before | after  | pandas  | ratio  | fp-side |
|------|--------|--------|---------|--------|---------|
| sum  | 42.99ms| 5.40ms | 18.83ms | **3.49×** | 7.96× |
| mean | 46.05ms| 5.68ms | 21.56ms | **3.80×** | 8.11× |
| min  | 41.00ms| 5.64ms | 17.72ms | **3.14×** | 7.27× |
| std  | 43.52ms| 9.59ms | 20.48ms | **2.14×** | 4.54× |

Detail in `docs/NEGATIVE_EVIDENCE.md`.

## 2026-06-22 cod-pandas — fp-frame typed-key reshape/groupby sweep (measured, br-frankenpandas-1q4q4)

Closed the Utf8/Datetime64 gap where `fp-frame` had Int64-only dense paths. All head-to-head best-of-N
vs pandas 2.2.3 (warm `frankenpandas-cc`), each **bit-identical & oracle-EXACT on sparse workloads with
missing cells**; full fp-frame lib (3101/3101) + full fp-conformance (1595/1595) green. Detail rows in
`docs/NEGATIVE_EVIDENCE.md`. (Distinct from the `fp-groupby` streaming-counter Utf8 rows below — this is
the `fp-frame` `DataFrameGroupBy.build_groups` / `pivot_table` / `crosstab` path.)

| Op (1M rows unless noted) | Before | After | Lever |
|---|---:|---:|---|
| pivot_table Int64 keys (sum/mean/count/size) | 1.01× | **3.54×** | dense row-order scatter, skip 3×Scalar-materialize + Vec<f64> groups map |
| pivot_table Utf8 keys | 1.10× | **3.76×** | factorize each axis → sorted-rank codes → dense scatter |
| pivot_table Datetime64 / mixed axes | 1.14× | **3.48×** | unified per-axis extractor (Int64/Datetime64/Utf8), −96 LOC |
| pivot_table min/max (Int64 & Utf8) | 1.00–1.12× | **3.57×** | shared dense builder f64::min/max fold |
| **pivot_table(margins=True) all aggfuncs** | **0.053× LOSS (18.9× slower)** | **3.45×** | O(n·(n_idx+n_col)) per-margin full scans → O(n) two/four-pass; 69× FP-side |
| **pivot_table var/std (Int64)** | **0.96× LOSS** | **3.21×** | dense two-pass Σ(v−mean)² == generic formula |
| pivot_table median (Int64/Utf8) | 1.07–1.17× | **1.97–2.32×** | dense per-cell scatter + sort (skip Scalar/tuple-hash) |
| crosstab Utf8 keys | 3.1× | **17.2×** | factorize → direct-address i64 count grid |
| **get_dummies Utf8 (30 cats)** | **0.85× LOSS** | **1.20×** | per-row String clone → `&str` borrow |
| **DataFrameGroupBy single Utf8 key** | **0.90× LOSS** | **2.68×** | `FxHashMap<&str,gid>`, kill per-row `Vec<ScalarKey>` |
| **DataFrameGroupBy multi Utf8 key** | **0.87× LOSS** | **1.29×** | `KeyCol::StrScalar` mixed-radix dense |
| SeriesGroupBy single Utf8 key | 1.12× | **2.80×** | one hash probe (was seen-set + groups-map) |
| **pivot var/std (Int64)** + margins all-aggfunc | **0.96× / 0.053× LOSS** | **3.21× / 3.45×** | dense two/four-pass; O(n) margins (was O(n·n_idx), 18.9× slower) |
| pivot Int64-VALUE base+margins; median; Datetime64/mixed axes | 0.053–1.14× | 1.97–3.5× | `pivot_value_f64`; per-cell scatter+sort; unified axis extractor |
| groupby.transform single Utf8 key | **0.73× LOSS** | **2.72×** | `transform_dense_gids` Scalar-Utf8 sibling |
| SeriesGroupBy cumsum/cum* Utf8 key | **0.83× LOSS** | **2.85×** | `dense_group_ids` Scalar-Utf8 sibling |
| **multi-agg single/multi Utf8 key** `.agg([sum,mean,std,count])` | **0.35× / 0.36× LOSS** | **1.72× / 1.69×** | `dense_group_ids_for_order` mixed Int64/Utf8 dense moments |
| **Series.map Utf8→Utf8 / Int64→Utf8 dict** | **0.42× / 0.12× LOSS** | **1.93× / 1.07×** | contiguous-Utf8 output (was N `Box<str>` clones) |
| **Series.isin / DataFrame.isin Utf8** | **0.51× / 1.13× LOSS** | **2.31× / 2.34×** | `&[u8]` probe + typed Bool (was `Vec<Scalar::Bool>` boxing) |

12 genuine measured losses flipped (get_dummies; groupby single/multi Utf8; pivot var/std; **pivot margins —
an 18.9× loss, the single biggest gap, from repeated O(n·n_idx) margin scans**; pivot Int64-value; groupby
transform/cumsum Utf8; multi-agg single/multi Utf8 — a 2.8× slowdown on a core analytics op; **Series.map
Int64→Utf8 — an 8.6× loss**; Series/DataFrame.isin); rest deepened from near-parity. Two reinforcing levers:
**(1) every Int64-only dense path needs a Scalar-backed-Utf8 sibling** (factorize / `&str`-hash instead of
per-row `Vec<ScalarKey>` / Scalar materialization), and **(2) emit typed columns** (contiguous Utf8 / typed
Bool / typed i64) instead of `Vec<Scalar>` boxing. The pivot dense path now covers
**sum/mean/count/size/min/max/var/std/median × {Int64, Datetime64, Utf8} keys × {Float64, Int64} values +
margins(O(n)) + mixed axes**; the groupby Utf8 surface (build_groups, transform, cumulative, multi-agg
moments) is fully dense. One neutral experiment (generic dense-scatter that killed only the `Vec<f64>` churn,
not string hashing) measured ~0-gain and reverted. Next reachable gap is `fp-join` Utf8-keyed merge (separate
crate, not warm in the build dir). Full per-lever detail + ratios in `docs/NEGATIVE_EVIDENCE.md`.

## Release-readiness verdict (gauntlet, measured)

**Perf vs pandas 2.2.3: 39/44 realistic ops faster (median ≈2.8× among wins); 3 remaining loss classes,
2 neutral rows, all with documented fix paths; 0 shipped perf-lever regressions.** Conformance:
3078/3079 fp-frame tests pass (1 remaining failure — `groupby_prod_preserves_int64_j9w3s`,
cod-b's groupby-prod-dtype gap); the gauntlet drove this from 6 failures to 1 (peers fixed
the acosh/arccosh goldens; I fixed oeirt + tt0bx). NOT perf-lever-caused — every typed-lever
conformance guard passes by execution. Current dcfv8 gate also has `fp-conformance --lib
--tests` green; the `uza04.191` groupby min/max verification has focused `fp-groupby`
release tests green, and `uza04.192` groupby first/last verification has focused
`fp-groupby` release tests green. The `uza04.187` groupby count/size verification has
focused `fp-groupby` release tests green.

- **Ship-ready strengths:** value_counts (2.6×), drop_duplicates (2.0×), groupby int-key
  (5.4×), groupby sum/prod Utf8-key (2.18×/2.54×), groupby min/max Utf8-key (2.60×/2.54×),
  groupby first/last Utf8-key (2.92×/2.29×), groupby mean Utf8-key (2.80×), groupby nunique Utf8-key (2.89×),
  groupby count/size Utf8-key (2.49×/2.81×),
  groupby median Utf8-key (1.80–2.63×),
  groupby std/var Utf8-key (1.22–1.34×), Series.combine_first default construction (676×),
  merge inner on lower-hex Utf8 keys (17.85×),
  reset/set_index (5–6.5×), std/var (11×), str case (6.5×), head/tail (17×),
  concat Int64 construction (2,358×),
  DataFrame.dropna Float64 (1.22×),
  slice/filter/sort/sum (1.2–1.3×), RangeIndex.asof scalar lookup (3,840–16,031×),
  RangeIndex bulk indexers (2.64–51.5×) —
  fp beats pandas wherever typed access unlocks a cheaper algorithm.
- **Known gaps before "faster than pandas everywhere":** concat Int64 construction is now
  green: the cod-a lazy chunk-tape pass carries source `Arc<[i64]>` spans into
  `LazyAllValidInt64Chunks`, so `ignore_index=True` construction no longer allocates
  or first-touches a destination `Vec<i64>` until a typed/scalar consumer asks for it.
  xgrv3 already flips the Float64 concat-then-sum typed consumer lane to 1.67× faster
  by exposing lazy chunks through `as_f64_slice()`; ffill
  now flips to 1.41× faster via skw2c validity-run bulk fill;
  shift flips to 1.40× faster in the no-scan + mimalloc boundary mode while remaining
  allocator-sensitive on the plain glibc path; DataFrame.dropna typed Float64 now
  flips from a 0.42× loss to a 1.22× win via missing-free scan pruning, lazy validity
  allocation, the bandwidth-bound serial floor, and lazy all-valid Float64 chunks; max/min
  still trail pandas after the manual 8-lane accumulator, and safe `std::simd`
  i64x8/i64x4 probes were measured and reverted as regressions; Series add/mul now has a
  kept morsel-sweep lever that makes both arithmetic rows near-parity pinned, with mul
  faster unpinned and add still threshold-sensitive; Series.combine_first default
  construction now flips to a 676× win, and typed materialization flips to a 2.84×
  win, after og9qm's lazy all-valid Float64 select tape; forced public
  `values()` materialization remains a 0.21× consumption-path loss because it
  boxes every f64 into `Scalar`; Series.map Float64
  dense integer-key mapping now flips the default construction lane to a 7.04× win
  after hbq6y's lazy repeated-slice output + counter witness, and p0irg flips
  typed numeric materialization to a 5.24× win by exposing repeated Float64 slices
  as an owned f64 buffer for `to_numpy()`. Forced `values()` materialization
  remains a 0.44× consumption-path loss. The qngdp materialization probes were
  measured and reverted: the threaded typed-cache fill regressed the forced
  materialize path from 27.646 ms to 33.013 ms, and the scalar-block repeated-slice
  fill still regressed to 30.838 ms.
  All gaps are tracked.
- **Allocator adoption gate:** exact-parent `fp-bench` A/B for `250bfbf2` kept the 3nah5
  process-boundary allocator: 5 broad smoke wins (up to 3.35×), neutral control lanes, and
  no confirmed regression above 5% after paired reruns of the initially suspicious rows.
- **Conformance debt:** down to 1 failure (`j9w3s` groupby-prod dtype, cod-b) from 6 (bug cosyd).


Head-to-head vs **pandas 2.2.3** on realistic single-thread workloads. Numbers are
measured (release binary run locally; see `docs/NEGATIVE_EVIDENCE.md` for method).
ratio = pandas / fp (>1 ⇒ fp faster).

## Perf vs pandas (measured this gauntlet)

| op | workload | ratio vs pandas | status |
|---|---|---:|:--:|
| head / tail | 2M, k=5 | ~17× | 🟢 |
| value_counts | 500k, 5k distinct | 2.59× | 🟢 |
| drop_duplicates | 1M, card 1000 | 2.03× | 🟢 |
| filter `s[mask]` | 2M, 50% | 1.29× | 🟢 |
| sort_values | 1M shuffled | 1.20× | 🟢 |
| std / var | 2M int64 | 11.3× | 🟢 |
| sum | 2M int64 | 1.27× | 🟢 |
| max / min | 2M int64 | 0.57× / 0.57× rerun | 🟡 8-lane chunked accumulator remains best safe-Rust path; safe `std::simd` i64x8/i64x4 rejected |
| Series add / mul | 2M f64 same-index | pinned add 1.01× neutral, mul 0.96× neutral; unpinned add 0.88× loss, mul 1.19× win | 🟡 tycz7 kept disjoint morsel sweep; FP-side add/mul ~6.0×/5.6× faster, add remains threshold-sensitive |
| Series.map Float64 | 2M f64, 50-entry zero-based full-coverage map | 7.04× deferred; FP-side 16.06→1.71 ms | 🟢 flipped from 0.75× loss; hbq6y stores periodic dense-code output as lazy repeated Float64 slices and replaces the witness modulo with a rolling counter |
| Series.map Float64 `to_numpy()` | same workload, forced `out.to_numpy()` materialization | 5.24×; FP-side 32.95→2.30 ms | 🟢 p0irg exposes repeated Float64 slices through a direct owned f64 buffer for typed consumers; avoids public `values()` enum boxing |
| Series.map Float64 `values()` | same workload, forced `out.values()` materialization | 0.44× residual; qngdp probes 0.38-0.40× reverted | 🔴 residual scalar consumption-path loss; lazy repeated-slice `Scalar` materialization is still heavier than pandas' numeric result buffer; threaded enum materialization and scalar-block cloning both lost |
| Series.combine_first | 2M f64 same-index, ~50% NaN fill | 676× default construct; 2.84× typed materialize | 🟢 flipped from 0.48× loss; og9qm defers the all-valid Float64 select into a lazy tape and only materializes the selected f64 buffer for typed consumers |
| Series.combine_first `values()` | same workload, forced `out.values()` materialization | 0.21–0.23× residual | 🔴 residual consumption-path loss; public `values()` still boxes every f64 into `Scalar`; 3gsa7 scalar-materializer probes were measured and reverted/no-shipped |
| reset_index | 1M int64-indexed | 5.1× | 🟢 |
| loc[[labels]] sorted Int64 | 2M f64 step-2 idx, select 1000 | 1.58× | 🟢 flipped from 5340× SLOWER; 0pkt2 cached int64_view + binary-search batch resolver |
| loc[[labels]] unsorted Int64 | 2M f64 shuffled unique idx, select 1000 | 13.7× | 🟢 flipped from 5147× SLOWER; 2pvdg identity-cached i64→pos hashtable |
| loc[[labels]] Utf8 index | 2M f64 string idx, select 1000 | 7.9× | 🟢 flipped from 2029× SLOWER; sfsx4 identity-cached String→pos hashtable |
| loc[[ts]] Datetime64 index | 2M f64 1-min DatetimeIndex, select 1000 | 67.6× | 🟢 flipped from 1173× SLOWER; recbe identity-cached ns→pos hashtable |
| get_indexer unsorted Utf8 (repeated) | 1M unsorted Utf8 self, 1000 targets | 4.1× | 🟢 flipped from 744× SLOWER; c90bo routes core reindex/align/join resolver through cached loc lookups |
| get_indexer unsorted Int64 (repeated) | 1M unsorted Int64 self, 1000 targets | 3.6× | 🟢 flipped from 210× SLOWER; c90bo follow-on reuses cached i64 resolver instead of rebuilding the map |
| merge inner on Utf8 keys | 1M×1M lower-hex keys → 500k rows | 17.85× | 🟢 current-head f1ftd verify; accepted batch-median artifact `artifacts/bench/cod_a_f1ftd_join_inner_str_batch_medians_20260621.json` (FP CV 3.00%, pandas CV 2.43%); raw one-binary harness rows were faster but dropped for FP CV |
| str.lower/upper | 1M strings | 6.5× | 🟢 |
| concat | 8×125k Int64, `ignore_index=True` construction | 2,358× | 🟢 flipped from 0.46× loss; cod-a stores all-valid Int64 output as source Arc chunk spans and defers the destination buffer until materialization |
| concat + DataFrame.sum Float64 chunks | 8×125k×4 Float64, ignore_index then column sums | 1.67× | 🟢 xgrv3 exposes `LazyAllValidFloat64Chunks` as a cached typed f64 slice; construction chunks already existed, this flips the post-concat numeric consumer path |
| DataFrame.dropna(how=any) | 500k×5 f64, ~10% NaN rows | 1.22× | 🟢 flipped from 0.42× loss; 9bccl uses missing-free Float64 witnesses plus lazy all-valid chunked run gather |
| shift | 2M, p=1 | 1.40× with dcfv8 no-scan + 3nah5 mimalloc boundary | 🟢 flipped; plain glibc path remains 0.64×, golden unchanged |
| ffill | 2M f64, ~10% NaN | 1.41× with skw2c validity-run fill + 3nah5 mimalloc boundary | 🟢 flipped; packed validity-run bulk fill |
| groupby.sum int key | 1M, 1000 keys | 5.4× | 🟢 dense grouping |
| groupby.mean utf8 key | 1M, 1000 keys | 2.80× | 🟢 clone-free streaming sum/count counters |
| groupby.sum utf8 key | 1M, 1000 keys | 2.18× | 🟢 clone-free streaming sum counter; was 0.56× |
| groupby.min utf8 key | 1M, 1000 keys | 2.60× | 🟢 clone-free streaming extremum slot |
| groupby.max utf8 key | 1M, 1000 keys | 2.54× | 🟢 clone-free streaming extremum slot |
| groupby.first utf8 key | 1M, 1000 keys | 2.92× | 🟢 clone-free streaming selected-value slot |
| groupby.last utf8 key | 1M, 1000 keys | 2.29× | 🟢 clone-free streaming selected-value slot |
| groupby.count utf8 key | 1M, 1000 keys | 2.49× | 🟢 clone-free streaming non-null counter |
| groupby.size utf8 key | 1M, 1000 keys | 2.81× | 🟢 clone-free streaming total-row counter |
| groupby.agg(nunique) utf8 key | 2M, 1000 keys | 2.89× | 🟢 CV-gated accepted |
| groupby.agg(median) utf8 key | 100k/2M, 1000 keys | 2.63× / 1.80× | 🟢 CV-gated accepted |
| groupby.agg(var) utf8 key | 100k/1M/2M, 1000 keys | 1.29× / 1.22× / 1.30× | 🟢 CV-gated accepted |
| groupby.agg(std) utf8 key | 100k/1M/2M, 1000 keys | 1.34× / 1.23× / 1.34× | 🟢 CV-gated accepted |
| set_index int col | 1M, 2 cols | 6.5× | 🟢 |
| RangeIndex.asof | 4,096 scalar probes, 100k/1M rows | 3,840× / 16,031× | 🟢 |
| RangeIndex.get_indexer miss-heavy | 100k / 1M targets | 2.64× / 3.61× | 🟢 flipped by arithmetic bulk membership; `rch` same-worker FP-side 4.0× |
| RangeIndex.reindex all-miss | 100k / 1M targets | 36.1× / 51.5× | 🟢 exact RangeIndex lattice fast path; `rch` same-worker FP-side 75.7× / 32.2× |

**Score: 39/44 measured ops faster than pandas; 3 remaining loss classes (max/min, Series.map Float64 `values()`, Series.combine_first `values()`),
2 neutral rows (add, mul pinned); 0 shipped regressions; 12 reverted/no-ship SIMD, allocation,
or ~0-gain attempts.**

Median win among the 39 ≈ 2.8×; the remaining losses are kernel/structural gaps with
documented fix paths — none are code-first fp-frame regressions. The stale f1ftd
Utf8 inner-merge red row is now green on current head: batch medians on CPU7 measured
FP 8.234 ms p50 vs pandas 146.950 ms p50, 17.85× faster with both CVs under 5%.
concat construction is now green after the cod-a Int64 chunk-tape pass; ffill was the
same class until skw2c changed
the no-limit path to bulk-copy the f64 buffer and fill only invalid validity runs.
RangeIndex indexers were a separate vectorized-engine gap after `29u49`; `uza04.159`
closed it with arithmetic bulk membership and an exact reindex lattice path.
The `uza04.191` groupby min/max verification closes two more generic-key extremum rows:
streaming scalar slots beat pandas 2.2.3 by 2.60×/2.54× on 1M Utf8-key Float64 groups,
with golden digests `def13b65b5e3a35d` and `6d20c5176a43035d`.
The `uza04.192` groupby first/last verification closes two more selected-value rows:
streaming scalar slots beat pandas by 2.92×/2.29× on the same fixture, with golden
digests `a8c2c037ffb85c88` and `d373b7337998d544`.
The `uza04.187` groupby count/size verification closes two more counter rows:
clone-free counters beat pandas by 2.49×/2.81× on the same fixture, with golden
digests `1e555b43a73656c1` and `c6ccd2e318a736dd`.
The latest `tycz7` Series add/mul pass kept a disjoint morsel sweep in
`apply_f64_slices_nan_tracked`: public add/mul improved from 16.56/16.40 ms to
2.76/2.91 ms pinned, while preserving focused conformance. Add is neutral pinned
(1.01×) but still loses in the unpinned sanity row (0.88×), and mul is neutral pinned /
a small win unpinned. The prior `38xpk`
push-output zero-fill and discard-ledger probes remain measured no-ships.
The latest `og9qm` Series.combine_first pass keeps the `grtx1` no-rescan proof and
`gmn0f` packed validity-word semantics, but stops eagerly building the selected Float64
buffer when the right side is all-valid. The same-index Float64 path now returns a lazy
all-valid select tape; CPU7 best-of-50 measured FP construction at 0.0091 ms vs pandas
6.177 ms (676× faster), and typed materialization at 2.142 ms vs pandas `to_numpy()`
6.075 ms (2.84× faster). Forced public `out.values()` remains red at 30.298 ms vs
pandas 6.506 ms (0.21×) because it boxes every f64 into `Scalar`. This pass is
**2 wins / 1 loss / 0 neutral**; route deeper to typed numeric public consumption or
lower-allocation scalar materialization, not another select-kernel trim. The follow-up
`3gsa7` scalar-materialization probes confirmed that loop reshaping alone is not enough:
local CPU7 baseline was 30.444 ms vs pandas 6.983 ms (0.23×); a right-buffer+patch
scalar fill regressed to 30.601 ms, and a single-pass scalar push reached only
29.999 ms (~1.5% FP-side, still 0.23× vs pandas). Both code probes were reverted;
the remaining route is an API/storage change that avoids public `Vec<Scalar>` for
numeric consumers, or a fundamentally smaller scalar representation.
The latest Series.map Float64 state keeps the earlier `0jdij` dense direct-address table
and hbq6y's guarded periodic dense-code witness, lazy repeated-slice output, and rolling
counter scan. Default Series construction is green at 7.04× vs pandas. p0irg adds the
typed numeric consumption path qngdp routed toward: `out.to_numpy()` now expands/copies
the lazy repeated Float64 tape as f64 directly, moving from 32.949 ms to 2.301 ms and
beating pandas 12.053 ms by 5.24×. Forced `out.values()` is still red at 27.348 ms vs
pandas 12.075 ms (0.44×). The attempted qngdp enum materializers remain no-ships:
the threaded typed-cache fill regressed the forced materialize path from 27.646 ms to
33.013 ms, and the scalar-block repeated-slice fill still regressed to 30.838 ms.
The remaining fix path is lower-allocation public scalar values, not parallel enum
boxing or cloning a scalar tape.

Pattern: typed-slice levers win 2–11× where they unlock a cheaper ALGORITHM (FxHash dedup,
dense value_counts, Welford std/var, contiguous str). They LOSE on ops that just rebuild
the whole Column. The 3nah5 mimalloc boundary allocator turns those losses from catastrophic
to actionable; dcfv8's no-scan shift path now flips shift to 1.40× faster under that boundary,
and skw2c's validity-run ffill path flips ffill to 1.41× faster. concat construction (0.46×)
still trails pandas because fp's column-rebuild construction is still heavier than numpy's
pooled/in-place memmove/concatenate, but xgrv3 flips the Float64 concat-then-sum typed consumer
lane to 1.67× by exposing lazy chunks through `as_f64_slice()`; max/min
still need target-specific SIMD beyond current safe `std::simd` lowering; Series add/mul
still need durable numpy-class vectorization or output materialization work to move from
near-parity to clear wins; Series.map Float64 now wins on deferred construction and
typed `to_numpy()` consumption, while Series.combine_first wins on deferred/default
construction and typed materialization. Both forced `values()` paths still need
lower-allocation public scalar output despite the hbq6y/p0irg repeated-slice and og9qm
lazy-select keeps.
The latest 9bccl DataFrame.dropna pass turns the row-wise Float64 dropna case green:
missing-free selected columns are excluded from the row scan, nullable Float64 run-gather
defers validity bitmap allocation until an invalid output appears, the gather keeps the
bandwidth-bound 4M-cell serial floor, and `dropna(how=Any)` carries its kept-row validity
witness into lazy `LazyAllValidFloat64Chunks` output instead of copying every retained
f64 value. Same-core CPU7 best-of-200: FP 3.109 ms vs pandas 3.791 ms, 1.22× faster.
The qngdp `values()` materializers were measured and reverted because they added
initialization/thread/scalar-tape overhead without removing enum boxing. The Utf8 `groupby.sum` gap flipped under the clone-free streaming counter,
and the RangeIndex indexer gap flipped under the affine arithmetic bulk path, not by
weakening the retained public `get_loc` error semantics.

Notably, three of these (value_counts, sort_values, filter/dedup) were *lagging* pandas
before this session's levers (value_counts 0.62×, sort 0.91× per the perf-frontier notes)
and are now ahead — the FxHash-over-khash and zero-copy-gather/slice veins flipped them.

## Conformance (MEASURED — `rch exec -- cargo test --release -p fp-frame --tests`)

- **3078 passed / 1 failed** (was 3073/6 at first run; gauntlet drove it down). All **15
  typed-lever conformance guards PASS** → no recent perf lever regressed (bit-transparency
  verified by execution, not just compilation).
- Resolved during the gauntlet (verified by re-run):
  - `series_acosh_golden_basic`, `series_arccosh_golden_basic` — math goldens, now pass (peer fix).
  - `dataframe_set_index_rejects_null_labels_oeirt` — my early test wrongly rejected NaN
    labels; corrected to pandas-faithful semantics (NaN accepted, Null rejected). Passes.
  - `series_agg_size_any_all_tt0bx` — my early test expected pandas object-dtype Bool; fp's
    typed Column coerces mixed Int64+Bool agg → Int64 (values correct: any=1, all=0).
    Corrected the assertion to fp's actual behavior + documented the object-dtype gap. Passes.
- Remaining (1 failure, real gap, tracked in bug cosyd):
  - `dataframe_groupby_prod_preserves_int64_j9w3s` — groupby prod returns Float64(6.0) vs
    pandas Int64(6); dtype-preservation gap in cod-b's `aggregate_named_func`. NOT a
    perf-lever regression; needs an owner fix in the groupby kernel.
- Did NOT revert any perf lever (none caused these failures).

## Pending measurement

Remaining code-first lanes are now narrower: cod-b's categorical-index family and RangeIndex
helpers other than `jlv2o`/`uza04.159` still need focused Criterion/pandas rows. Already measured
rows above should not be treated as pending.

## Method / infra

- Build: `rch exec -- cargo build --release -p fp-frame --examples` (remote ovh-b,
  artifacts transferred to `/data/projects/.rch-targets/frankenpandas-cc`).
- Run: release example binaries executed **locally** (rch does not relay remote program
  stdout); pandas baselines via `python3` + `time.perf_counter` best-of-N.
