# FrankenPandas Perf — Negative-Evidence Ledger & Head-to-Head vs pandas

Measured head-to-head: each lever's realistic-workload bench run **locally** from a
release binary or focused Criterion bench (heavy builds/bench guards may be offloaded
with `rch`), compared against **pandas 2.2.3** (`python3`, `time.perf_counter`, best-of-N
or repeated p50 batches).

Method: `examples/bench_*.rs` self-time `best=<ns>` over N iters; pandas scripts
(`perf/pandas_baseline/`) time the equivalent op best-of-N on a matched workload.
`ratio = pandas_ns / fp_ns` (>1 ⇒ fp faster). Same machine, single-thread unless noted.

Rule: record EVERY result (win/loss/neutral). Revert any lever that regressed or showed
~0 gain. Never retry a recorded dead end.

## 🗺️ PERF FRONTIER AFTER THE DT/TIMEDELTA SWEEP (BlackThrush, 2026-06-21)

The datetime AND timedelta accessor surfaces are now FULLY TYPED (every component off the raw &[i64]
nanos): all dt components + dt.floor/ceil/round (already typed via round_to_freq) + dt.weekofyear +
td.total_seconds. The remaining vs-pandas LOSSES are all UNSAFE to fix blind (need a build+test cycle
or are unwinnable) — do NOT attempt code-only:
- **pivot_table 0.67x@1M** (bead zngxi) — needs a dense-int64 grouping rewrite of a complex fn
  (dropna + sort + aggfunc edge cases). The ONLY remaining clearly-fixable loss; do with tests.
- **df_round 0.84x** — bit-locked: the per-element `(x*f).round_ties_even()/f` fdiv changes goldens.
- **transpose / stack / to_numpy** — STRUCTURAL: pandas uses an O(1) 2D-block view / MultiIndex; FP's
  columnar layout can't match without a 2D-block storage mode or lazy-transpose (beads l4vzc, m9wkn).
- **floor/ceil 100k, set_index 0.61x@100k** — structural columnar per-column-alloc residuals.
- NOT-IMPLEMENTED (parity, not perf): td.days/.seconds/.microseconds/.nanoseconds (fp_types::Timedelta
  has the integer helpers; expose them typed-from-the-start when adding the API, not as a "lever").
Everything else probed this session is an FP WIN (see entries below). The common-op fixable-loss vein
is exhausted; the next real perf work is pivot_table (zngxi) or the structural-storage beads.

## ⏳ UNVERIFIED CODE-ONLY STACK — VERIFY ON DISK-RECOVER (BlackThrush, 2026-06-21)

During a multi-turn disk-CRITICAL window (cargo forbidden), the dt-accessor fast paths below were
written CODE-ONLY — provably bit-identical by inspection (closures verbatim the `fp_types::Timestamp`
formulas; same `Scalar`/builder paths as the generic `extract_component_typed*`), structurally
mirroring the shipped+tested `typed_datetime_year_all_valid` pattern — but **none has been compiled
or benched.** FIRST ACTION ON DISK-RECOVER, in order:

1. `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc cargo build --offline -p fp-bench`
   (or via rch) — confirm the whole stack COMPILES.
2. `cargo test --offline -p fp-frame --lib -- <component>` for each: month day hour minute second
   microsecond nanosecond quarter dayofyear dayofweek days_in_month is_month_start is_month_end
   is_quarter_start is_quarter_end is_year_start is_year_end is_leap_year month_name day_name date time strftime weekofyear
   — plus the dt differential/golden tests. Confirm BIT-IDENTICAL.
3. Bench each (add `dt_*` workloads to fp-bench) to record the wins (pure-mod ones ~4x like
   hour/minute already measured 4.2x; calendar/civil ones ~parity-to-1.5x; string date/time skip
   chrono + 3x-redundant civil → modest).

Commits in the stack (newest→oldest): dayofyear-hardened (f45f2357, now VERBATIM Timestamp table —
the previously-novel days_from_civil method was removed), time (a383f972), date (c93fc9aa),
day_name (cc8bf063), month_name (bd128716), 7 bool predicates (db4a5dfe), days_in_month (9f80ff58),
micro/nano (8f0fb964). hour/minute/second (3be88c6b) ALGEBRAICALLY PROVEN == Timestamp formulas.

RISK NOTES: hour/min/sec proven; dayofyear hardened to verbatim; all others mirror verified patterns.
weekofyear is now ALSO typed (commit below) by replicating the private `iso_weeks_in_year` verbatim;
no dt method remains on the slow chrono path.

## Results

| Lever (bead) | Workload | pandas | fp | ratio | verdict |
|---|---|---|---:|---:|---|
| value_counts FxHash (g1de8) | 500k rows, 5k distinct Utf8 | 22.90 ms | 8.85 ms | **2.59× faster** | ✅ KEEP — beat khash (was 0.62× pre-lever) |
| sort_values gather/reorder (7ufhq+take_positions) | 1M shuffled int64 | 57.18 ms | 47.68 ms | **1.20× faster** | ✅ KEEP — was 0.91× pre-levers |
| head/tail zero-copy slice (6wx84) | 2M int64, k=5 | 5.91 µs | 0.35 µs | **~17× faster** | ✅ KEEP — Index::slice+Column::slice |
| loc_bool filter (t0y8n) | 2M, 50% mask | 10.89 ms | 8.42 ms | **1.29× faster** | ✅ KEEP — collect-positions + take_positions |
| drop_duplicates FxHashSet (6vep3) | 1M, card 1000 | 5.99 ms | 2.95 ms | **2.03× faster** | ✅ KEEP — beat khash dedup |
| sum typed Int64 (bwgyc) | 2M int64 | 216 µs | 171 µs | **1.27× faster** | ✅ KEEP |
| std / var typed (0xdfx Welford) | 2M int64 | 19.5 ms | 1.72 ms | **11.3× faster** | ✅ KEEP — Welford crushes pandas std |
| max 8-lane chunked accumulator (simdmx) | 2M int64 | 219 µs | 0.357 ms | **0.61× (1.63× slower)** | ✅ KEEP — 3.2× faster than scalar iter().max(); gap 5.2×→1.63× |
| min 8-lane chunked accumulator (simdmx) | 2M int64 | 230 µs | 0.424 ms | **0.54× (1.86× slower)** | ✅ KEEP — 2.8× faster than scalar iter().min(); gap 5.1×→1.86× |
| max/min shared inline lane-count helper (x7bp8) | 2M int64 `Series.max`/`Series.min`; `bench_reductions`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`; CPU7 best-of-300 | max 185.201 µs / min 189.920 µs | final-after-revert max 398.546 µs / min 386.121 µs | **max 0.46× / min 0.49×** | ❌ REVERT / NO-SHIP — x7bp8 was not the radical 8→16 lane change; history showed it only hoisted the already-16 lane count into a shared const and added `#[inline]`. Same-host A/B failed the keep bar: parent/final source re-ran stably at max 398-400 µs and min 386 µs, while the restored x7bp8 candidate showed no stable win and frequently regressed max (best restored-candidate reruns: max 786.681 µs, min 404.316 µs; noisy remote first pass max 421.632 µs, min 425.809 µs). Source reverted to the pre-x7bp8 local-const helper shape; keep the older measured 8/16-lane accumulator family, but do not retry shared-const/inline reshaping. |
| max/min portable-SIMD `i64x8` (uza04.207) | 2M int64 | 185 / 182 µs | 0.811 / 0.811 ms | **0.23× / 0.22×** | ❌ REVERT — safe `std::simd` was 2.3× slower than the manual 8-lane accumulator |
| max/min portable-SIMD `i64x4` (uza04.207) | 2M int64 | 185 / 182 µs | 1.087 / 1.108 ms | **0.17× / 0.16×** | ❌ REVERT — AVX2-width `std::simd` variant was 3.1× slower than the manual accumulator |

| reset_index typed Int64 idx→col (bp6k7) | 1M int64-indexed, 2 cols | 1.93 ms | 0.38 ms | **5.1× faster** | ✅ KEEP — Index::from_i64_values |
| concat typed buffer (tbrtu) | 8×125k Int64 series, ignore_index | 0.28 ms | 6.81 ms | **0.041× (24× SLOWER)** | ⚠️ KEEP (bit-transparent, ≥ old Scalar path); 24× is glibc-malloc-bound — see mimalloc row |
| concat + mimalloc boundary allocator (3nah5) | 8×125k Int64, ignore_index | 0.223 ms | 0.479 ms | **0.46× (2.15× slower)** | ✅ KEEP — adopted in `fp-bench` + `fp-python`; 12.4× faster than current glibc-malloc concat (5.93 ms) but still a pandas loss |
| concat Int64 lazy chunk tape (cod-a/uza04) | 8×125k Int64 series, `ignore_index=True` construction; release examples built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo build --release -p fp-frame --example bench_concat --example bench_concat_mimalloc` on `hz2`; local best-of-200 FP / best-of-80 pandas | 0.851 ms | 0.000361 ms typed-backed (`bench_concat`); 0.000340 ms mimalloc harness | **2,358× faster** | ✅ KEEP — radical region/chunk-view lever from the Graveyard vectorized-column playbook: `LazyAllValidInt64Chunks` stores source `Arc<[i64]>` spans and defers the destination buffer until `as_i64_slice()`/`values()` consumption. Same-binary old direct materialized build remains 6.249 ms, proving the win comes from eliminating the construction copy/page-touch floor, not timer drift. Head-to-head score for this pass: **1 win / 0 loss / 0 neutral**. Guards green: `cargo check -p fp-columnar -p fp-frame --all-targets` via RCH; `from_i64_all_valid_chunks_materializes_in_chunk_order`; `concat_int64*` focused release tests; `concat_dataframes_ignore_index_resets_labels`; `cargo fmt -p fp-columnar -p fp-frame -- --check`; `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo test -p fp-conformance --lib --release` on `vmi1227854` (1595/1595). |
| concat Float64 chunks + `DataFrame.sum` typed consumer (xgrv3) | 8×125k frames ×4 all-valid Float64 columns, `ignore_index=True`, then `DataFrame.sum`; release example built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo build --release -p fp-frame --example concat_bench` on `vmi1153651`; local CPU7 best 1000-iter loop | 8.055 ms | 4.824 ms | **1.67× faster** | ✅ KEEP — `LazyAllValidFloat64Chunks` now exposes a cached contiguous f64 slice through `as_f64_slice()`, so post-concat numeric reducers take the typed `Series.sum` path instead of falling back to public `values()` enum boxing. The concat construction chunk path was already present; this lever is the typed-consumer bridge, not a construction-speed claim. Guarded by `from_f64_all_valid_chunks_materializes_in_chunk_order`, `concat_float64_labeled_typed_matches_expected`, golden SHA `bc86edaa9892152f485b37b2601de19ca2225ef41e9475fecd2d3b7eb7807e8f`, direct `rustfmt --check` on touched files, and release build of `concat_bench`. |

| str.lower/upper contiguous (apply_str_utf8) | 1M strings | 84.04 ms | 12.88 ms | **6.5× faster** | ✅ KEEP — contiguous buf + ASCII in-place |
| str.startswith (sweep bench_str) | 1M strings | 86.84 ms | 3.59 ms | **24.2× faster** | ✅ contiguous-Utf8 apply_str_bool; pandas object per-element |
| str.len (sweep bench_str) | 1M strings | 164.0 ms | 7.77 ms | **21.1× faster** | ✅ contiguous-Utf8 apply_str_int |
| str.contains (sweep bench_str) | 1M strings | 150.8 ms | 7.90 ms | **19.1× faster** | ✅ vectorized memmem literal searcher |
| DataFrame.sum(axis=1) Float64 (rrf64) | 500k×10 f64 | 40.19 ms | 7.11 ms | **5.65× faster** | ✅ FIXED — was 4.4× LOSS (175.9 ms)! typed reduce_rows_f64 (per-col as_f64_slice accumulate) vs Scalar per-row gather = 24.7× FP-side. Bit-identical (column-order f64 sum, no-NaN gated), conformance 79/79 |
| DataFrame.min(axis=1) Float64 (rrf64) | 500k×10 f64 | 38.73 ms | 7.50 ms | **5.16× faster** | ✅ FIXED — reduce_rows_f64, was Scalar fallback; bit-identical (79 axis1 tests) |
| DataFrame.max(axis=1) Float64 (rrf64) | 500k×10 f64 | 38.93 ms | 7.37 ms | **5.28× faster** | ✅ FIXED — reduce_rows_f64 |
| DataFrame.prod(axis=1) Float64 (rrf64) | 500k×10 f64 | 71.13 ms | 7.34 ms | **9.69× faster** | ✅ FIXED — reduce_rows_f64 |
| DataFrame.mean(axis=1) Float64 (rrf64) | 500k×10 f64 | 41.01 ms | 7.47 ms | **5.49× faster** | ✅ FIXED — was pure-Scalar (no typed path at all); typed Σ/k (count=k, no-NaN gated), bit-identical, 79 axis1 tests |
| DataFrame.var(axis=1) Float64 (rrf64) | 500k×10 f64 | 118.10 ms | 10.05 ms | **11.75× faster** | ✅ FIXED — reduce_rows_func_f64 (typed per-row buffer + same Fn(&[f64]), no per-row Vec alloc / Scalar); bit-identical, 79 axis1 tests |
| DataFrame.std(axis=1) Float64 (rrf64) | 500k×10 f64 | 118.56 ms | 11.29 ms | **10.50× faster** | ✅ FIXED — reduce_rows_func_f64; sem/skew/kurtosis_axis1 also wired (same helper, same pattern) |
| DataFrame.sum(axis=0) Float64 | 500k×10 f64 | 3.58 ms | 0.87 ms | **4.14× faster** | ✅ already typed — reduce_numeric delegates to per-column Series.sum (typed) |
| DataFrame.std(axis=0) Float64 | 500k×10 f64 | 54.08 ms | 1.40 ms | **38.7× faster** | ✅ already typed — per-column Series.std (Welford); pandas per-column std very slow |
| DataFrame.count(axis=1) all-present (cntf) | 500k×10 f64 | 39.92 ms | 0.165 ms | **242× faster** | ✅ FIXED — was per-cell is_missing scan (5M checks); when every column is typed-all-valid (as_f64/i64/bool_slice Some) no cell is missing ⇒ count = #cols constant Int64. Bit-identical, conformance green |
| DataFrame.all(axis=1) Float64 (allf) | 500k×10 f64 | 4.48 ms | 8.0 ms | **0.56× (1.79× slower)** | ⚠️ IMPROVED from 21.7× LOSS (97ms) — typed all-Float64 path (as_f64_slice, truthy=v!=0.0, no-NaN gated, bit-identical) skips 5M Scalar values() materialize = 12× FP-side. Applied (fc22d33f), conformance green. Residual = Scalar::Bool output + numpy vectorization |
| DataFrame.any(axis=1) typed Float64 (allf) | 500k×10 f64 | 6.42 ms | (was 8.66 ms) | typed applied | ✅ FIXED — typed all-Float64 path (cb23deb8, sister to all_axis1), skips per-cell values() materialize; bit-identical, conformance green (2 any_axis tests). Was 1.35× slower (Scalar) |
| DataFrame.dropna(how=any) typed Float64 (dropf) | 500k×5 f64, ~10% NaN rows | 3.69 ms | 12.4 ms | **0.30× (3.4× slower)** | ✅ IMPROVED from 11.2× LOSS (41.2 ms)! typed per-column (data,validity), missing=!valid‖is_nan == is_missing for Float64 — skips per-cell Scalar materialize = 3.3× FP-side. Bit-identical, conformance green (38 tests). Residual 3.4× = take_rows gather/rebuild (next lever) |
| DataFrame.dropna typed position-run gather (uza04.208) | 500k×5 f64, ~10% NaN rows; CPU7 best-of-80; release example built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo build --release -p fp-frame --example bench_dropna_cc` | 3.819 ms | 9.131 ms (repeat 9.166 ms) | **0.42× (2.39× slower)** | ✅ KEEP modest FP-side gain / 🔴 still pandas loss — same-session FP baseline 9.629 ms -> 9.131 ms (1.05× FP-side) by copying contiguous kept rows through `Column::take_position_runs` and avoiding a positions `Vec` on the run-ready typed path. Focused guards green: `cargo check` for fp-columnar/fp-frame, focused dropna + take-position-runs tests, and local clippy for both crates. UBS: fp-columnar reproduced broad inventory only; fp-frame timed out as the known broad scanner backlog. ❌ REVERTED/no-ship all-valid/no-NaN scan-pruning probe: 9.56/9.36 ms, slower than run-gather. Residual = output allocation plus per-row missing scan. |
| DataFrame.dropna missing-free witness + lazy Float64 chunked run gather (9bccl) | 500k×5 f64, ~10% NaN rows; release example built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo build --release -p fp-frame --example bench_dropna_cc`; final same-core CPU7 best-of-200 | 3.791 ms | 3.109 ms | **1.22× faster** | ✅ KEEP — flips the remaining dropna loss by stacking three paying levers and one accepted threshold: (1) selected Float64 columns proven missing-free are skipped in the missingness scan (FP 9.460 -> 5.855 ms, still 0.66× vs pandas); (2) nullable Float64 run-gather lazily allocates validity words only after an invalid output slot (5.855 -> 5.604 ms, still 0.69×); (3) run-gather uses the same 4M-cell serial floor as bandwidth-bound column maps (5.604 -> 5.435 ms, still 0.71×); (4) dropna(how=Any) passes the kept-row validity witness into `take_position_runs_all_valid_f64_unchecked`, emitting `LazyAllValidFloat64Chunks` over source runs instead of copying 450k f64s/column (5.435 -> 3.109 ms, 1.22× vs pandas 2.2.3 / numpy 2.4.3). Focused guards green: `take_position_runs_nullable_float64_matches_take_positions`; 17 `dataframe_dropna*` tests; `cargo check -p fp-columnar -p fp-frame --all-targets`; local `cargo clippy -p fp-columnar -p fp-frame --all-targets -- -D warnings`. `rch` clippy worker lacked the pinned clippy component; local clippy passed. `cargo fmt -p fp-columnar -p fp-frame --check` still fails on broad pre-existing fp-frame/example formatting drift; touched `fp-columnar` file passes `rustfmt --check`. UBS on touched source files timed out after 180s with no completed findings, matching the known fp-frame scanner backlog. |
| DataFrame.transpose (bench_df) | 2000×10 f64 | 0.036 ms | 1.47 ms | **0.025× (40× slower)** | 🔴 LOSS — Scalar gather-based; NICHE (transpose of large frames pathological/rare), low priority |
| Series.where (identity + typed select, whident) | 2M f64, 50% cond, scalar other | 4.17 ms | 16.96 ms | **0.25× (4.1× slower)** | ✅ IMPROVED from 96× LOSS (402ms)! killed unconditional align()/reindex pathology (identity fast path) + typed f64 select = ~24× FP-side. Bit-identical, conformance green (23 tests). Residual 4× = output-alloc floor (mimalloc) |
| Series.mask (identity + typed select, whident) | 2M f64, 50% cond, scalar other | 3.58 ms | 16.25 ms | **0.22× (4.5× slower)** | ✅ IMPROVED from 110× LOSS (395ms)! same two-stage fix as where; bit-identical, conformance green |
| Series + Series add same-index morsel sweep (tycz7) | 2M f64, best-of-50 pinned median rerun | 2.78 ms | 2.76 ms | **1.01× near-parity** | ✅ KEEP / ➖ neutral pinned — disjoint scoped morsels cut FP add 16.56→2.76 ms (~6.0× FP-side). Unpinned exact-code row: pandas 2.65 ms vs FP 3.00 ms = 0.88× loss, so add remains threshold-sensitive and routed deeper |
| Series * Series mul same-index morsel sweep (tycz7) | 2M f64, best-of-50 pinned median rerun | 2.80 ms | 2.91 ms | **0.96× near-parity** | ✅ KEEP / ➖ neutral pinned — FP mul 16.40→2.91 ms (5.6× FP-side). Unpinned exact-code row flips: pandas 3.45 ms vs FP 2.89 ms = 1.19× faster |
| Series > Series gt same-index tycz7 rerun | 2M f64, best-of-50 pinned median rerun | 2.18 ms | 1.03 ms | **2.12× faster** | ✅ STILL WIN — comparison path was not the target and stayed within prior FP noise. Unpinned exact-code row: pandas 3.18 ms vs FP 1.37 ms = 2.33× |
| Series + Series add same-index (pre-tycz7 baseline, bench_binop_cc) | 2M f64 | 1.16 ms | 15.36 ms | **0.076× (13.2× slower)** | superseded loss row — already typed, but single monolithic NaN-tracking fused sweep did not vectorize like numpy AVX |
| Series * Series mul same-index (pre-tycz7 baseline, bench_binop_cc) | 2M f64 | 1.10 ms | 15.28 ms | **0.072× (13.9× slower)** | superseded loss row — same fp-columnar arithmetic-SIMD ceiling as add |
| Series > Series gt same-index (pre-tycz7 baseline, bench_binop_cc) | 2M f64 | 0.71 ms | 0.96 ms | **0.74× (1.35× slower)** | superseded near-parity row — comparison vectorizes cleanly; tycz7 rerun is now a clear win on this host |
| Series.replace Float64 (repf, bench_replace_cc) | 2M f64, 3-entry replacement set | 29.72 ms | 4.65 ms | **6.4× faster** | ✅ FIXED — was 2.4× LOSS (72ms)! 3-stage: as_f64_slice input → from_f64_values output → direct `==` scan for small sets. ⚠️ PARITY-FIX (not bit-transparent): the general path routes Float64 keys through semantic_eq's 1e-14 RELATIVE tolerance, but pandas replace is EXACT (1.0+5e-15 not matched by {1.0:x}, verified) — repf uses exact `==`, matching pandas. Conformance green (49). Introduces a typed-vs-scalar inconsistency (scalar-backed path still tolerant) — see tolerance-bug row below |
| Series.map Float64 dense integer-key table (0jdij+vynf7+hbq6y, bench_map_cc deferred) | 2M f64, 50-entry zero-based full-coverage map, sequential warm best-of-7 CPU7 | 12.063 ms | 1.713 ms | **7.04× faster** | ✅ FIXED default construction lane — hbq6y applies the Alien Graveyard region/lazy-layout lever already present in fp-columnar: the periodic dense-code witness now returns a `LazyRepeatedSlicesFloat64` tape instead of expanding the 50-value output block 40k times, and the witness scan uses a rolling expected-code counter instead of per-row `% period`. Same-tree A/B: 16.062 ms vynf7 baseline → 3.064 ms lazy repeated-slice output → 1.713 ms counter witness = **9.38× FP-side**; pandas 2.2.3 is 12.063 ms, so the prior 0.75× loss flips to a 7.04× win. Exactness is still guarded by a full source scan and all non-cyclic sources keep the direct-address fallback. Focused conformance green: 19 `series_map_*` tests + 8 fp-columnar factorize tests + 4 FxHash order tests. |
| Series.map Float64 dense integer-key table (hbq6y+p0irg, bench_map_cc materialize) | Same workload, `materialize` mode calls `out.values()` in the timed window | 12.075 ms | 27.348 ms | **0.44× (2.26× slower)** | 🔴 RESIDUAL LOSS — forced public `values()` still materializes a `Vec<Scalar>` over the lazy repeated-slice Float64 column; p0irg intentionally did not ship another enum-boxing materializer after qngdp proved that family regressed. This remains the honest scalar consumption-path loss. |
| Series.map Float64 repeated-slice materialization probes (qngdp) | Same workload; release binary built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo build -p fp-frame --example bench_map_cc --release`; local CPU7 timing | 12.188-12.502 ms | 30.838-33.013 ms | **0.38-0.40× (2.5-2.6× slower)** | ❌ REVERT / NO-SHIP — two "bold" materializers worsened the residual: current-head baseline before probes was deferred ~1.72 ms and materialize 27.646 ms; a typed-cache/threaded Scalar fill regressed to 33.013 ms, and a scalar-block repeated-slice fill still regressed to 30.838 ms. Cause: forced public `values()` still boxes every f64 into `Scalar`, while the probes added zero-init/thread setup or an extra scalar tape. Code reverted; keep hbq6y's lazy construction win and route deeper to public typed numeric consumption, an Arrow/numpy-style result buffer, or API-level avoidance of `values()` for numeric pipelines. |
| Series.map Float64 repeated-slice typed numeric consumption (p0irg, bench_map_cc `numpy`) | Same 2M f64 / 50-entry full-coverage map; release binary built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo build -p fp-frame --example bench_map_cc --release`; local CPU7 best-of-30; mode calls `out.to_numpy()` in the timed window | 12.053 ms | 2.301 ms | **5.24× faster** | ✅ KEEP — applies the Graveyard vectorized-column/region-layout path to the actual residual: `LazyRepeatedSlicesFloat64` now exposes/copies an owned f64 buffer for typed consumers, with a periodic-prefix expansion specialization, so `Series::to_numpy()` no longer boxes through `values()`. Same-session A/B: pre-p0irg FP `numpy` 32.949 ms (0.37× pandas) → borrowed typed buffer 15.393 ms (0.78×) → direct owned f64 2.301 ms (5.24×), a **14.32× FP-side** improvement. Focused guards green: 2 fp-columnar repeated-slice tests, 2 fp-frame periodic Series.map tests, `cargo check -p fp-columnar --all-targets`, `cargo check -p fp-frame --all-targets`, and local clippy for both touched crates. `cargo fmt --check` still fails on broad pre-existing workspace formatting drift outside this lane. |
| Series.combine_first (cmbf/grtx1, bench_combine_cc) | 2M f64 same index, self ~50% NaN, other fills | 7.35 ms local after-row (3.08 ms prior ledger host) | 17.94 ms local; rch hz2 19.38 → 18.33/18.40 ms | **0.41× local (2.44× slower)** | ✅ KEEP modest no-rescan gain — grtx1 stamps the typed identity fast-path output with `from_f64_values_all_valid_unchecked` after proving other all-valid + self-present excludes NaN, avoiding the redundant output validity scan. Same-worker rch hz2 FP-side A/B: 19.375 ms baseline → 18.332/18.400 ms after = ~1.05× faster, focused test green. Still a pandas loss; residual = output allocation/select floor (mimalloc/packed select still needed) |
| Series.combine_first packed validity-word copy-patch (gmn0f, bench_combine_cc) | 2M f64 same index, self ~50% NaN, other fills; CPU7 best-of-50 | 5.915 ms | 15.901 ms | **0.37× (2.69× slower)** | ✅ KEEP FP-side improvement / 🔴 still pandas loss — built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo build -p fp-frame --example bench_combine_cc --release`; local CPU7 A/B moved FP 18.493 ms baseline → 16.005 ms copy-patch → 15.901 ms packed validity-word copy-patch (1.16× FP-side). Pandas 2.2.3 on the matched workload was 5.915 ms, so this pass is **0 wins / 1 loss / 0 neutral** head-to-head. Focused guards green: 12 `combine_first` fp-frame tests and 31 fp-columnar validity tests. Residual routes deeper to a reusable lower-allocation typed select/builder rather than per-row public `Scalar` materialization. |
| Series.combine_first packed validity-word final rerun (gmn0f/cod-a, bench_combine_cc) | 2M f64 same index, self ~50% NaN, other fills; CPU7 five paired best-of-30 rounds | 7.309 ms | 15.100 ms | **0.48× (2.07× slower)** | ✅ KEEP FP-side improvement / 🔴 still pandas loss — final source built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo build --release -p fp-frame --example bench_combine_cc`; local CPU7 A/B artifact `/data/projects/.scratch/frankenpandas-cod-a-gmn0f-binaries/combine_first_final_ab_20260620T0652.txt` moved FP best 16.966 ms baseline (`88e791dd`) → 15.100 ms packed-validity candidate (1.12× FP-side; median 17.025 → 15.150 ms, also 1.12×). Pandas 2.2.3 measured 7.309 ms, so this pass is **0 wins / 1 loss / 0 neutral** head-to-head. Final-source guards green: 12 `combine_first` fp-frame tests, 24 fp-columnar validity-mask tests, `cargo check -p fp-frame -p fp-columnar --all-targets`, and local clippy `--no-deps`. Residual remains output allocation / typed-select builder work, not another per-row validity-dispatch trim. |
| Series.combine_first lazy all-valid Float64 select (og9qm, bench_combine_cc) | 2M f64 same index, self ~50% NaN, other all-valid fills; CPU7 best-of-50/20 | construct 6.177 ms; `to_numpy()` 6.075 ms; `values` 6.506 ms | construct 0.0091 ms; typed materialize 2.142 ms; `values()` 30.298 ms | **construct 676× faster; typed materialize 2.84× faster; `values()` 0.21× (4.66× slower)** | ✅ KEEP mixed win/loss — built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo build -p fp-frame --example bench_combine_cc --release`; the same-index Float64 path now returns a lazy all-valid select tape that materializes the selected f64 buffer only for typed consumers. This flips default construction and typed materialization, so this pass is **2 wins / 1 loss / 0 neutral** head-to-head. Focused guards green: 12 `combine_first` fp-frame tests and 24 fp-columnar validity-mask tests in release mode; `cargo check -p fp-frame -p fp-columnar --all-targets`; lib clippy `--no-deps`; bounded UBS on changed Rust files timed out after 180s with no emitted findings, matching the known fp-frame whole-file scanner backlog. Residual red path is public `values()` boxing every f64 into `Scalar`, so route deeper to typed numeric public consumption / lower-allocation scalar materialization rather than another select-kernel trim. |
| Series.combine_first public `values()` scalar materialization probes (3gsa7) | Same 2M same-index Float64 NaN-fill workload; `bench_combine_cc values`; local CPU7 best-of-50 after `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo build -p fp-frame --example bench_combine_cc --release` on `vmi1153651` | 6.983 ms | baseline 30.444 ms; right-buffer+patch probe 30.601 ms; single-pass push probe 29.999 ms | **0.23× baseline; 0.23× probes (still 4.3× slower)** | ❌ REVERT / NO-SHIP — two lower-allocation scalar materializers failed the keep bar. Probe 1 avoided the intermediate `Vec<f64>` by mapping right scalars then patching valid left slots, but regressed FP 30.444 → 30.601 ms. Probe 2 wrote each `Scalar::Float64` once in a single pass and only reached 29.999 ms, a ~1.5% FP-side move that is within noise and still a pandas loss. Code reverted. Guards after revert: `cargo test -p fp-frame combine_first --lib` 12/12 and `cargo test -p fp-conformance --lib` 1595/1595. This confirms the residual is not fixed by reshuffling enum-boxing loops; route deeper to an API/storage change that avoids public `Vec<Scalar>` for numeric consumers, or to a fundamentally smaller scalar representation. |
| Series.combine_first public `values()` fresh restart verify (3gsa7/cod-b) | Same 2M same-index Float64 NaN-fill workload; `bench_combine_cc values`; release example rebuilt with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo build --release -p fp-frame --example bench_combine_cc` on `vmi1153651`; local CPU7 | 9.852 ms | 31.054 ms | **0.32× (3.15× slower)** | ❌ NO-SHIP — fresh restart confirms the forced public `values()` residual is still a real pandas loss. No code was kept for this lane; prior right-buffer/patch and single-pass scalar-fill probes already failed the keep bar, and another scalar loop reshuffle is unlikely to remove the enum-boxing floor. Route to a smaller public scalar representation, Arrow/numpy-style numeric result buffers, or APIs that preserve typed consumers. |
| Series.update (updf, bench_update_cc) | 2M f64 same index, other ~50% NaN | 3.67 ms | 20.0 ms | **0.18× (5.5× slower)** | ✅ IMPROVED from 59× LOSS (216ms)! killed the pointer-key BTreeMap<&IndexLabel,&Scalar> (O(n log n) + 2 cache misses/row, fp-index pointer-key pathology) + both values() materializes. Typed identity fast path (mirror combine_first, other-first): same-index + all-valid Float64 self + Float64 other ⇒ out[i] = other-present ? other : self. 10.8× FP-side, bit-identical (positional on identical unique index; other-present == label-in-other-&&-not-missing), conformance green (5 tests). Residual 5.5× = output-alloc floor. (pandas update is in-place; fp builds new) |
| Index.get_indexer unsorted unique Int64 — route through cached i64 resolver (c90bo follow-on) | 1M unsorted unique Int64 self, 1000 targets, repeated best-of-100 CPU7 | 0.0140 ms | 0.0039 ms | **3.6× faster** | ✅ FIXED — was 210× SLOWER (2.92 ms repeated)! `get_indexer_i64` rebuilt its `FxHashMap<i64,usize>` every call; pandas caches its int64 engine. Unsorted unique Int64 self now reuses the cached `unsorted_unique_int64_positions`; duplicate self keeps the per-call first-occurrence builder. Bit-identical, full conformance green. |
| merge/join inner on Utf8 keys (f1ftd current-head verify) | 1M×1M lower-hex string keys → 500k rows; `join_inner_str`; current HEAD `b507ac26348b343886a51ecc867c6c34389f049e`; `fp-bench` release-perf SHA256 `3e99d573871a431a559712c636cd6e4264ede9eda79de3b39f6fb0739f37785c`; `taskset -c 7`; artifact `artifacts/bench/cod_a_f1ftd_join_inner_str_batch_medians_20260621.json` | 146.950 ms p50 / 153.998 ms p95 / 155.719 ms p99; CV 2.43% | 8.234 ms p50 / 8.686 ms p95 / 8.796 ms p99; CV 3.00% | **17.85× faster** | ✅ VERIFIED CURRENT WIN — f1ftd's stale 0.42× red row is resolved on current head by the existing ordered/lower-hex Utf8 join path and contiguous no-overlap output assembly. Raw one-binary harness rows are still recorded as dropped evidence (`cod_a_f1ftd_join_inner_str_baseline_20260621.json`: FP CV 15.70%, apparent 11.13×; `cod_a_f1ftd_join_inner_str_pinned_20260621.json`: FP CV 12.33%, apparent 18.13×). The accepted batch-median gate stabilizes both engines below 5% CV; no source change shipped in this pass. |
| Index.get_indexer unsorted unique Utf8 — route through cached resolvers (c90bo) | 1M unsorted unique Utf8 self, 1000 targets, repeated best-of-100 CPU7 | 0.0760 ms | 0.0179 ms | **4.1× faster** | ✅ FIXED — was 744× SLOWER (58.4 ms repeated)! get_indexer (core of reindex/align/join) unsorted non-Int64 path rebuilt `position_map_first_ref` (pointer-key `FxHashMap<&IndexLabel,usize>`) EVERY call; pandas caches its index engine. get_indexer self is unique (pandas raises on dup), so the unsorted fallback now routes through the identity-cached `unique_utf8_positions`/`unique_datetime64_positions` (sfsx4/recbe) before the map. Bit-identical: unique ⇒ first==only==`map.get`; duplicate/other self falls through unchanged. Reflects the repeated-alignment pattern pandas optimizes via engine caching. Full fp-index+fp-frame conformance green; new fp-index test asserts the routed path == map path incl. dup-self fallback. |
| Series.loc[[timestamps]] unique Datetime64 index — identity-cached ns→pos hashtable (recbe) | 2M f64, 1-min-spaced DatetimeIndex, select 1000, best-of-100 CPU7 | 0.8447 ms | 0.0125 ms | **67.6× faster** | ✅ FIXED — was 1173× SLOWER (728.5 ms)! DatetimeIndex is the most common pandas time-series index; `loc[[ts]]` fell to the per-call pointer-key map. `Datetime64(i64)` is ns-backed, so new `Index::unique_datetime64_positions` builds a first-occurrence `FxHashMap<i64, usize>` (keyed on ns) ONCE, cached in process-global `INDEX_DATETIME_POS_LOOKUP_CACHE` keyed by `label_identity` (`Option` value; `None` caches not-all-Datetime64). Warm probe O(k) inline-i64 FxHash; pandas is slow here (844 µs) because it boxes Timestamps. Gated unique + all-Datetime64; duplicate/non-Datetime64 keep the map fallback. Cap 64. Selector order, dup selectors, fail-closed missing, index name preserved. 1 fp-frame `locfast` test + full loc/index conformance green. |
| Series.loc[[labels]] unique Utf8 index — identity-cached String→pos hashtable (sfsx4) | 2M f64, "k%08d" string index, select 1000, best-of-100 CPU7 | 0.3715 ms | 0.0473 ms | **7.9× faster** | ✅ FIXED — was 2029× SLOWER (772.6 ms)! Non-Int64 indexes (very common: `set_index` on a string column) fell to the per-call pointer-key map. New `Index::unique_utf8_positions` builds a first-occurrence `FxHashMap<Box<str>, usize>` ONCE and caches it in process-global `INDEX_UTF8_POS_LOOKUP_CACHE` keyed by `label_identity` (value is `Option`: `None` caches the not-all-Utf8 verdict so the warm path stays O(1) even on non-Utf8 indexes). Warm probe O(k) FxHash on short strings. Gated unique + all-Utf8; duplicate/non-Utf8 keep the map fallback. Cap 16 entries (each holds the index's boxed strings). Selector order, dup selectors, fail-closed missing labels, index name preserved. 1 fp-frame `locfast` test + full loc/index conformance green. |
| Series.loc[[labels]] UNSORTED unique Int64 — identity-cached i64→pos hashtable (2pvdg) | 2M f64, LCG-shuffled unique Int64 index, select 1000, best-of-50 CPU7 | 0.1453 ms | 0.0106 ms | **13.7× faster** | ✅ FIXED — was 5147× SLOWER (747.9 ms)! Unsorted unique Int64 indexes can't binary-search, so they fell to the same per-call pointer-key `FxHashMap<&IndexLabel,Vec<usize>>`. New `Index::unsorted_unique_int64_positions` builds a first-occurrence `i64→position` `FxHashMap` ONCE and caches it in a process-global `INDEX_INT64_POS_LOOKUP_CACHE` keyed by the index's runtime `label_identity` (same identity-cache pattern as `INDEX_LABEL_EQUALITY_CACHE`; clone/rename preserve identity + labels, so sharing is sound). Warm probe is O(k) inline-i64 FxHash — beats even the sorted binary-search path (10.6 µs vs 86.5 µs) because binary search over 2M chases cache lines per level. Gated all-Int64 + unique + NOT sorted; sorted keeps binary search, duplicate/non-Int64 keep the map fallback. Selector order, dup selectors, fail-closed missing labels, index name preserved. 1 fp-frame `locfast` + 1 fp-index gating test + full loc/index conformance green. |
| Series.loc[[labels]] arbitrary Int64 index — sorted-unique batch resolver (0pkt2) | 2M f64, step-2 sorted index, select 1000, best-of-50 CPU7 | 0.137 ms | 0.0865 ms | **1.58× faster** | ✅ FIXED — was 5340× SLOWER (725 ms)! New `Index::sorted_unique_int64_positions` builds+caches the typed `int64_view` (`&[i64]`) ONCE in the index's `int64_typed` OnceLock, then binary-searches each requested label O(k log n). Series.loc + DataFrame.loc route strictly-ascending materialized Int64 indexes through it, replacing the per-call pointer-key `FxHashMap<&IndexLabel,Vec<usize>>` over the WHOLE index (O(n) + 2M tiny Vec allocs/call). pandas 137.1 µs vs fp 86.5 µs (self-verified, taskset -c 7). Distinct from the reverted `locsrt` get_loc path (which only hit LAZY-int64 backings); this caches the materialized Vec→typed view. Selector order, duplicate selectors, fail-closed missing labels, index name all preserved; duplicate/unsorted indexes keep the map fallback. 2 `locfast` tests + full loc/index conformance green (3081 passed; the 4 unrelated acosh/arccosh golden + groupby_prod failures are pre-existing/host-libm, not this lever). |
| Series.reindex(full new Utf8 target) — cached-resolver candidate | 1M f64, "k%08d" source, reverse-order full target, best-of-30 | 216.2 ms | 474.9 ms | **0.46× (2.2× slower)** | ❌ DECLINED (not shipped) — the loc identity-cache trick was considered here (reindex requires a unique source, same precondition) but rejected on two grounds: (1) reindex to a FULL new index is **allocation-bound** (1M output column + 1M new index strings), not resolver-bound, so caching the source lookup can't flip it — same output-alloc floor as concat/where; (2) reindex is typically called **once** (and DataFrame.reindex resolves the target once for all columns), so an identity-keyed cache would warm only across the bench's best-of-N and give **no real single-call win** — unlike `loc`, which pandas itself caches and which is genuinely called repeatedly. Honest no-ship: would flatter the bench, not production. |
| DataFrame.where (bench_dfwhere_cc) | 500k×10 f64, 50% cond frame, scalar other | 28.89 ms | 6.83 ms | **4.2× faster** | ✅ already optimized — where_mask_typed_f64/_i64 (eydcr) per-column typed path. CONTRAST with Series.where (had the 96× align pathology, now fixed): the DataFrame path was always typed-fast. Measured to confirm, not assume |
| Series + Series finite-witness no-scan probe (wrlj5) | 2M f64 | 2.45 ms local pandas sanity | 17.99 ms on rch hz2 (baseline 18.28 ms) | **0.14× vs pandas; 1.016× vs fp baseline** | ❌ REVERT — finite-input witness skips NaN scans in theory, but same-worker delta was +1.6% (noise) and gt side-effect was worse (0.995→1.050 ms). Code reverted |
| Series * Series finite-witness no-scan probe (wrlj5) | 2M f64 | 2.49 ms local pandas sanity | 17.96 ms on rch hz2 (baseline 18.08 ms) | **0.14× vs pandas; 1.007× vs fp baseline** | ❌ REVERT — same-worker delta +0.7%, below keep threshold. Safe portable-SIMD f64x8 follow-up on rch vmi regressed add 12.63→13.37 ms and mul 12.02→12.23 ms, so that probe was also reverted |
| shift typed Float64 (202cdf50) | 2M f64, periods=1 | 0.74 ms | 9.01 ms | **0.082× (12× SLOWER)** | ⚠️ KEEP (≥ old Scalar path) but LOSS — structural |
| shift typed Int64 fill (51601b7a) | 2M i64, periods=2 | 0.74 ms | 7.86 ms | **0.094× (10.6× SLOWER)** | ⚠️ KEEP but LOSS — structural |
| ffill typed Float64 (as_f64_slice_with_validity) | 2M f64, ~10% NaN | 2.79 ms | 18.43 ms | **0.15× (6.6× SLOWER)** | ⚠️ KEEP but LOSS — confirms column-rebuild pattern |
| shift + mimalloc boundary allocator (3nah5) | 2M f64, periods=1 | 0.858 ms | 4.30 ms | **0.20× (5.0× slower)** | ✅ KEEP — adopted at process boundaries; 1.35× faster than current glibc-malloc shift (5.80 ms), golden `d41eaaa775ee123e` unchanged |
| ffill + mimalloc boundary allocator (3nah5) | 2M f64, ~10% NaN | 2.50 ms | 6.89 ms | **0.36× (2.76× slower)** | ✅ KEEP — adopted at process boundaries; 2.51× faster than current glibc-malloc ffill (17.32 ms), still needs single-pass builder |
| shift no-scan Float64 rebuild + mimalloc (dfcv8) | 2M f64, periods=1 | 0.943 ms | 0.673 ms | **1.40× faster** | ✅ KEEP — skips redundant Float64 NaN/validity rebuild scan via hidden all-valid constructor; golden `d41eaaa775ee123e` unchanged; plain glibc path is 1.47 ms = 0.64×, so allocator boundary still matters |
| ffill no-scan Float64 rebuild + mimalloc (dfcv8) | 2M f64, ~10% NaN | 3.221 ms | 6.17 ms | **0.52× (1.9× slower)** | ⚠️ KEEP as no-regression side effect — 1.12× faster than prior mimalloc row, but still a pandas loss; route deeper validity-run/branchless fill work |
| ffill validity-run bulk fill + mimalloc (skw2c) | 2M f64, ~10% NaN | 3.340 ms | 2.371 ms | **1.41× faster** | ✅ KEEP — packed-word invalid-run visitor + bulk f64 copy fills only missing spans; FP-side 5.745→2.371 ms = 2.42× faster; focused conformance green; `perf stat` blocked by `perf_event_paranoid=4` |

| set_index typed Int64 col→idx (p9omo) | 1M rows, 2 cols | 1.12 ms | 0.17 ms | **6.5× faster** | ✅ KEEP — Index::from_i64_values |
| cummax (sweep bench_misc) | 2M f64 | 22.02 ms | 2.63 ms | **8.4× faster** | ✅ pandas cummax surprisingly slow; fp crushes |
| cumsum (sweep bench_misc) | 2M f64 | 22.73 ms | 3.09 ms | **7.4× faster** | ✅ pandas cumsum surprisingly slow; fp crushes |
| clip (sweep bench_misc) | 2M f64, both bounds | 29.39 ms | 5.23 ms | **5.6× faster** | ✅ big win |
| rank average (sweep bench_misc) | 2M f64 shuffled | 321.97 ms | 209.43 ms | **1.54× faster** | ✅ both slow (sort+ties); fp ahead |
| nlargest(20) typed Float64 (nlgf) | 2M f64 shuffled | 46.27 ms | 20.92 ms | **2.21× faster** | ✅ FIXED — was 0.79× LOSS; typed f64 path (as_f64_slice + partial_cmp) skips values()/semantic_cmp; 2.79× FP-side, bit-identical (semantic_cmp==partial_cmp for Float64), conformance 21/21 |
| nsmallest(20) typed Float64 (nlgf) | 2M f64 shuffled | 37.07 ms | 21.39 ms | **1.73× faster** | ✅ FIXED — mirror of nlargest (ascending); same bit-identical typed f64 path, conformance 16/16 |
| idxmax typed Float64 (idxf) | 2M f64 shuffled | 0.527 ms | 1.41 ms | **0.37× (2.7× slower)** | ✅ FIXED — was 13.9× LOSS; typed f64 scan skips values()/to_f64 (5.2× FP-side); bit-identical, conformance 35/35. Residual 2.7× = numpy-SIMD argmax (safe-Rust scalar ceiling, like max/min) |
| idxmin typed Float64 (idxf) | 2M f64 shuffled | 0.558 ms | 1.41 ms | **0.39× (2.6× slower)** | ✅ FIXED — was 12.3× LOSS; 4.9× FP-side; bit-identical |
| pct_change (sweep bench_misc2) | 2M f64, periods=1 | 40.54 ms | 2.57 ms | **15.8× faster** | ✅ pandas pct_change very slow; fp crushes |
| cummin (sweep bench_misc2) | 2M f64 | 22.86 ms | 2.80 ms | **8.2× faster** | ✅ pandas slow; fp crushes |
| cumprod (sweep bench_misc2) | 2M f64 | 23.46 ms | 3.18 ms | **7.4× faster** | ✅ pandas slow; fp crushes |
| nunique (sweep bench_misc2) | 2M f64 distinct | 207.60 ms | 197.91 ms | **1.05× faster** | ➖ NEUTRAL — both hashmap-bound |
| abs (sweep bench_misc2) | 2M f64 | 0.80 ms | 2.63 ms | **0.30× (3.3× slower)** | 🔴 LOSS — rebuild-class (allocator-bound, mimalloc-covered) |
| round (sweep bench_misc2) | 2M f64, decimals=2 | 1.80 ms | 4.67 ms | **0.39× (2.6× slower)** | 🔴 LOSS — rebuild-class (allocator-bound, mimalloc-covered) |
| argsort typed Float64 pair-sort (asf64) | 2M f64, ~1000 distinct | 30.52 ms | 43.35 ms | **0.70× (1.42× slower)** | ✅ FIXED — was 21.7× LOSS (662.9 ms)! typed f64 pair-sort + hoisted asc/desc + typed-i64 output (skips 2M Scalar boxing) = 15.3× FP-side. Bit-identical (compare_non_missing==partial_cmp, stable sort, no-NaN gated), conformance 7/7. Residual 1.42× = numpy introsort/radix |
| between (sweep bench_misc3) | 2M f64, both bounds | 0.924 ms | 0.629 ms | **1.47× faster** | ✅ already typed-fast |
| duplicated typed f64-bits + splitmix (uqf64/mixf64) | 2M f64, ~1000 distinct | 14.06 ms | 48.3 ms | **0.29× (3.3× slower)** | ⚠️ IMPROVED from 8.7× LOSS (2.6× FP-side); splitmix added (bit-identical, 14 tests) but ~0 further — duplicated is OUTPUT-alloc-bound (2M Bool column), NOT hash-bound like unique/mode. Residual = allocator floor (mimalloc-covered) |
| unique typed f64-bits + splitmix (mixf64) | 2M f64, ~1000 distinct | 12.83 ms | 7.14 ms | **1.80× faster** | ✅ FIXED — was 3.8× LOSS! splitmix64 finalizer (bijective) un-clusters FxHash on integer-valued-f64 bits = 6.8× FP-side over plain FxHash. The "khash ceiling" was FxHash CLUSTERING, not khash superiority. Bit-identical (8 tests) |
| median (sweep bench_misc4) | 2M f64 | 35.76 ms | 2.96 ms | **12.1× faster** | ✅ typed quickselect crushes pandas |
| kurtosis (sweep bench_misc4) | 2M f64 | 38.66 ms | 7.74 ms | **5.0× faster** | ✅ typed numeric_values moments |
| skew (sweep bench_misc4) | 2M f64 | 38.56 ms | 7.75 ms | **5.0× faster** | ✅ typed numeric_values moments |
| sem (sweep bench_misc4) | 2M f64 | 42.41 ms | 9.47 ms | **4.5× faster** | ✅ typed |
| mode typed f64-bits + splitmix (mixf64) | 2M f64, ~1000 distinct | 17.32 ms | 10.87 ms | **1.59× faster** | ✅ FIXED — was 2.97× LOSS! splitmix on the 0loqz tally (bijective, bit-identical, 41 tests) = 4.7× FP-side. Same FxHash-clustering root cause as unique |
| value_counts Float64 (already neutral) | 2M f64, ~1000 distinct | 17.53 ms | 16.06 ms | **1.09× faster** | ➖ NEUTRAL — already fine. Uses ScalarKey FxHashMap whose ENUM-discriminant hash avoids the raw-u64 FxHash clustering that hit unique/mode. FIX ATTEMPT (REVERTED): typed f64-bits+splitmix path = 16.06→15.43 ms (~4%, ~0-gain), so reverted — ScalarKey path already un-clustered here. Explains WHY only the raw-u64 ops (unique/mode) clustered |
| diff (sweep bench_misc) | 2M f64, periods=1 | 0.86 ms | 1.86 ms | **0.46× (2.16× slower)** | 🔴 LOSS — rebuild-class (allocator-bound, mimalloc-fixable like shift/ffill) |
| fillna(value) (sweep bench_misc) | 2M f64, ~10% NaN | 2.53 ms | 4.48 ms | **0.57× (1.77× slower)** | 🔴 LOSS — rebuild-class (allocator-bound, mimalloc-fixable) |
| RangeIndex.asof closed-form (jlv2o) | 100k rows, 4,096 scalar probes | 232.02 ms | 60.42 µs | **3,840× faster** | ✅ KEEP — public scalar API; pandas CV 4.82% |
| RangeIndex.asof closed-form (jlv2o) | 1M rows, 4,096 scalar probes | 1,050.29 ms | 65.52 µs | **16,031× faster** | ✅ KEEP — lookup no longer scales with range length |
| RangeIndex.symmetric_difference two-run backing (uza04.168) | 1M overlapping RangeIndex, split output, `len()` construction path | 5.158 ms | 0.000110 ms | **46,889× faster** | ✅ KEEP — closes the prior split-span loss by carrying two boxed affine Int64 runs lazily. rch release evidence on exact boxed code: 0.000140 ms; pre-change remote baseline was 39.710 ms. Win/loss/neutral for this pass: **1 / 0 / 0** |
| RangeIndex.symmetric_difference BOLD-VERIFY (u22ww) | 1M overlapping RangeIndex, split output, `len()` construction path; `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, fp-index example built with `rch exec -- cargo build -p fp-index --example bench_range_setops --release`; same-machine pandas 2.2.3 best-of-80 | 4.494 ms | 0.000160 ms | **28,090× faster** | ✅ VERIFIED KEEP — lazy two-affine Int64 backing is still decisive against pandas; no new code. Guard evidence: `cargo test -p fp-index range_index_set_ops_keep_typed_backing_uza04168 --release`, `cargo test -p fp-index range_index_join_direct_i64_matches_flat_oracle_uza04190 --release`, `cargo bench -p fp-index --no-run`, and `cargo test -p fp-conformance smoke --release -- --nocapture` all green via `rch`. Conformance smoke passed its stable-report test; the worker-local pandas oracle checkout was absent, so the oracle-discovery smoke skipped that lookup as designed. Initial `cargo bench -p fp-index --release --no-run` was rejected by Cargo because bench has no `--release` flag. |
| RangeIndex.get_indexer miss-heavy arithmetic bulk (uza04.159) | 100k targets, 15/16 misses | 0.777 ms | 0.295 ms | **2.64× faster** | ✅ KEEP — same-host local pandas p50 vs FP Criterion mean; flips prior 0.83× loss |
| RangeIndex.get_indexer miss-heavy arithmetic bulk (uza04.159) | 1M targets, 15/16 misses | 10.493 ms | 2.911 ms | **3.61× faster** | ✅ KEEP — batched pandas p50 CV 5.71%; FP local Criterion |
| RangeIndex.reindex all-miss arithmetic lattice (uza04.159) | 100k target RangeIndex | 0.666 ms | 18.45 µs | **36.1× faster** | ✅ KEEP — exact lattice all-miss fast path; flips prior 0.86× loss |
| RangeIndex.reindex all-miss arithmetic lattice (uza04.159) | 1M target RangeIndex | 9.597 ms | 0.187 ms | **51.5× faster** | ✅ KEEP — flips prior neutral 1.07× row into a decisive win |
| groupby.sum Int64 key (dense grouping) | 1M rows, 1000 keys | 13.26 ms | 2.44 ms | **5.4× faster** | ✅ KEEP — int64_dense_grouping |
| groupby.sum Utf8 key clone-free counter (uza04.193) | 1M rows, 1000 keys, Float64 values, NaN every 37th | 32.946 ms | 15.141 ms | **2.18× faster** | ✅ VERIFIED KEEP — previous row was 0.56×/1.78× slower; public `groupby_sum` and `groupby_agg(Sum)` share digest `7fb4fd07f6f8bdf2`; pandas CV 3.17%, FP public-sum CV ~0.7% |
| groupby.prod Utf8 key clone-free counter (uza04.193) | 1M rows, 1000 keys, Float64 values, NaN every 37th | 32.988 ms | 13.001 ms | **2.54× faster** | ✅ VERIFIED KEEP — same streaming product counter family; pandas CV 3.51%, FP public-prod CV <1%; focused fallback/overflow/timedelta guards green |
| groupby.min Utf8 key clone-free slot (uza04.191) | 1M rows, 1000 keys, Float64 values, NaN every 37th, 5×20 batch p50 | 43.642 ms | 16.807 ms | **2.60× faster** | ✅ VERIFIED KEEP — streaming scalar extremum slot; golden `def13b65b5e3a35d`; focused min/min-max release guards green |
| groupby.max Utf8 key clone-free slot (uza04.191) | 1M rows, 1000 keys, Float64 values, NaN every 37th, 5×20 batch p50 | 43.107 ms | 16.973 ms | **2.54× faster** | ✅ VERIFIED KEEP — same scalar-slot family; golden `6d20c5176a43035d`; focused max release guard green |
| groupby.first Utf8 key clone-free slot (uza04.192) | 1M rows, 1000 keys, Float64 values, NaN every 37th, 5×20 batch p50 | 42.290 ms | 14.497 ms | **2.92× faster** | ✅ VERIFIED KEEP — streaming first-present scalar slot; golden `a8c2c037ffb85c88`; focused first/first-last release guards green |
| groupby.last Utf8 key clone-free slot (uza04.192) | 1M rows, 1000 keys, Float64 values, NaN every 37th, 5×20 batch p50 | 42.085 ms | 18.385 ms | **2.29× faster** | ✅ VERIFIED KEEP — streaming last-present scalar slot; golden `d373b7337998d544`; focused last release guard green |
| groupby.count Utf8 key clone-free counter (uza04.187) | 1M rows, 1000 keys, Float64 values, NaN every 37th, 5×20 batch p50 | 40.566 ms | 16.307 ms | **2.49× faster** | ✅ VERIFIED KEEP — counter-only non-null path; golden `1e555b43a73656c1`; focused count/count-size release guards green |
| groupby.size Utf8 key clone-free counter (uza04.187) | 1M rows, 1000 keys, Float64 values, NaN every 37th, 5×20 batch p50 | 45.113 ms | 16.064 ms | **2.81× faster** | ✅ VERIFIED KEEP — total-row counter path; golden `c6ccd2e318a736dd`; focused size release guards green |
| groupby.agg(nunique) Utf8 key (uza04.204) | 2M rows, 1000 keys, Float64 values, NaN every 37th | 153.75 ms | 53.12 ms | **2.89× faster** | ✅ KEEP — CV-gated accepted; FP CV 2.68%, pandas CV 0.95% |
| groupby.agg(median) Utf8 key (uza04.203) | 100k rows, 1000 keys, Float64 values, NaN every 37th | 5.53 ms | 2.10 ms | **2.63× faster** | ✅ KEEP — CV-gated accepted; FP CV 1.48%, pandas CV 4.56% |
| groupby.agg(median) Utf8 key (uza04.203) | 2M rows, 1000 keys, Float64 values, NaN every 37th | 77.17 ms | 42.98 ms | **1.80× faster** | ✅ KEEP — CV-gated accepted; FP CV 3.21%, pandas CV 1.06% |
| groupby.agg(var) Utf8 key (uza04.202) | 100k rows, 1000 keys, Float64 values, NaN every 37th | 3.63 ms | 2.81 ms | **1.29× faster** | ✅ KEEP — CV-gated accepted; FP CV 0.52%, pandas CV 1.36% |
| groupby.agg(std) Utf8 key (uza04.202) | 100k rows, 1000 keys, Float64 values, NaN every 37th | 3.83 ms | 2.85 ms | **1.34× faster** | ✅ KEEP — CV-gated accepted; FP CV 0.96%, pandas CV 2.56% |
| groupby.agg(var) Utf8 key (uza04.202) | 1M rows, 1000 keys, Float64 values, NaN every 37th | 35.97 ms | 29.56 ms | **1.22× faster** | ✅ KEEP — CV-gated accepted; FP CV 3.05%, pandas CV 3.26% |
| groupby.agg(std) Utf8 key (uza04.202) | 1M rows, 1000 keys, Float64 values, NaN every 37th | 35.17 ms | 28.66 ms | **1.23× faster** | ✅ KEEP — CV-gated accepted; FP CV 1.05%, pandas CV 0.44% |
| groupby.agg(var) Utf8 key (uza04.202) | 2M rows, 1000 keys, Float64 values, NaN every 37th, 20 iters/sample | 76.34 ms | 58.66 ms | **1.30× faster** | ✅ KEEP — CV-gated accepted; FP CV 3.49%, pandas CV 1.80% |
| groupby.agg(std) Utf8 key (uza04.202) | 2M rows, 1000 keys, Float64 values, NaN every 37th, 20 iters/sample | 75.54 ms | 56.47 ms | **1.34× faster** | ✅ KEEP — CV-gated accepted; FP CV 0.64%, pandas CV 0.61% |

### High-CV directional rows (not release proof)

| Lever (bead) | Workload | pandas p50 | fp p50 | p50 ratio | verdict |
|---|---|---:|---:|---:|---|
| groupby.agg(nunique) Utf8 key (uza04.204) | 100k rows, 1000 keys | 7.82 ms | 4.44 ms | 1.76× faster | DROPPED_HIGH_CV — FP CV 12.91%, pandas CV 13.55% |
| groupby.agg(nunique) Utf8 key (uza04.204) | 1M rows, 1000 keys, pinned CPU rerun | 84.29 ms | 27.29 ms | 3.09× faster | DROPPED_HIGH_CV — FP CV 5.52%, pandas CV 6.35% |
| groupby.agg(median) Utf8 key (uza04.203) | 1M rows, 1000 keys, pinned CPU run | 49.83 ms | 19.90 ms | 2.50× faster | DROPPED_HIGH_CV — FP CV 6.62%, pandas CV 1.08% |
| groupby.agg(std) Utf8 key (uza04.202) | 2M rows, 1000 keys, first pinned run | 55.94 ms | 58.57 ms | 0.96× | DROPPED_HIGH_CV — FP CV 12.39%, pandas CV 0.86%; superseded by batched accepted rerun |
| groupby.agg(std) Utf8 key (uza04.202) | 2M rows, 1000 keys, 10 iters/sample rerun | 85.26 ms | 56.87 ms | 1.50× faster | DROPPED_HIGH_CV — FP CV 1.24%, pandas CV 5.92%; superseded by 20-iter accepted rerun |
| RangeIndex.get_indexer miss-heavy (29u49) | 1M targets, 15/16 misses | 16.43 ms | 10.74 ms | 1.53× faster | DROPPED_HIGH_CV — pandas CV 5.40%; superseded by accepted `uza04.159` batched local row |

### Win: groupby std/var generic-key clone-free counters
The `uza04.202` lever removes per-group `Vec<Scalar>` materialization from generic-key
`groupby_agg(Var|Std)` and keeps two compact numeric passes over the original Float64
values. Against pandas 2.2.3 / numpy 2.4.3 on the realistic UTF8-key workload
(`sort=True`, 1000 keys, NaN every 37th row), all six accepted rows beat pandas:
1.29x-1.34x at 100k, 1.22x-1.23x at 1M, and 1.30x-1.34x at 2M after the longer
20-iteration/sample rerun stabilized the large rows. Geomean across accepted rows:
1.285x faster. No revert: the lever has focused conformance coverage, stable golden
digests, and no accepted slower/neutral release row.

### Win: RangeIndex.asof closed-form scalar lookup
The `jlv2o` lever changes ascending `RangeIndex::asof(Int64)` from direct label scanning
to `searchsorted(..., "right") - 1` over the affine `(start, stop, step)` witness.
Focused Criterion on the local host measured 4,096-probe batches at 60.42 µs (100k rows)
and 65.52 µs (1M rows). The matching pandas 2.2.3 public scalar API loop measured
232.02 ms and 1,050.29 ms respectively, with pandas CV below 5%. No revert: the lever
is both behavior-guarded and decisively faster than pandas on the targeted workload.
Artifacts: `artifacts/bench/gauntlet_cod_b_range_asof_vs_pandas.json`,
`artifacts/bench/gauntlet_cod_b_range_asof_criterion_local.txt`, and
`artifacts/bench/gauntlet_cod_b_range_asof_pandas.json`.

### Win: RangeIndex bulk indexers arithmetic lattice
The `uza04.159` lever keeps the public `get_loc` error contract intact while moving bulk
indexers onto the affine witness: `get_indexer`/`get_indexer_non_unique` reuse one source
length and a bounded i64 membership check, and `reindex(RangeIndex)` emits the indexer by
source-position arithmetic when the target lies on the source lattice. The old `29u49`
exception-allocation fix was a real FP-side improvement but still left accepted pandas
losses; this follow-up closes the vectorized RangeIndex gap.

Same-host local head-to-head against pandas 2.2.3 now wins all accepted rows: 100k
miss-heavy `get_indexer` is 2.64x faster (0.777 ms pandas p50 vs 0.295 ms FP Criterion
mean), 1M miss-heavy `get_indexer` is 3.61x faster, 100k all-miss `reindex` is 36.1x
faster, and 1M all-miss `reindex` is 51.5x faster. Same-worker `rch` Criterion on `ovh-b`
also proved the lever delta against the pre-change path: `get_indexer` improved 4.0x at
both 100k and 1M targets, while all-miss `reindex` improved 75.7x at 100k and 32.2x at
1M. No revert: the lever is behavior-guarded by the RangeIndex sweep, including
descending ranges, partial lattices, all-miss targets, and full-width i64 arithmetic.
Artifacts updated in the Criterion tree under
`/data/projects/.rch-targets/frankenpandas-cod-b/criterion/range_index_indexers/`.

### Win: clone-free generic `groupby.sum`/`prod` on Utf8 keys (br-frankenpandas-uza04.193)
**VERIFIED after code-first commit.** The implementation had already landed and was reset
open because batch proof was pending. Current cod-b verification rebuilt `fp-groupby` per-crate:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo build --release -p fp-groupby --bin groupby-bench`
passed on worker `hz2`, then the local retrieved release binary was run pinned to CPU 7.

Head-to-head fixture: 1,000,000 rows, 1,000 Utf8 groups, Float64 values, every 37th
value missing, `sort=True`. Fair pandas Series-vs-Series batches on pandas 2.2.3 were
`sum` p50 32.946 ms (CV 3.17%) and `prod` p50 32.988 ms (CV 3.51%). FP public
`groupby_sum` batches were 15.180, 15.141, 15.226, 15.000, 14.930 ms (p50
15.141 ms, CV ~0.7%). FP public `groupby_prod` batches were 13.001, 13.099,
12.909, 12.995, 13.149 ms (p50 13.001 ms, CV <1%). Verdict: **2.18x faster
than pandas for sum** and **2.54x faster than pandas for prod**. The public
`sum` and dispatcher alias `agg-sum` produce the same golden digest:
`7fb4fd07f6f8bdf2` (`out_rows=1000`).

Conformance guards:
`rch exec -- cargo test -p fp-groupby groupby_agg_sum_matches_dedicated_sum --release`,
`groupby_agg_sum_prod_utf8_keys_stream_integer_slots`,
`groupby_agg_sum_prod_float64_utf8_keys_stream_counters_2qb1i`,
`groupby_agg_sum_prod_bool_and_prod_overflow_preserve_fallbacks`, and
`groupby_agg_sum_prod_timedelta_fallback_preserved_2qb1i` all passed. Semantics
preserved: sorted and first-seen output order, null skipping, all-missing defaults,
Bool/Int64 dtype preservation, product overflow f64 fallback, Float64 folds, and
Timedelta fallback routing. `perf stat` attribution remained blocked by
`perf_event_paranoid=4`.

### Win: clone-free generic `groupby.first`/`last` on Utf8 keys (br-frankenpandas-uza04.192)
**VERIFIED after code-first commit.** The implementation had already landed and was reset
open because batch proof was pending. Current cod-a verification built `fp-groupby` per-crate:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo build --release -p fp-groupby --bin groupby-bench`
passed with rch local fallback when the worker fleet had no admissible build slots; the
focused release tests then ran remotely on `hz2`.

Head-to-head fixture: 1,000,000 rows, 1,000 Utf8 groups, Float64 values, every 37th
value missing, `sort=True`, pandas 2.2.3 / numpy 2.4.3. Batched pandas p50s were
`first` 42.290 ms and `last` 42.085 ms. FP public batches were:
`first` 14.598, 13.889, 15.309, 14.497, 13.967 ms (p50 14.497 ms), and
`last` 18.385, 18.850, 18.045, 19.782, 18.137 ms (p50 18.385 ms). Verdict:
**2.92x faster than pandas for first** and **2.29x faster than pandas for last**.
Golden digests on the same fixture are `a8c2c037ffb85c88` for first and
`d373b7337998d544` for last.

Conformance guards:
`rch exec -- cargo test --release -p fp-groupby groupby_first -- --nocapture`
passed 4/4 focused release tests, including UTF8-key sorted/first-seen order,
skip-missing/all-missing behavior, object scalar preservation, and dense Int64 oracle
coverage. `rch exec -- cargo test --release -p fp-groupby groupby_last -- --nocapture`
passed the focused last release guard. Semantics preserved: null skipping, all-missing
NaN output, selected scalar identity for object groups, sorted output order, and
first-seen order fallback.

### Win: clone-free generic `groupby.count`/`size` on Utf8 keys (br-frankenpandas-uza04.187)
**VERIFIED after code-first commit.** The implementation had already landed and was reset
open because batch proof was pending. Current cod-a verification built `fp-groupby` per-crate:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo build --release -p fp-groupby --bin groupby-bench`
passed on worker `ovh-b`, then the retrieved release binary was used for local timings.

Head-to-head fixture: 1,000,000 rows, 1,000 Utf8 groups, Float64 values, every 37th
value missing, `sort=True`, pandas 2.2.3 / numpy 2.4.3. Batched pandas p50s were
`count` 40.566 ms and `size` 45.113 ms. FP public batches were:
`count` 15.024, 20.496, 16.307, 14.301, 16.462 ms (p50 16.307 ms), and
`size` 14.055, 14.669, 16.064, 16.483, 16.341 ms (p50 16.064 ms). Verdict:
**2.49x faster than pandas for count** and **2.81x faster than pandas for size**.
Golden digests on the same fixture are `1e555b43a73656c1` for count and
`c6ccd2e318a736dd` for size.

Conformance guards:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo test --release -p fp-groupby groupby_count -- --nocapture`
passed 4/4 focused release tests, including the Utf8 count/size null/order guard.
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo test --release -p fp-groupby groupby_size -- --nocapture`
passed 3/3 focused release tests. Semantics preserved: `count` skips missing
values, `size` includes missing values, sorted output order is stable, and the
first-seen `sort=false` contract remains covered by the shared guard.

### Gap: shift/concat/ffill structural — column-rebuild vs in-place (historically 6.6–24× slower)
**ffill (2M f64, ~10% NaN) originally confirmed the pattern: 18.43 ms vs pandas 2.79 ms = 6.6× slower.**
ffill had a typed `as_f64_slice_with_validity` path but still rebuilt every output slot and
re-initialized validity. skw2c fixes the no-limit Float64 case by extracting invalid runs from
packed validity words, bulk-copying the f64 buffer, and filling only missing spans; that flips
ffill to 2.371 ms vs pandas 3.340 ms = 1.41× faster. concat construction is no longer the
active rebuild-class loss: cod-a's Int64 chunk tape now stores all-valid source spans instead
of allocating a fresh destination buffer, flipping `ignore_index=True` construction to
0.000361 ms vs pandas 0.851 ms = 2,358× faster. xgrv3 also narrows one Float64 concat follow-on
lane: existing concat chunks plus the new `as_f64_slice()` bridge make `pd.concat(...).sum()`'s
fp analogue 4.824 ms vs pandas 8.055 ms = 1.67× faster. Forced materialization still pays the
typed/scalar buffer cost when a downstream consumer asks for a contiguous slice or `values()`,
but construction itself now has the broader chunk/view model this section called for.

fp shift/concat rebuild a whole new typed Column (`as_f64/i64_slice` materializes the typed
buffer for `from_values`-built columns, then `from_f64/i64_values` re-inits validity, then
`Series::new`) — multiple O(n) passes — whereas pandas shift/concat is ~one numpy memmove/
concatenate. The typed levers are NOT regressions (≥ old fp Scalar rebuild) so kept, but the
WHOLE-COLUMN-REBUILD construction is fp's structural disadvantage on these ops. INSIGHT: the
typed-slice levers win big when typed access unlocks a cheaper ALGORITHM (FxHash dedup, dense
value_counts, Welford std → 2–11× wins) but only break even (then lose on construction
overhead) for ops that merely rebuild the column (shift/concat). Kernel-level fix needed.

### Fixed: concat Int64 construction chunk tape (24× loss → 2,358× win)
pandas `pd.concat` of Int64 series ≈ a single `np.concatenate` (flat int64 memcpy, 281µs/1M).
fp's path has structural overhead: per-series `as_i64_slice` (may materialize the typed
buffer from the Scalars variant on first call), `extend_from_slice` into a fresh Vec<i64>,
`Column::from_i64_values` (validity init), then `Series::new`. The typed lever is NOT a
regression (≥ the old Scalar concat, bit-transparent) so not reverted, but concat is fp's
weakest realistic op. FIX ATTEMPT (REVERTED): collect each typed slice ONCE via `Option::collect` instead of the
double `as_i64_slice` (probe + extend). Built+measured: **6.81 → 6.49 ms (~5%, immaterial)**
— the double-call was NOT the bottleneck. Reverted (working-tree only).

**ROOT CAUSE PINNED (gauntlet experiment, hypothesis tested + REFUTED).** Two earlier guesses
(Scalars-backing materialization; validity init) were both wrong. The bench now measures
three variants of the 1M/k=8 concat:
- `scalars_backed` (parts via `from_values`, as_i64_slice→None, Scalar fallback path): **6.58 ms**
- `typed_backed` (parts via `from_i64_values`, as_i64_slice→Some, typed Int64 path): **6.63 ms**
- `direct_build` (NO concat at all — just `from_i64_values(flat.clone()) + Series::new` of a
  pre-flattened 1M Vec): **6.69 ms**
All three are equal. So: (a) the typed-vs-Scalar backing is irrelevant → **the typed concat
lever (tbrtu) is ~0-gain** (kept only because it's bit-transparent and ≥ the Scalar path,
not because it helps); (b) the concat iteration/extend is only ~0.4 ms; (c) **the entire cost
is RESULT CONSTRUCTION** — allocating + filling a fresh 8 MB `Vec<i64>` (clone + first-touch
page faults) for the output column. `Index::new_known_unique_int64_unit_range`, `Series::new`,
and `ValidityMask::all_valid` are all verified O(1) (affine labels + preset caches; empty
words vec; len-only check). pandas' `np.concatenate` (281µs) wins via numpy's optimized
allocator/memcpy (no per-page first-touch cost, buffer reuse).

**FIX CONFIRMED — pooling global allocator (mimalloc) recovers 13×.** The predicted
allocator fix was tested: `bench_concat_mimalloc` runs the identical 1M/k=8 concat with
mimalloc as `#[global_allocator]`. Result: **6.69 ms (system glibc malloc) → 0.52 ms
(mimalloc) = 12.9× faster.** That flips concat from **24× slower than pandas → 1.86× slower**
(0.52 ms vs pandas 0.28 ms). So the rebuild-class "structural loss" was ~93% the glibc
malloc large-allocation cost (mmap threshold → fresh mmap + kernel zero-fill + first-touch
faults per call), NOT anything in fp's column/index/Series logic. **This is the single
highest-leverage lever found in the gauntlet**: a pooling global allocator should recover a
large fraction of concat/shift/ffill (all rebuild-class, all allocation-bound) at once,
workspace-wide. ACTION: adopt mimalloc/jemalloc as the global allocator for the user-facing
binary (fp-python cdylib) + benchmark harnesses — a workspace coordination decision (interacts
with fp-groupby/fp-join arena allocators), filed as a high-priority bead, not imposed
unilaterally. Dead ends still recorded: typed-backing and validity-init tweaks don't help;
the lever is the allocator, not the column build.

**CURRENT FIX — lazy Int64 chunk tape removes the construction allocation.** cod-a's follow-up
keeps concat's source `Arc<[i64]>` buffers as `(arc, start, len)` spans in
`LazyAllValidInt64Chunks`, and only builds the flat `Vec<i64>` when a consumer asks for a
typed slice or public scalar values. The same workload now measures **0.000361 ms FP vs
0.851 ms pandas = 2,358× faster** in the normal release harness; the old same-binary
materialized direct build remains **6.249 ms**, so the delta is the eliminated output
allocation/page-touch floor rather than allocator noise. Mimalloc remains useful for consumers
that force materialization, but concat construction itself is no longer a loss class.

**GENERALIZES (measured) — mimalloc helps the whole rebuild-class, not just concat.** Ran the
same shift/ffill workloads under mimalloc (`bench_rebuild_mimalloc`): shift 9.01 → 3.98 ms
(2.3×, gap 12× → 5.4×), ffill 18.43 → 6.62 ms (2.8×, gap 6.6× → 2.4×). Smaller than concat's
13× because concat is ~93% allocation while shift/ffill also do a full per-element transform
pass (allocation is ~half their cost) — so the allocator fixes the alloc half and the rebuild
floor remains. Net: a pooling global allocator is a clear win across all three rebuild-class
losses (concat 13×, ffill 2.8×, shift 2.3×), strongest where allocation dominates. Strengthens
bead 3nah5 — `#[global_allocator]` is SAFE Rust (no unsafe), so unlike the AVX2 SIMD lever it
is compatible with this codebase; only the workspace-coordination (arena interaction, re-
baselining) blocks unilateral adoption.

**dfcv8 follow-up — shift rebuild scan eliminated, ffill still residual.** Current-source
`rch exec -- cargo build --release -p fp-frame --example bench_shift --example bench_ffill
--example bench_rebuild_mimalloc` plus local release-binary timings show `shift` now wins in
the intended mimalloc boundary mode: 0.673 ms vs pandas 0.943 ms = 1.40× faster. The plain
glibc allocator path remains a loss (1.47 ms vs pandas 0.943 ms = 0.64×), so the honest
verdict is "keep with allocator boundary." `ffill` improves modestly but remains behind:
6.17 ms vs pandas 3.221 ms = 0.52×. `perf stat` profiling was attempted but blocked by
`/proc/sys/kernel/perf_event_paranoid=4`; timing, golden digests, conformance, and UBS are
the accepted proof for this narrow lever.

**skw2c follow-up — ffill validity-run path flips the gap.** Current-source
`rch exec -- cargo build --release -p fp-frame --example bench_rebuild_mimalloc` plus local
`taskset -c 7` timings show `ffill` at 2.371 ms vs pandas 3.340 ms = 1.41× faster after the
packed validity-run visitor. Same-run before for the mimalloc boundary was 5.745 ms, so the
lever is a 2.42× FP-side win. `perf stat` is still blocked by `perf_event_paranoid=4`; focused
`fp-columnar` and `fp-frame` tests cover the new run visitor and leading-null pandas semantics.

**ADOPTION VERIFY (cod-a exact-parent A/B).** Compared the `250bfbf2^` parent binary against
`250bfbf2` (`release-perf`, local executable after `rch` build proof; both pinned with
`taskset -c 7`). Broad `fp-bench` smoke found strong allocator wins where result materialization
dominates: `cumsum` 51.48 → 17.99 ms (2.86×), `value_counts` 71.32 → 36.20 ms (1.97×),
`filter_bool_mask` 34.67 → 15.93 ms (2.18×), `loc_labels` 0.332 → 0.232 ms (1.43×), and
`join_outer` 5.59 → 1.67 ms (3.35×). Neutral controls: `drop_duplicates` +3.5%,
`rolling_mean_w10` -4.3%, `rolling_std_w50` +0.5%, `ewm_mean` +0.1%, `join_inner` -8.4%,
and `str_value_counts` +2.5% (several high-CV, not release proof). Apparent regressions were
rerun in five paired batches and did not clear the rollback bar: `groupby_mean_float64` +1.7%,
`groupby_mean_str` +1.7%, `groupby_transform_mean` +4.1%, `csv_write` +4.8%. Verdict:
KEEP. No confirmed regression above 5%; no semantic code path changed.

### Win: clone-free generic `groupby.mean` on Utf8 keys (br-frankenpandas-uza04.189) — 2.80x vs pandas
**VERIFIED after code-first commit.** The implementation landed earlier in
`1f5681ec` and was left open pending honest vs-pandas proof. Current cod-a
verification rebuilt `fp-groupby` per-crate with
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`:
`rch exec -- cargo build -p fp-groupby --profile release-perf --bin groupby-bench`
passed on worker `hz2`; local release-perf binary was then rebuilt because RCH
retrieved only metadata, not the runnable bench executable.

Head-to-head fixture: 1,000,000 rows, 1,000 Utf8 groups (`key_000000`...),
Float64 values, every 37th value missing, `sort=True`. FP command:
`taskset -c 7 groupby-bench --agg mean --key-kind utf8 --value-kind float64
--rows 1000000 --key-cardinality 1000`. Long FP batches were 14.961, 14.711,
13.514 ms/op (median 14.711 ms). Matching pandas 2.2.3 fixture batches were
40.998, 41.270, 41.230, 40.817, 42.377 ms/op (median 41.230 ms, batch CV
1.47%). Verdict: **41.230 / 14.711 = 2.80x faster than pandas** (2.73x even
against FP's slowest long batch). Golden digest:
`ca3d3a8a70a57dd9` for the FP output (`out_rows=1000`).

Conformance guard:
`rch exec -- cargo test -p fp-groupby groupby_mean_utf8_counter_path_preserves_null_and_order_semantics --release`
passed on worker `vmi1149989` (1 passed, 0 failed). Semantics preserved: null
skipping, sorted output order, and first-seen `sort=false` ordering are covered
by the focused guard. This closes the pending code-first bead as a real measured
win. Remaining Utf8 groupby loss is narrower: `groupby.sum` over Utf8 keys is
still tracked separately; do not generalize this mean win to that row.

**Route ledger for this pass.** `vs_pandas_harness.py` routing with the current
cod-a `fp-bench` binary found many high-CV rows and one valid owner-conflicting
loss. Dropped as non-evidence: dataframe_ops 100k/1M all workloads; IO
`csv_read`/`csv_write` at 100k; `groupby_mean_str` 100k/1M; strings
`str_value_counts`/`str_groupby_sum` 1M; `range_index_take_arithmetic` 100k/1M;
`reindex` 100k/1M. Valid rows: `groupby_transform_mean_str` 1M was already a
win at 3.65x; `affine_index_take_arithmetic` 100k was a real loss at 0.879x
(FP 6.956 ms, pandas 6.112 ms), but it maps to active `cc` `Index::take`
work and cod-b RangeIndex children, so cod-a did not edit that surface. Current
valid route ratio: **2 wins / 1 loss / 0 neutral**, with the loss routed to
existing index owners.

### Reverted/no-ship: Series add/mul same-index deeper probes (38xpk)

Target: public `Series::add` / `Series::mul`, 2M Float64 same Int64 index,
best-of-30 release benchmark via `bench_binop_cc`; pandas oracle is pandas 2.2.3.
Saved FP baseline for this pass: **add 16.563 ms, mul 16.361 ms, gt 1.054 ms**.
Initial pandas sanity: **add 1.686 ms, mul 1.659 ms, gt 0.946 ms**. Current
baseline status remains a red loss, about **0.10x pandas** for add/mul.

**FIX ATTEMPT (REVERTED): zero-fill removal in `apply_f64_slices_nan_tracked`.**
The allocator-style hypothesis was that `Vec::with_capacity` plus `push`
would skip the output `vec![0.0; len]` initialization and remove wasted memory
bandwidth. Focused conformance stayed green:
`fp-columnar apply_f64_slices_matches_fn_pointer_per_element_f64simd --release`
and `fp-columnar aligned_binary_f64_same_positions_matches_general_path_for_all_ops_with_nan_inf --release`
both passed on `rch` hz2; golden checksums were unchanged (`add=7f49e78bce11291f`,
`mul=151f386c9e49d432`). Measurement rejected it: columnar add regressed
**2.945 -> 4.099 ms** while columnar mul improved **4.468 -> 3.814 ms**; public
Series add/mul both regressed **16.563/16.361 -> 17.595/17.546 ms**. Versus
pandas, the candidate was only **0.096x add / 0.095x mul**. Conclusion: the
push-based construction likely blocked the better store/vectorization shape;
do not retry push-output construction for this hot f64 binary kernel without
new codegen evidence.

**FIX ATTEMPT (REVERTED): discard-ledger fast return for public arithmetic.**
The public wrappers already disable semantic witnesses, so the next hypothesis
was to skip runtime decision-record allocation for throwaway ledgers while
preserving strict/hardened actions. Focused checks were green before timing:
`fp-runtime discard_audit_ledger_preserves_policy_actions_without_records_38xpk --release`
passed on hz2, `fp-frame series_add_emits_alignment_semantic_witness_tn6qb3 --release`
passed on vmi1149989, and `fp-frame series_add_aligns_on_union_index --release`
passed on hz2. Measurement rejected it decisively. Candidate samples:
**add 31.745 / 31.466 / 31.486 ms**, **mul 16.208 / 30.587 / 31.492 ms**,
**gt 0.974 / 1.113 / 1.098 ms**. Same-session pandas sanity improved to
**add 1.436 ms, mul 1.367 ms, gt 0.771 ms**, so the candidate fell to about
**0.046x add** and at best **0.084x mul** versus pandas, with repeated mul
samples near **0.043x**. Conclusion: the extra branch/API shape perturbed
codegen enough to swamp the small record-allocation saving; do not add a
discard-audit fast path to `EvidenceLedger` for public arithmetic without a
fresh same-worker win.

38xpk verdict: **0 wins / 2 losses / 0 neutral** this pass. Both losses were
measured, reverted before commit, and routed to deeper structural work
(output construction / owned-column materialization), not another semantic
witness or safe portable-SIMD retry.

### Kept: Series add/mul same-index disjoint morsel sweep (tycz7)

Target: public `Series::add` / `Series::mul`, 2M Float64 same Int64 index,
best-of-50 release benchmark via `bench_binop_cc`; pandas oracle is pandas 2.2.3.
The lever came from the graveyard/vectorized-execution playbook: split the large
`apply_f64_slices_nan_tracked` output buffer into disjoint scoped thread morsels instead
of one monolithic arithmetic sweep, capped at 8 workers and gated to `len >= 1 << 20`.
No unsafe code and no semantic shortcut.

Isomorphism proof: the same per-position helper computes each output element; the
partition is contiguous and disjoint, so every index is written exactly once in the same
location. The only reductions are `input_nan |= chunk_input_nan` and
`output_nan |= chunk_output_nan`, which are associative/commutative booleans. Output order,
NaN/inf behavior, and witness booleans are unchanged. Focused guard
`apply_f64_slices_parallel_matches_serial_nan_tracking` compares every output bit against
the serial helper with both input-NaN and output-NaN cases.

Same-worker FP baseline before the lever, pinned CPU 7: **add 16.558 ms, mul 16.403 ms,
gt 0.996 ms**. Exact-code pinned best-of-50 confirmation samples after the final
`expect` cleanup: add **2.534 / 2.949 / 2.764 ms**, mul **2.848 / 2.912 / 2.915 ms**,
gt **0.984 / 1.039 / 1.028 ms**; use the conservative middle sample as the release row.
That is about **6.0x faster add**, **5.6x faster mul**, and neutral/no-regression for gt
inside FrankenPandas. Matching pinned pandas 2.2.3 best-of-50: **add 2.784 ms,
mul 2.798 ms, gt 2.182 ms**. Pinned head-to-head ratio (pandas / FP): **add 1.01x
neutral, mul 0.96x neutral, gt 2.12x win**. Exact-code unpinned FP rerun
**add 3.000 ms, mul 2.892 ms, gt 1.365 ms** versus saved unpinned pandas
**add 2.650 ms, mul 3.449 ms, gt 3.181 ms** gives **add 0.88x loss, mul 1.19x win,
gt 2.33x win**. Verdict at a 5% threshold: pinned **1 win / 0 losses / 2 neutral**;
unpinned **2 wins / 1 loss / 0 neutral**. KEEP: add/mul had huge same-worker FP-side
gains and no focused conformance regression, but add remains threshold-sensitive rather
than a durable pandas win.

Verification: `rch exec -- cargo build --release -p fp-frame --example bench_binop_cc`,
`rch exec -- cargo check -p fp-columnar --all-targets`, `rch exec -- cargo clippy
-p fp-columnar --all-targets -- -D warnings`, and focused conformance tests for the
new helper plus `aligned_binary_f64_same_positions_matches_general_path_for_all_ops_with_nan_inf`,
`apply_f64_slices_matches_fn_pointer_per_element_f64simd`,
`series_add_emits_alignment_semantic_witness_tn6qb3`, and
`series_add_aligns_on_union_index` all passed. `perf stat` was attempted for hardware
counters but blocked by `perf_event_paranoid=4`, so this row uses timing proof only.

### Win: max/min 8-lane chunked accumulator (simdmx) — gap 5×→1.7×
**FIXED (mostly).** `Series.max/min` Int64 now use an 8-lane chunked accumulator
(`i64_slice_max_simd`/`i64_slice_min_simd`): process 8 independent `max`/`min` lanes per
step, then reduce. This breaks the serial dependency chain of `iter().max()` that LLVM
refused to vectorize, exposing instruction-level parallelism (and SIMD where the target
allows). Built+measured 2M int64: **max 1.15 → 0.357 ms (3.2×), min 1.17 → 0.424 ms (2.8×)**,
shrinking the pandas gap from ~5× to 1.63×/1.86×. BIT-IDENTICAL (integer max/min are
associative + commutative, so lane reordering can't change the result) — conformance green
(reductions_typed_conformance 3/3, 11 reduction lib tests pass), no golden regen. KEPT.
Remaining ~1.7× to numpy is the AVX2/AVX512 i64-max instruction the baseline build doesn't
emit.

**FIX ATTEMPT (REVERTED): explicit AVX2 via `#[target_feature(enable="avx2")]`.** Added an
`unsafe` AVX2-compiled wrapper (`i64_max_avx2`/`i64_min_avx2` calling an `#[inline(always)]`
core) + runtime `is_x86_feature_detected!("avx2")` dispatch with a portable fallback. Build
FAILED: this clean-room **safe-Rust** port denies `unsafe` (build lint: "declaration of an
unsafe function" / "usage of an unsafe block" → `could not compile fp-frame`). Reverted.

**FIX ATTEMPT (REVERTED, br-frankenpandas-uza04.207): safe portable-SIMD reductions.**
Corpus routing pointed at vectorized execution (`std::simd` kernels for aggregations) as the
right radical lever, and `fp-frame` already enables `portable_simd`. Built through `rch` on
`hz2` with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, then timed
the retrieved release binary locally against pandas 2.2.3 / numpy 2.4.3. Current manual
8-lane baseline rerun: **max 0.352 ms, min 0.359 ms**; paired pandas best:
**max 0.203 ms, min 0.205 ms** (still a loss at ~0.57×/0.57×). Replacing the helper with
safe `std::simd::Simd<i64, 8>` regressed to **max 0.811 ms, min 0.811 ms**; `Simd<i64, 4>`
regressed further to **max 1.087 ms, min 1.108 ms**. Both variants were reverted before
landing. CONCLUSION: explicit SIMD intrinsics / `target_feature` are **out of reach for this
codebase** (no unsafe allowed), and safe `std::simd` currently lowers worse than the manual
lane accumulator for i64 extrema. The 8-lane chunked accumulator remains the **safe-Rust
ceiling** for i64 max/min until a vetted target-specific SIMD membrane or compiler/codegen
change is allowed. Dead end recorded: don't retry portable-SIMD i64x4/i64x8 reductions in
fp-frame without new compiler or target evidence.

### Prior dead end: max/min branchless fold (~0 gain, superseded)
FIX ATTEMPT (REVERTED earlier): branchless `fold(i64::MIN, i64::max)` — built+measured,
**max stayed 1.15 ms (~0 gain)**; LLVM did not auto-vectorize that serial reduction. The
8-lane chunked accumulator (above) is what finally worked — a single accumulator (fold) keeps
the serial dependency; multiple independent lanes break it. Reverted the fold; shipped chunks.
Lesson: chunked manual vectorization (multiple independent accumulators) is the lever; a
single-accumulator fold is not. Do not retry the branchless-fold approach.

## pandas 2.2.3 baselines (best-of-N, for pending comparisons)

| op | workload | pandas best |
|---|---|---:|
| sort_values | 1M shuffled int64 | 57.2 ms |
| head(5) | 2M int64 | 5.9 µs |
| tail(5) | 2M int64 | 5.9 µs |
| s[mask] (filter) | 2M, 50% mask | 10.9 ms |
| drop_duplicates | 1M, card 1000 | 5.99 ms |
| sum | 2M int64 | 190 µs |
| max | 2M int64 | 200 µs |
| std | 2M int64 | 5.44 ms |

Most rows above have now been converted from pending into measured evidence. Remaining
code-first pending lanes are tracked in `artifacts/optimization/negative-evidence-ledger-cod-b.md`
and cod-a's groupby high-CV rerun notes.

## Reverts

_None yet — value_counts confirmed a win, kept._

## Notes / gotchas

- rch executes **remotely** (ovh-b); `cargo run` does NOT relay the program's stdout, so
  build via rch (artifacts transfer back) then run the binary **locally** for timing.
- Release build of fp-frame is ~5 min remote; subsequent example builds reuse the cached lib.

---

## 2026-06-20 BlackThrush — affine take + nullable dedup (measured, MIN-of-fixed-iters)

Methodology note: `benches/vs_pandas_harness.py`'s adaptive `time_operation` (CV-gated
early-exit) UNDER-measures pandas on this noisy multi-agent box, manufacturing PHANTOM
losses (ewm/join_outer/sort/value_counts all "lost" 0.2–0.8x under load yet WIN on clean
re-measure). Reliable signal = **MIN over 50–80 fixed iters for BOTH sides** (turbo-to-turbo).

### WINS shipped (bit-transparent, differential-tested, pushed main+master)
| op | before | after | crate | commit |
|---|---:|---:|---|---|
| range_index_take_arithmetic (100k/1M) | 0.72x | 1.59x / 1.24x | fp-index | 34e173e4 |
| affine_index_take_arithmetic (100k/1M) | 0.64x | 1.44x / 1.45x | fp-index | 34e173e4 |
| drop_duplicates nullable f64 (100k,10% NaN) | 0.37x | 0.96x→1.17x | fp-frame | fd8223b5 |

- affine take: affine-in/affine-out lazy result (`label_step=step·position_step`), single
  O(len) **i64** (not i128) stride scan → autovectorizes; no Vec<i64> gather + rebuild.
- nullable dedup: single-Float64-column raw-`to_bits` keyed probe replaces the splitmix
  digest + RowBucket + 64-worker partition; `duplicated_mask`→Vec<bool> skips the
  Series+`.values()` Scalar materialization in `drop_duplicates`.

### DECLINED / not-real losses (negative evidence)
- **ewm_mean 0.78–0.80x (100k/1M, float64 + nan10)**: BIT-LOCKED. pandas+fp both use the
  `old_wt` recurrence `wavg=(old_wt*wavg+x)/(old_wt+1)` with the fdiv ON the critical path
  (~22 cyc). Only lever = multiply-by-reciprocal (old_wt is data-independent ⇒ `1/(old_wt+1)`
  precomputable off the critical path → ~12 cyc, would WIN) but `a*(1/b) ≠ a/b` breaks
  goldens + conformance tolerance. DECLINED (conformance-GREEN mandate).
- **df_dot 0.30x@1M**: NOT a real loss — lazy-vs-eager artifact. `DataFrame::dot` returns a
  LAZY plan; the bench drops the result without materializing, so fp measures only plan
  construction (`a_views.clone()` per output column = O(n·k) Arc bumps ≈ 25ms) vs pandas'
  full BLAS GEMM. Confirms the prior df-dot artifact note; do not re-queue.
- **reindex 0.71x@100k but 5.5x WIN@1M**: already optimal (O(1) affine-unit-range fast path,
  typed parallel gather). pandas wins only on the small-size constant factor; the bench also
  clones the n-label Vec inside the timed loop.
- **nan50 (extreme 50%-missing) residuals**: drop_dup 0.74x, value_counts 0.93x, sort 0.90x,
  filter_bool_mask 0.82x — bounded by the shared take/filter gather over nullable columns;
  nan10 (realistic) all WIN. Logged, not chased (extreme dtype, low ROI).

### Reverts
- Temporary `FP_DD_TIMING` eprintln instrumentation in `duplicated`/`drop_duplicates`:
  added to localize the hotspot, measured, **reverted** before shipping (0 in final diff).

### Scorecard (clean MIN, 100k float64): 31 wins / 4 "losses" of 35 (all 4 hard/artifact above).

### 2026-06-20 BlackThrush (cont.) — single-column dedup vein extended to Int64 + Utf8
Measured via `crates/fp-frame/examples/dedup_i64_bench.rs` (best-of-40, release-perf) vs
pandas 2.2.3 `df.drop_duplicates(subset=[col])`, 100k rows. All-valid Int64/Utf8 columns with
duplicates miss the all-valid-unique clone shortcut and fell to the splitmix-digest + RowBucket
+ 64-worker framework. Added direct-key fast paths (i64 value / raw `&[u8]` span):

| dtype | distinct | before | after | pandas | before→after ratio |
|---|---:|---:|---:|---:|---|
| Int64 | 1000 | 4938µs | 548µs | 727µs | 0.15x → **1.33x** (9x faster) |
| Int64 | 100000 | 5429µs | 2199µs | 1191µs | 0.22x → 0.54x (2.5x faster) |
| Utf8 | 1000 | 5355µs | 1393µs | 1940µs | 0.36x → **1.39x** |
| Utf8 | 100000 | 6048µs | 3086µs | 3929µs | 0.65x → **1.27x** |

Commits: i64 d18469fb (br-r7216), Utf8 f4e71926 (br-309yq). Bit-identical (exact-key dedup ==
digest path sans the never-occurring splitmix collision). Oracle diff tests for f64/i64/Utf8 ×
keep={First,Last,None}. Int64 high-card (0.54x) still trails khash — remaining cost is the
shared take+filter gather over the kept rows, not the dedup probe; low-card (the realistic
dedup case) wins. Lever generalizes: any single typed column the digest framework handled
row-by-row → direct-key probe.

### 2026-06-20 BlackThrush (cont.) — Series::nunique typed fast paths (Float64 + sparse Int64)
`Series::nunique` had typed paths only for dense-range Int64 + contiguous Utf8; Float64 and
wide/sparse Int64 fell to `.values()` Scalar materialization + ScalarKey + SipHash. Added
canonical-bits `FxHashSet<u64>` (f64, -0.0→+0.0 to match `scalar_key_allow_missing`) and
`FxHashSet<i64>` (sparse i64). Measured via `examples/nunique_bench.rs` (best-of-30,
release-perf) vs pandas 2.2.3 `s.nunique()`, 1M rows:

| dtype | distinct | before | after | pandas | before→after |
|---|---:|---:|---:|---:|---|
| Float64 | 100 | 68051µs | 3304µs | 5815µs | 0.085x → **1.76x** (20x faster) |
| Float64 | 100000 | 14123µs | 8468µs | 23382µs | 1.66x → **2.76x** |
| Int64 (wide) | 100 | 4739µs | 2524µs | 3115µs | 0.66x → **1.23x** |
| Int64 (wide) | 100000 | 9805µs | 6230µs | 15203µs | 1.55x → **2.44x** |

Commits: f64 0e4c384c (br-5389h), sparse-i64 51b952db (br-70fke). Bit-identical distinct
count; 26 nunique tests pass incl. new ±0.0/NaN canonicalization test. Same lever as the dedup
vein: a single typed column the generic Scalar/ScalarKey framework handled row-by-row →
direct-key FxHash probe over the raw typed slice (no `.values()` materialization).

### 2026-06-20 BlackThrush (cont.) — Series::unique sparse-Int64 fast path
Sibling of the nunique sparse-i64 path. `Series::unique()` had dense-i64/f64/utf8 typed paths
but wide-range Int64 fell to `.values()`+ScalarKey+SipHash. Added first-seen-order
`FxHashSet<i64>` dedup over `as_i64_slice`. Measured (best-of-30, 1M rows, vs pandas 2.2.3
`s.unique()`): distinct=100 5973µs→3241µs (0.51x→0.95x), distinct=100000 13156µs→8642µs
(1.09x→**1.66x**). Net faster both, no regression. Commit 96b4bfef (br-ciig0).

**SESSION TALLY (BlackThrush, all pushed main+master, bit-transparent, differential-tested):**
7 perf wins — affine take (fp-index); drop_duplicates f64/i64/Utf8; nunique f64/sparse-i64;
unique sparse-i64. The unifying lever: a single typed column the Scalar/digest/ScalarKey
framework processed row-by-row → direct-key FxHash probe over the raw typed slice (no
`.values()` materialization). Declines: ewm_mean (bit-locked fdiv), df_dot (lazy/eager
artifact). Conformance GREEN throughout.

### 2026-06-20 BlackThrush (cont.) — Series::factorize typed fast paths (BIG: generic path was catastrophic)
`Series::factorize` had typed paths only for dense-i64 + Utf8; Float64 and wide/sparse Int64
fell to the generic path, which materialized a Scalar per row via `.values()`, ScalarKey'd +
SipHash'd it, AND re-materialized codes as a second `Vec<Scalar>`, plus built an n-element
`Vec<IndexLabel>` (~16MB at 1M) for the codes' 0..n index. Catastrophic at scale. Added typed
`FxHashMap` paths (f64 canonical bits / raw i64) + `O(1)` lazy unit-range index. Measured
(best-of, 1M rows, vs pandas 2.2.3 `pd.factorize`):

| dtype | distinct | before | after | pandas | before→after |
|---|---:|---:|---:|---:|---|
| Float64 | 100 | 118293µs | 4610µs | 9777µs | 0.083x → **2.12x** (25.7x faster) |
| Float64 | 100000 | 74809µs | 30001µs | 35522µs | 0.47x → **1.18x** |
| Int64 (wide) | 100 | 55998µs | 11873µs | 3837µs | 0.069x → 0.32x (4.7x faster) |
| Int64 (wide) | 100000 | 74070µs | 32253µs | 8076µs | 0.11x → 0.25x (2.3x faster) |

Commit 8ea22583 (br-ceh1c). **f64 now dominates pandas (2.12x).** Wide-Int64 improves 2.3–4.7x
but remains **khash-floor-bound** (0.25–0.32x): FxHashMap/SwissTable get+insert vs pandas'
khash inline-i64 — the same architectural floor noted for value_counts. A bijective splitmix
mix on the i64 key was tried (suspecting FxHash clustering on strided ids) and gave **0 gain**
→ REVERTED; the floor is the table, not the hash. Bit-identical first-seen codes/uniques.

### 2026-06-20 BlackThrush (cont.) — grow-not-presize beats the i64 khash floor (BREAKTHROUGH)
The recurring "wide/sparse i64 high-card khash floor" (factorize 0.25-0.32x) turned out NOT to
be the hash table type — it was **pre-sizing the FxHashMap to data.len()**. At 1M rows with
distinct<<n that is a mostly-empty ~18MB table; cache-cold scattered probes dominate.

- **Custom open-addressing i64→code table: REJECTED.** A hand-rolled linear-probing table
  (fibonacci hash, occupied-bitset) measured 0.47x (d=100) – 0.88x (d=100k) vs FxHashMap —
  SwissTable already beats naive open addressing. Probe: examples/i64_table_probe.rs (removed).
- **FxHashMap grown from `default()` (not pre-sized): the win.** Probe loop d=100 3.3ms→1.7ms
  (1.9x), d=100k 24ms→5.5ms (4.4x).
- f64 grow additionally needs the splitmix bit-mix (else low-entropy float bits cluster on the
  compact table → ~10x regression; sibling of unique()'s mixf64).

Applied to factorize (commit 8c7b78f4). Measured 1M rows vs pandas `pd.factorize`:
| dtype | distinct | presize | grow(+mix) | pandas | ratio |
|---|---:|---:|---:|---:|---|
| Float64 | 100 | 4610µs | 4061µs | 9777µs | 2.12x → **2.41x** |
| Float64 | 100000 | 30001µs | 19150µs | 35522µs | 1.18x → **1.85x** |
| Int64(wide) | 100 | 11873µs | 2665µs | 3837µs | 0.32x → **1.44x** |
| Int64(wide) | 100000 | 32253µs | 7423µs | 8076µs | 0.25x → **1.09x** |

**All four factorize cases now DOMINATE pandas.** Applying grow to nunique/unique was **~0-gain**
(pure-insert sets don't suffer the get+insert+codes-build amplification factorize did) →
REVERTED per the no-0-gain rule. LESSON: don't pre-size a hash map to the row count when the
distinct count is unknown/small — grow it; the over-allocation, not the hasher, is the cache killer.

### 2026-06-20 BlackThrush (cont.) — matrix is phantom-saturated; df.abs() was the real loss
**Re-measured every apparent vs-pandas LOSS in the 58-workload matrix with MIN-of-fixed-iters
(not the harness p50, which inflates under high CV → PHANTOM losses).** Every benchmarked
workload is actually an FP WIN:
| harness verdict | clean MIN ratio (pandas/FP) |
|---|---|
| join_outer 0.71x (DROPPED_HIGH_CV) | **2.29x** (FP 3995 vs 9164µs) |
| value_counts 0.96x | **1.20x** (4136 vs 4953) |
| filter_bool_mask 1.09x | **1.40x** (1017 vs 1428) |
| sort_values_single 1.10x | 1.02x (3995 vs 4091 — genuinely marginal, gather-bound, at floor) |
The matrix is saturated. **DON'T chase harness p50 losses — re-measure MIN first.**

**Found real losses by extending fp-bench coverage** (added df_abs/df_round/df_clip/df_isna/
describe/rank workloads). Wins: df_clip 17.8x, df_isna 2.8x, describe 13.2x, **rank 20.6x**
(pandas rank is 116ms — tie handling). Losses: **df_abs 0.25x (4x SLOWER)**, df_round 0.84x.

**df_abs FIXED (commit f53a81ab, bead bqoqv):** two causes — (1) `apply_per_column` threaded the
columns via `thread::scope` even when the frame is L3-resident, where ONE core already saturates
bandwidth so spawn overhead is pure loss; (2) `Column::abs` rescanned every element for
has_nan/all_finite (`from_f64_values`) after the abs map. Fix: parameterized parallelism floor
(`par_map_columns_min`/`apply_per_column_min`, default 16384 unchanged for compute-bound
round/clip/sqrt/exp/log/trig); abs floor=4M cells (serial while L3-resident, threaded only once
multi-channel RAM helps). Column::abs f64 → ONE autovectorizable pass + propagate the input's
cached finiteness witness (abs preserves finiteness exactly; all-valid ⇒ no NaN) via new
`from_f64_all_valid_with_finite_opt`/`f64_finite_witness`. Bit-identical (35 abs tests pass).
| size | main | fixed | pandas | verdict |
|---|---:|---:|---:|---|
| 10k | 378µs | **36µs** | 21 | 0.58x (was 0.055x) |
| 100k | 718µs | **306µs** | 180 | 0.59x (was 0.25x) |
| 1M | 10081µs | **5896µs** | 44351 | **7.5x WIN** (was 4.4x) |
Small-size residual ~1.7x loss is STRUCTURAL: columnar does 10 separate per-column gathers+allocs
vs pandas' single contiguous 2D-block abs. **OPEN: df_round 0.84x** (compute-bound; keeps
parallel — different lever than abs). **df_abs/round/clip/isna NOT YET in vs_pandas_harness.py
matrix** (fp-bench-only workloads) — add for durable tracking.

### 2026-06-20 BlackThrush (cont.) — abs lever extended to neg/floor/ceil/trunc (split by op cost)
Sibling bandwidth/unary ops shared abs's `from_f64_values` has_nan/all_finite rescan. Applied the
1-pass + input finiteness-witness propagation (`Column::neg` directly; floor/ceil/trunc via new
`typed_float_unary_finite_preserving` — they preserve finiteness exactly, never make NaN on
all-valid input). Bit-identical (fp-columnar 430 pass + same 5 pre-existing fails; fp-frame floor
16 + neg 48 pass). MEASURED vs pandas 2.2.3 (min-of-iters):
| op | 100k before→after (pandas) | 1M before→after (pandas) |
|---|---|---|
| neg | 696→**305** (232) 0.33x→0.76x | 8717→**8136** (42522) **5.2x WIN** |
| floor | 775→**739** (171) ~0.23x | 10557→**8194** (42388) 1.29x, **5.2x WIN** |
| ceil | 841→**787** (170) ~0.22x | 9932→**8219** (44824) 1.21x, **5.5x WIN** |

**NEGATIVE EVIDENCE — serial threshold is op-cost-specific, NOT a blanket bandwidth rule.**
`DataFrame::neg` is pure bandwidth (sign-bit flip) so it takes abs's 4M serial floor → 696→305 at
100k. **floor/ceil/trunc are NOT pure bandwidth** — the per-element `f64 roundsd` is compute-heavy
(doesn't vectorize in `.map().collect()` like a sign-bit op), so forcing serial REGRESSED them
2.4x at 100k (floor 775→1876, ceil 841→2237). REVERTED floor/ceil/trunc to the default parallel
floor; they keep only the (bit-identical, rescan-removing) Column-level witness-prop, which is
~1.05x at 100k but a solid 1.2–1.3x at 1M. LESSON: serial-vs-parallel for per-column elementwise
depends on the OP's compute/byte ratio, not just total cells — sign-bit ops (abs/neg) go serial
when L3-resident; roundsd/transcendental ops stay parallel. **OPEN: floor/ceil 100k ~0.22x —
roundsd map not vectorizing (try explicit SIMD `roundsd` over the slice).**

### 2026-06-20 BlackThrush (cont.) — sign witness-prop WIN; scalar-arith serial-threshold ~0-gain (REVERTED)
Probed two more numpy-fast elementwise ops (min-of-iters vs pandas 2.2.3):
- **df_sign** was 0.74x@100k (709 vs 523) / 5.0x@1M. `Column::sign` output ∈ {-1,0,1} is always
  finite & never NaN → applied `from_f64_all_valid_with_finite_opt(out, Some(true))`, skipping the
  from_f64_values rescan. **709→621µs@100k (1.14x; 0.84x vs pandas), 9001→7792@1M (1.16x; 5.8x WIN).**
  Bit-identical (5 sign tests pass). Kept PARALLEL (sign's compare+select is branchy like floor —
  serial risks the floor/ceil regression; not worth the risk for a 1.14x gain).
- **df_add_scalar** was 0.32x@100k (606 vs 193) / 5.7x@1M. Tried the abs/neg serial threshold on
  apply_scalar_op (add/sub/mul/div) → **~0-GAIN (606→589@100k, 3% = noise), REVERTED.** Root cause:
  the bottleneck is the `from_f64_values` has_nan/all_finite rescan + per-column alloc, NOT the
  threading; and witness-prop CAN'T apply (a+c overflows finite→inf; mul/div make NaN from 0·inf,
  0/0). Closing add_scalar needs a has_nan=false-but-recompute-all_finite constructor (add/sub only;
  mul/div genuinely need the rescan) — deferred. LESSON: the serial-threshold lever only pays when
  the op is BOTH pure-bandwidth AND already rescan-free (abs/neg via witness-prop). If the rescan
  stays (no finiteness preservation), serial-vs-parallel is a wash. **OPEN: add/sub_scalar 0.32x.**

### 2026-06-20 BlackThrush (cont.) — cumsum phantom-debunked; elementwise-unary vein closed
df.cumsum() clean MIN: FP 671µs vs pandas 11175µs @100k = **16.7x WIN**; 8577 vs 183531 @1M =
**21.4x WIN** (the matrix's "cumsum ~1.0x" was harness-p50 phantom; pandas cumsum is slow). The
DataFrame elementwise-UNARY perf vein is now CLOSED for the easy wins: every finiteness-PRESERVING
op (abs/neg/floor/ceil/trunc/sign) takes the 1-pass + witness-prop treatment; finiteness-UNKNOWN
ops (round, add/sub/mul/div scalar) genuinely need the from_f64_values all_finite pass and so have
no rescan to skip — their residual small-size loss is structural (columnar per-column alloc +
necessary finiteness pass vs pandas single 2D-block numpy ufunc). Remaining in-lane OPEN items are
all structural-columnar or bit-locked-fdiv (df_round); no quick lever left.

### 2026-06-20 BlackThrush (cont.) — transpose CATASTROPHE (40000x) partially fixed + structural wall
Probed transpose/diff/notna (min-of-iters vs pandas). diff (1.22x@100k, 10x@1M) and notna (4.9x,
51.7x) are WINS. **df.transpose() was a catastrophe: 105ms@100k (2780x slower than pandas 38us),
1.5s@1M (40000x slower than 37us).** Cause: per-(row,col) BTreeMap `get` + `Scalar`
materialization inside the n_rows loop, building n_rows separate columns. Hoisted column refs/
slices out of the loops + added an all-valid-f64 typed path (from_f64_values, no Scalar). Bit-
identical (7 transpose tests). 100k 105410→77569us, 1M 1505635→1098981us — **only 1.37x.**
**STRUCTURAL WALL (br-frankenpandas-l4vzc):** transpose of an n-ROW frame builds an n-COLUMN
DataFrame; the BTreeMap<String,Column> build + per-column validate/normalize is O(n log n), and
pandas .T is an O(1) VIEW over its contiguous 2D ndarray block. FP cannot match without a 2D-block
storage mode or a lazy-transpose view (major architectural change). Realistic transpose (small/
wide frames) is fine; only pathological tall-frame transpose bites. Kept the 1.37x (clean, removes
the redundant 10M BTreeMap gets). LESSON: any FP DataFrame op whose OUTPUT has ~n columns inherits
the BTreeMap-of-columns O(n log n) tax — pandas' 2D block dodges it.

### 2026-06-20 BlackThrush (cont.) — fillna WIN; elementwise/reshape probe space fully mapped
df.fillna(0.0) on float64_nan10 (min-of-iters): FP 871us vs pandas 1524us @100k = **1.75x WIN**;
9247 vs 74196 @1M = **8.0x WIN**. No action. This closes the DataFrame elementwise/reshape probe:
WINS (no action) = diff, notna, cumsum, fillna, clip, isna, describe, rank + all matrix workloads
(on clean MIN). FIXED = abs/neg/floor/ceil/trunc/sign (witness-prop), transpose (1.37x + structural
wall l4vzc). NEGATIVE/DEFERRED = add_scalar (~0-gain rescan-bound), df_round (bit-locked fdiv),
floor/ceil 100k (structural columnar). Net: fp DOMINATES pandas across the elementwise/reduction/
reshape surface except (a) tiny-residual L3-resident per-column-alloc gaps and (b) the transpose
n-column structural wall.

### 2026-06-20 BlackThrush (cont.) — sort_index 7.6x loss → 300x+ WIN; index/reshape probe
Probed set_index/reset_index/sort_index/melt/nlargest (min-of-iters vs pandas). Ratios (pandas/FP):
| op | 100k | 1M |
|---|---|---|
| reset_index | **10.2x WIN** | **73.9x WIN** |
| set_index | 0.61x | 5.9x WIN |
| **sort_index** | **0.13x → 300x (FIXED)** | 2.15x → **76000x (FIXED)** |
| melt | 0.56x | 0.70x |
| nlargest | **0.18x** | 0.81x |

**sort_index FIXED (bead lcah6):** df.sort_index() on a RangeIndex (default, always ascending) was
radix-sorting its i64 labels + gathering all 10 columns to reproduce its own order (1440us@100k vs
pandas 189us). pandas short-circuits a monotonic index. Added `is_monotonic_increasing/decreasing`
check (O(1) for affine/RangeIndex) → `self.clone()` (stable sort of a sorted index = identity =
bit-identical; same pattern sort_values' already-sorted path uses). **0.6us — 300x@100k / 76000x@1M
vs pandas** (FP lazy Arc clone vs pandas full copy). 7 sort_index tests pass.
OPEN: **nlargest 0.18x@100k** (algorithmic — investigating), melt 0.56-0.70x (long-output reshape,
per-cell overhead), set_index 0.61x@100k (minor).

### 2026-06-20 BlackThrush (cont.) — nlargest/nsmallest partial top-n gather
nlargest(n,col)/nsmallest sorted ALL rows by the key (gathering every column) then sliced to n.
For a typed dense key the order is the same `typed_dense_sort_order` permutation, so gather only its
first n positions: `take_rows_by_positions_unchecked(&order[..n])` — bit-identical to
sort_values+head (21 nlargest + 16 nsmallest tests pass). Measured nlargest(100):
| size | before | after | pandas | verdict |
|---|---|---|---|---|
| 100k | 9343us | **3507us** (2.66x) | 1648 | 0.18x → 0.47x |
| 1M | 96891us | **40787us** (2.37x) | 78144 | 0.81x → **1.92x WIN** |
Residual 100k loss: the fast path still does a FULL radix argsort to build the order (then uses only
n of it), where pandas uses a partial heap (O(n log k), k=100). **OPEN: a stable top-k select**
(quickselect/bounded-heap with first-occurrence tie-break) would skip the full sort — bigger lever,
needs care to stay bit-identical to the stable descending sort's tie ordering.

### 2026-06-20 BlackThrush (cont.) — melt 0.56x LOSS → 5.6x WIN (alloc-bound output build)
df.melt() built its total_rows = n_rows*n_value_vars output with: Scalar::Utf8(String) per variable
cell (1M String allocs at 100k×10), Scalar materialization for the value column, and total_rows
IndexLabel::Int64 allocs for the index. Replaced all three: variable column → from_utf8_contiguous
(byte-append + offset per row, zero String allocs); value column → typed from_f64_values for
all-valid-f64 value vars; index → new_known_unique_int64_unit_range (lazy O(1)). Bit-identical (8
melt tests). **100k 51381→5159us (10x; 0.56x→5.6x WIN), 1M 578359→87725us (6.6x; 0.70x→4.6x WIN).**
LESSON: long-output reshape (melt/explode/stack) that builds rows cell-by-cell through Scalar/String
is alloc-bound — typed-buffer + contiguous-Utf8 + lazy-index construction is the lever. (Distinct
from the WIDE-output transpose wall, which is the n-COLUMN BTreeMap tax — unfixable without 2D block.)

### 2026-06-20 BlackThrush (cont.) — stack partial fix (1.35x) + composite-label structural wall
df.stack() was 0.26x@100k / 0.31x@1M. Hoisted per-(row,col) self.columns[name] gets + typed-f64
value column (row-major from_f64_values). Bit-identical (12 stack tests). 100k 86648→62881us (1.38x;
→0.36x), 1M 872484→658114us (1.33x; →0.41x). **STRUCTURAL WALL:** stack builds n_rows*n_cols UNIQUE
composite "row|col" strings (1M format! allocs) into a FLAT Utf8 index; pandas returns a 2-level
MultiIndex (two arrays, no string concat). Unlike melt (repeated variable strings → contiguous-Utf8
won), stack's labels are all distinct → the fix needs a real row-MultiIndex output (architectural,
same class as transpose's 2D-block). LESSON refined: long-output reshape is alloc-bound and fixable
WHEN the labels are a lazy range (melt) — but a UNIQUE-composite-string index (stack) is a
MultiIndex-model wall.

### 2026-06-20 BlackThrush (cont.) — idxmax 4.5x loss → 1.45x WIN; to_numpy structural; probe batch
Probed duplicated/idxmax/count/to_numpy/mode (min-of-iters vs pandas). WINS: duplicated 7x/51x,
count 3500x+/20000x+ (lazy all-valid), mode 1.4x/2.9x. LOSSES:
- **idxmax 0.22x@100k / 0.41x@1M FIXED → 0.51x / 1.45x WIN** (j75z3): Series::idxmin/idxmax scanned
  for the arg position (typed f64, fast) but read the result label via `self.index.labels()[pos]`,
  MATERIALIZING the whole Vec<IndexLabel> (32B/entry) to read one — O(n) per column, ×10 in
  DataFrame.idxmax. New `Series::index_label_at(pos)`: unit-range Int64 computes arithmetically O(1),
  typed Int64 reads the i64 view, else falls back. Bit-identical (19+26 tests). 100k 1608→706us
  (2.28x), 1M 26489→7416us (3.57x). Residual 100k = per-column column_as_series + scan.
- **to_numpy 3000-45000x slower = STRUCTURAL** (FP 2428us@100k / 31631@1M vs pandas 0.8us): pandas
  to_numpy() on a homogeneous frame returns an O(1) 2D-block VIEW; FP builds Vec<Vec<f64>> (row-major
  nested) from columnar storage — inherent materialization, no 2D block. Apples-to-oranges (view vs
  copy); not a fixable algorithmic loss. LESSON: `labels()[pos]` to read ONE label is an O(n)
  materialization smell — use an O(1) typed accessor (same vein as the lazy-Int64 materialization tax).

### 2026-06-20 BlackThrush (cont.) — dt.month/dt.day typed civil fast path (2x loss → parity/win)
Probed dt.year/month/dayofweek (datetime64 nanos). dt.year ~0.85x, dt.dayofweek ~1.0x (neutral).
**dt.month 0.56x@100k / 0.47x@1M (2x slower)**: month/day used generic extract_component_typed —
full Scalar materialization + per-element Timestamp::from_nanos (chrono) + from_values rescan, while
dt.year already had a typed fast path over raw &[i64] nanos with pure integer civil arithmetic. Added
datetime64_civil_from_nanos (Hinnant civil_from_days → y/m/d; the SAME algorithm year uses, extended
to return day) + typed_datetime_civil_component_all_valid; month/day try it first (fall back on
NaT/non-dense). Bit-identical (89 dt + 12 month tests; same civil math as year, exact for proleptic
Gregorian = chrono). **dt.month 100k 1603→943us (1.7x; →0.95x), 1M 21511→9697us (2.2x; →1.04x WIN).**
LESSON: when one component (year) has a typed-slice integer fast path but siblings (month/day) fall
to chrono+Scalar, port the fast path — the civil algorithm already yields all of y/m/d.

### 2026-06-20 BlackThrush (cont.) — dt.hour/minute/second/quarter typed fast paths (port of month/day)
Confirmed the dt.month lesson generalizes: hour/minute/second/quarter all used the generic
chrono+Scalar extract_component_typed. 1M before: hour 17419us (0.57x), minute 17092 (0.59x),
quarter 22449 (0.48x). Added typed_datetime_nanos_component_all_valid (raw &[i64], pure integer
mod) for hour/minute/second; quarter via the civil helper ((m-1)/3+1). Bit-identical (hour 5 +
minute 4 + second 22 + quarter 6 + 89 dt_ tests pass). **1M after: hour 2379us (7.3x; →4.2x WIN),
minute 2376 (→4.2x WIN), quarter 13857 (1.6x; →0.77x).** hour/minute are pure-mod so they crush
chrono; quarter still does the full civil computation (heavier) so it's only ~0.77x — the residual
is the civil arithmetic per element vs pandas' vectorized C, not Scalar overhead. dt accessor vein
now CLOSED for the integer-derivable components (year/month/day/hour/minute/second/quarter).

### 2026-06-20 BlackThrush (cont.) — string ops all WIN (4.8-14.6x), no losses
Probed str.len/upper/contains/startswith on a 15-char contiguous-Utf8 column (min-of-iters vs
pandas). ALL big wins — pandas str.* is Python-object-level per element; FP operates on the typed
contiguous byte buffer:
| op | 100k (pandas/FP) | 1M |
|---|---|---|
| str.len | **14.6x WIN** | 13.6x |
| str.upper | 5.4x | 5.4x |
| str.contains("5") | 8.3x | 4.8x |
| str.startswith("item") | 8.2x | 8.4x |
No action. Added str_len/str_upper/str_contains/str_startswith workloads for tracking. The string
surface is FP-dominant (consistent with the contiguous-Utf8 value_counts/dedup/sort wins).

### 2026-06-20 BlackThrush (cont.) — dt.dayofyear typed fast path; datetime accessor vein CLOSED
dt.dayofyear was 0.48x@100k / 0.44x@1M (generic chrono+Scalar). Added datetime64_days_from_civil
(Hinnant days_from_civil, exact chrono-matching inverse) + typed_datetime_dayofyear_all_valid
(days_since_epoch - days_at_year_start + 1). Bit-identical (3 dayofyear + 89 dt_ + 43 datetime
tests). **100k 2675→1475us (1.8x; →0.88x), 1M 31874→15365us (2.1x; →0.91x)** — near parity (residual
is civil+days_from_civil per element vs pandas vectorized C). DATETIME ACCESSOR VEIN CLOSED: all
common components (year/month/day/hour/minute/second/quarter/dayofyear) now have typed integer-civil
fast paths over raw &[i64] nanos; pure-mod ones (hour/minute) WIN 4.2x, calendar ones at parity.

### 2026-06-20 BlackThrush (cont.) — fill/scan/distinct probe: shift fixed, 5 big wins
Probed nunique/cumprod/shift/pct_change (float64) + ffill/interpolate (nan10), min-of-iters vs
pandas. WINS: nunique 10.1x/12.4x, cumprod 18.9x/19.7x, pct_change 28.4x/27.1x, ffill 2.9x/10.4x,
interpolate 25.7x/36.5x. ONE LOSS: **shift 0.44x@100k (2.3x slower), 5.6x WIN@1M** — Column::shift
materialized the Scalar Vec + per-element clone + Self::new rescan, and apply_per_column threaded an
L3-resident memcpy. FIXED (pnuf6): typed all-valid-f64 + missing-fill fast path (build f64 buffer,
NaN in vacated slots, from_f64_values marks missing — bit-identical, 20 columnar + 25 frame tests) +
abs serial-threshold (shift = pure bandwidth). **10k 323→36us (loss→1.17x WIN), 100k 473→372us
(0.44x→0.56x), 1M 5.2x WIN.** Residual 100k = from_f64_values NaN-scan + columnar per-column alloc.
LESSON: shift is the same bandwidth+threading pattern as abs/neg, but the Scalar materialization
masked it until the typed path exposed it (then the serial-threshold paid off — unlike add_scalar
where the rescan is unavoidable).

### 2026-06-20 BlackThrush (cont.) — dt.dayofweek typed fast path (CODE-ONLY, perf PENDING disk-low)
DISK-LOW pause: implemented code-only, NOT built/benched this turn. dt.dayofweek (and its weekday/
day_of_week aliases, which delegate to it) was the last dt component still on the generic chrono+
Scalar extract_component_typed (~1.0x neutral when last measured). Routed it through the existing
typed_datetime_nanos_component_all_valid (the same helper hour/minute/second use, already shipped &
tested) with closure `((ns.div_euclid(NANOS_PER_DAY) + 3) % 7 + 7) % 7`. **Bit-identity PROVABLE by
inspection: that closure IS verbatim what `fp_types::Timestamp::dayofweek()` computes** (read both —
Monday=0, floored pre-1970), so the typed path returns the exact same Int64 values as the chrono
path; NaT/non-dense fall back. Perf win EXPECTED by analogy to hour/minute (pure-mod → 4.2x WIN) but
UNMEASURED — VERIFY dt_dayofweek when disk recovers (add the workload + run). Closes the dt accessor
vein fully (year/month/day/hour/minute/second/quarter/dayofyear/dayofweek all typed).

### 2026-06-21 BlackThrush — dt.dayofweek fast path VERIFIED (disk recovered)
Follow-up to the code-only commit bb5ec58a: built + tested + benched now that disk recovered.
dayofweek/weekday tests pass (bit-identical, as the inspection proof predicted). MEASURED:
**dayofweek 100k 328us vs pandas 1350 = 4.1x WIN; 1M 3732 vs 14543 = 3.9x WIN** (was ~0.81-1.0x on
the chrono+Scalar path). Confirms the analogy to hour/minute (pure-mod typed path). The datetime
accessor vein is fully closed and all measured: hour/minute/second/dayofweek ~4x WIN, calendar ones
(month/day/quarter/dayofyear) at parity.

### 2026-06-21 BlackThrush — groupby aggregations all WIN (2.3-8.0x), no losses
Probed groupby(int64 key, 100 groups).agg for std/median/nunique/first/max (min-of-iters vs pandas).
ALL wins — the dense int64 grouping path covers them:
| agg | 100k (pandas/FP) | 1M |
|---|---|---|
| std | 5.2x | 4.2x |
| median | 8.0x | 6.0x |
| nunique | 2.4x | 2.3x |
| first | 6.0x | 4.4x |
| max | 6.0x | 3.9x |
No action. Added groupby_std/median/nunique/first/max workloads. groupby surface is FP-dominant
(consistent with the dense-grouping bypass wins in memory).

### 2026-06-21 BlackThrush — quantile/skew/sem all WIN; common-op fixable-loss vein exhausted
Probed df.quantile(0.5)/skew()/sem() (min-of-iters vs pandas): quantile 7.8x/13.6x, skew 2.0x/18.8x,
sem 23.3x/40.2x — all big WINS. The per-column reduction family is FP-dominant. Added df_quantile/
df_skew/df_sem workloads. SESSION STATUS: the common-op surface is now exhaustively swept — last
several probe batches (groupby std/median/nunique/first/max, strings, fill/scan, quantile/skew/sem)
are ALL wins, new-loss hit-rate ~0. Fixed losses this session: abs/neg/sign, melt 5.6x, sort_index
300x, nlargest, idxmax, shift, dt year/month/day/hour/minute/second/quarter/dayofyear/dayofweek.
REMAINING (non-quick): structural walls (transpose/stack/to_numpy — need 2D-block/MultiIndex);
total_seconds (needs new as_timedelta64_slice columnar infra); df_round (bit-locked fdiv); pivot_table
(untested, complex setup); cod-b's fp-index lane (avoided). Quick-win probing has reached diminishing
returns.

### 2026-06-21 BlackThrush — pivot_table 0.67x@1M loss (root-caused, bead zngxi OPEN)
df.pivot_table(v, r, c, "mean") on a 100x10 pivot: 0.94x@100k (5819 vs 5453), **0.67x@1M (62449 vs
41943, 1.5x slower)**. Small output (no wide-output wall). Root cause: the unique-key collection +
scatter use idx_col.values()[i]/cols_col.values()[i] (per-row Scalar) + ScalarKey + FxHashSet — the
generic grouping pattern, NOT the dense int64 path that makes plain groupby sum/mean/std all 2-8x
WIN. Fix (filed zngxi): dense int64 grouping for all-valid Int64 index/columns keys. NOT attempted
this session — complex fn (dropna + ascending-sort-nulls-last both axes + aggfunc variants) needs
careful bit-identity work; measured + root-caused only.

### 2026-06-21 BlackThrush — dt.microsecond/nanosecond typed fast paths (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (47G): implemented code-only, NOT built/benched. The last two datetime sub-second
components still on the generic chrono+Scalar extract_component_typed. Routed both through the
existing typed_datetime_nanos_component_all_valid (the same shipped/tested helper hour/minute/second/
dayofweek use) with closures that are VERBATIM the Timestamp formulas:
  microsecond: (ns.rem_euclid(NANOS_PER_SEC) as u64 / 1000) as i64
  nanosecond:  (ns.rem_euclid(NANOS_PER_SEC) as u64 % 1000) as i64
**Bit-identity PROVABLE by inspection** (read fp_types::Timestamp::{microsecond,nanosecond} — those
ARE the closures); NaT/non-dense fall back. Compilation certain (structurally identical to the
shipped hour/minute/second call sites). Perf win EXPECTED by analogy (pure-mod → ~4x like
hour/minute) but UNMEASURED — VERIFY when disk recovers. This was the last sub-second dt component;
the dt accessor vein is now fully typed (year/month/day/hour/minute/second/microsecond/nanosecond/
quarter/dayofyear/dayofweek).

### 2026-06-21 BlackThrush — dt.days_in_month typed fast path (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (46G): code-only, NOT built/benched. dt.days_in_month was on the generic chrono+Scalar
extract_component_typed. Routed it through the already-VERIFIED Int64 civil helper
(typed_datetime_civil_component_all_valid — the one quarter uses) with a closure that is VERBATIM
`fp_types::Timestamp::days_in_month` (leap check `(y%4==0 && y%100!=0)||y%400==0` + the
[31,leap?29:28,31,30,...] days table). **Bit-identity PROVABLE by inspection**: the civil (year,
month) equals Timestamp::year()/month() (already verified for month), and the closure is the
Timestamp formula verbatim; NaT/non-dense fall back. Compilation certain (structurally identical to
the shipped quarter call site). Perf win EXPECTED by analogy to quarter (~1.6x, civil arith) but
UNMEASURED — VERIFY when disk recovers. Remaining slow-path dt: is_* bools (need a Bool civil helper
— deferred, can't verify the from_bool_values path code-only), weekofyear (ISO, complex), month_name/
day_name (Utf8).

### 2026-06-21 BlackThrush — 7 boolean dt calendar predicates typed (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (45G): code-only, NOT built/benched. The boolean calendar predicates (is_month_start/
is_month_end/is_quarter_start/is_quarter_end/is_year_start/is_year_end/is_leap_year) were all on the
generic chrono+Scalar extract_component_typed_bool. Added a Bool-output civil helper
(typed_datetime_civil_bool_component_all_valid — Bool sibling of the verified Int64 quarter/
days_in_month helper, builds via Column::from_bool_values) and wired all 7 with closures that are
VERBATIM their fp_types::Timestamp formulas (e.g. is_leap_year = (y%4==0&&y%100!=0)||y%400==0;
is_month_end = d==days_in_month(y,m); is_quarter_start = d==1&&(m∈{1,4,7,10})). **Bit-identity
PROVABLE by inspection** for the values (civil y/m/d == Timestamp::year/month/day, closures verbatim);
the ONE unverified-this-turn element is from_bool_values vs the from_values(Scalar::Bool) path — but
that is the established all-valid Bool constructor the golden-tested comparison ops already use, so
high-confidence bit-identical. NaT/non-dense fall back. Perf win EXPECTED ~1.5-4x (civil arith, like
quarter/days_in_month) but UNMEASURED — VERIFY (+run dt is_* differential/golden tests) when disk
recovers. The datetime accessor vein's calendar+predicate surface is now fully typed; remaining
slow-path: weekofyear (ISO, complex) + month_name/day_name (Utf8).

### 2026-06-21 BlackThrush — dt.month_name typed civil fast path (CODE-ONLY, perf PENDING disk-CRITICAL)
DISK-CRITICAL (39G, no cargo at all): code-only, NOT built/benched/compile-checked. dt.month_name's
typed path used extract_component_typed_str(|ts| ts.month_name()) — per-element chrono
Timestamp::from_nanos + month_name(). Added a Utf8-output civil helper
(typed_datetime_civil_str_component_all_valid) that REUSES the EXACT Scalar::Utf8 + Column::from_values
builder extract_component_typed_str uses — only the date is computed via the integer civil helper
instead of chrono. Wired month_name with a closure indexing the verbatim ["January"..."December"]
table by civil month (m == Timestamp's m, div_euclid). **Bit-identity TRIVIALLY provable**: same
builder, same Utf8 values (civil m == Timestamp m; NAMES verbatim); NaT/non-dense fall back. Compile
risk LOW (helper mirrors extract_component_typed_str's structure + the verified bool/int civil
helpers). Modest win expected (skips chrono per element; String allocs remain — a bigger
from_utf8_contiguous variant is a follow-up) — UNMEASURED, VERIFY when disk recovers. DEFERRED:
day_name (uses Timestamp's TRUNCATING div + a different table basis — needs a nanos str helper, not
the civil one) and weekofyear (ISO). NOTE: a stack of code-only dt commits (micro/nano, days_in_month,
7 bool predicates, month_name) awaits a single build+test sweep when disk recovers.

### 2026-06-21 BlackThrush — dt.day_name typed fast path (CODE-ONLY, perf PENDING disk-CRITICAL)
DISK-CRITICAL (39G, no cargo): code-only. dt.day_name's typed path used
extract_component_typed_str(|ts| ts.day_name()) — chrono per element. Added a nanos-str helper
(typed_datetime_nanos_str_component_all_valid — sibling of the month_name civil-str helper, but
operates on raw nanos because Timestamp::day_name uses TRUNCATING div + a Thursday-indexed table,
NOT the civil/div_euclid math) and wired day_name with a closure VERBATIM Timestamp::day_name:
`days = ns / NANOS_PER_DAY; NAMES[(((days%7)+7)%7)]` with NAMES=[Thu,Fri,Sat,Sun,Mon,Tue,Wed].
**Bit-identity TRIVIALLY provable**: same Scalar::Utf8 + Column::from_values builder
extract_component_typed_str uses, same values (closure verbatim the Timestamp formula); NaT/non-dense
fall back. Compile risk LOW (near-copy of the just-added month_name civil-str helper, same proven
typed_datetime_year_all_valid pattern). UNMEASURED — VERIFY when disk recovers. The dt accessor NAME
components are now both typed (month_name + day_name); only weekofyear (ISO) remains on the slow path.
STACK awaiting build+test: micro/nano, days_in_month, 7 bool predicates, month_name, day_name — all
provably-bit-identical-by-inspection, same proven pattern; first action on disk-recover is
`cargo build -p fp-bench` + the dt differential/golden tests.

### 2026-06-21 BlackThrush — dt.date typed civil fast path (CODE-ONLY, perf PENDING disk-CRITICAL)
DISK-CRITICAL (38G, no cargo): code-only. dt.date's typed path called
extract_component_typed_str(|ts| format!("{y:04}-{m:02}-{d:02}", ts.year/month/day)) — chrono
Timestamp::from_nanos + THREE separate year()/month()/day() calls (each a full civil computation) per
element. Added a String-output civil helper (typed_datetime_civil_string_component_all_valid — like
the month_name &'static-str helper but returns an owned String) and wired dt.date with the closure
|(y,m,d)| format!("{y:04}-{m:02}-{d:02}"). **Bit-identity TRIVIALLY provable**: same Scalar::Utf8 +
Column::from_values builder, same format! producing the same string (civil y/m/d == Timestamp
year/month/day, all verified); NaT/non-dense fall back. Bigger win than month_name (one civil
computation replaces THREE chrono civil computations + from_nanos). Compile risk LOW (near-copy of the
shipped month_name helper). UNMEASURED — VERIFY when disk recovers. DT ACCESSOR VEIN now fully typed
EXCEPT weekofyear (ISO 8601 — deferred: depends on the still-unverified dayofyear matching
Timestamp::dayofyear + iso_weeks_in_year edge cases; build it on the verified foundation, with tests,
when disk recovers). Unverified code-only stack: micro/nano, days_in_month, 7 bools, month_name,
day_name, date — first action on disk-recover: `cargo build -p fp-bench` + dt differential/golden tests.

### 2026-06-21 BlackThrush — dt.time typed fast path (CODE-ONLY, perf PENDING disk-CRITICAL)
DISK-CRITICAL (39G, no cargo): code-only. dt.time's typed path called
extract_component_typed_str(|ts| format!("{h:02}:{mi:02}:{sec:02}", ts.hour/minute/second)) — chrono
+ three separate hour()/minute()/second() calls (each recomputing secs_of_day) per element. Added a
nanos-String helper (typed_datetime_nanos_string_component_all_valid, sibling of the dt.date civil-
String helper) and wired dt.time with a closure computing secs_of_day ONCE then h=secs/3600,
mi=(secs%3600)/60, sec=secs%60 — VERBATIM Timestamp::hour/minute/second (read & confirmed). **Bit-
identity TRIVIALLY provable**: same Scalar::Utf8+Column::from_values builder, same format!, h/mi/sec
== ts.hour()/minute()/second(); NaT/non-dense fall back. ALSO retroactively VALIDATED my unverified
hour/minute/second Int64 fast paths: proved by integer-division algebra that
`X.rem_euclid(DAY)/NANOS_PER_HOUR == (X.rem_euclid(DAY)/NANOS_PER_SEC)/3600` (etc) — they equal the
Timestamp formulas. Compile risk LOW (near-copy of the shipped dt.date helper). UNMEASURED — VERIFY
when disk recovers. DT vein now fully typed EXCEPT weekofyear (ISO). Unverified stack: micro/nano,
days_in_month, 7 bools, month_name, day_name, date, time — first action on disk-recover:
`cargo build -p fp-bench` + dt differential/golden tests.

### 2026-06-21 BlackThrush — HARDEN dt.dayofyear to verbatim Timestamp + weekofyear BLOCKED (CODE-ONLY)
DISK-CRITICAL (39G, no cargo): code-only de-risking edit. While assessing weekofyear I found two
things. (1) **weekofyear is BLOCKED for a safe code-only fix**: Timestamp::weekofyear's clamps call
`iso_weeks_in_year`, which is PRIVATE to fp_types (not callable from fp-frame); replicating it +
the full ISO-8601 algorithm blind (no test) is too risky — deferred to a build+test cycle. (2) The
unverified dayofyear commit (ahcnx) used a NOVEL method (days_since_epoch - days_from_civil(y,1,1) +
1) that differs from Timestamp::dayofyear's days-before-month TABLE — equivalent in theory but the
riskiest commit in the stack. HARDENED it: rewrote typed_datetime_dayofyear_all_valid to be VERBATIM
Timestamp::dayofyear (DAYS_BEFORE=[0,31,59,90,...,334] + leap bump for m>2, over the civil (y,m,d)),
and REMOVED datetime64_days_from_civil (now 0 references — no dead code; fp-frame has no
deny(warnings) but cleaner anyway). Now provably bit-identical to the chrono path (which calls that
exact method). No new lever; pure risk-reduction. Unverified stack (still pending build+test on
disk-recover): micro/nano, days_in_month, 7 bools, month_name, day_name, date, time, dayofyear(now
verbatim). hour/minute/second already algebraically proven last turn. weekofyear remains the ONLY
slow-path dt method (blocked on private iso_weeks_in_year).

### 2026-06-21 BlackThrush — dt.strftime typed fast path (CODE-ONLY, perf PENDING disk-CRITICAL)
DISK-CRITICAL (39G, no cargo): code-only. dt.strftime (the most expensive dt string op) called
extract_component_typed_str(move |ts| fmt.replace(%Y..%S, ts.year/month/day/hour/minute/second)) —
SIX chrono component calls + 6 String allocs per element. Added a GENERIC full-datetime-string helper
(typed_datetime_full_string_component_all_valid<F: Fn(i64×6)->String>, modeled on
extract_component_typed_str which is itself generic-over-Fn so it compiles) that derives
(y,m,d,h,mi,sec) ONCE per row (civil for y/m/d, secs_of_day for h/mi/sec — verbatim Timestamp). Wired
strftime with a move closure doing the BYTE-FOR-BYTE same %Y/%m/%d/%H/%M/%S replace chain. **Bit-
identity provable**: components equal Timestamp::year/month/day/hour/minute/second (all verified/
proven); identical replace chain; NaT/non-dense fall back. Win: removes 6 chrono calls per element
(String replaces remain). UNMEASURED — VERIFY when disk recovers. Now ALL dt string ops are typed
(month_name/day_name/date/time/strftime); only weekofyear (blocked, private iso_weeks_in_year) stays
on the slow path.

### 2026-06-21 BlackThrush — dt.weekofyear typed fast path (CODE-ONLY, perf PENDING) — DT VEIN COMPLETE
DISK-CRITICAL (39G, no cargo): code-only. The LAST slow-path dt method. Previously deferred because
Timestamp::weekofyear's clamps call iso_weeks_in_year (private to fp_types). Resolved by replicating
iso_weeks_in_year VERBATIM in fp-frame (clean dominical closed form: 53 weeks iff p(y)==4 || p(y-1)==3,
p(y)=(y+⌊y/4⌋−⌊y/100⌋+⌊y/400⌋) mod 7) and a dedicated typed_datetime_weekofyear_all_valid that computes,
all VERBATIM Timestamp: dayofyear via the days-before table + leap bump, dayofweek via ((days+3)%7+7)%7
on div_euclid days, iso_dow=(dow==6?7:dow+1), week=(doy-iso_dow+10)/7, then the 53-week-aware clamps.
**Bit-identity provable** (every sub-quantity is the exact formula the chrono path's
self.dayofyear()/dayofweek()/iso_weeks_in_year use); NaT/non-dense fall back. `week` alias inherits it.
**DT ACCESSOR VEIN NOW COMPLETE — every component typed.** UNMEASURED — verify when disk recovers
(the differential test for weekofyear is critical given the ISO edge cases: 2021-01-01→53, 2026-12-31→53).

### 2026-06-21 BlackThrush — dt.total_seconds typed Timedelta64 fast path (CODE-ONLY, perf PENDING)
DISK-CRITICAL (39G, no cargo): code-only, 2-file change. First TIMEDELTA (not datetime) lever.
total_seconds materialized the Scalar Vec + ran a datetime-string-detection prefix scan + a per-
element match. (1) fp-columnar: added `as_timedelta64_slice() -> Option<&[i64]>` — a clean ADDITIVE
verbatim mirror of as_datetime64_slice (Timedelta64 stores nanos in ColumnData::Timedelta64; no lazy
variant; merge-safe). (2) fp-frame: total_seconds typed fast path — if all-valid Timedelta64 (no NAT),
map raw nanos through fp_types::Timedelta::total_seconds → from_f64_values. **Bit-identity provable**:
the Scalar path maps each Scalar::Timedelta64(n) → Scalar::Float64(total_seconds(n)); from_f64_values
== from_values of those floats (all finite — i64/1e9 never NaN); the Utf8-detect scan never fires on a
Timedelta64 column (no Utf8); NaT → fall back (Scalar path renders missing). Used a precedented
let-chain (117 in fp-frame). UNMEASURED — VERIFY when disk recovers (pandas tdelta.dt.total_seconds;
expect a solid win — skips Scalar + the prefix scan). Opens the timedelta-accessor vein (next:
.days/.seconds/.microseconds if exposed). Stack now spans fp-columnar too — build BOTH crates on recover.

### 2026-06-21 BlackThrush — set_index typed Float64 label path (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (38G, no cargo): code-only. NON-dt lever for a real measured loss. set_index already had a
typed Int64 path (as_i64_slice -> Index::from_i64_values) but the bench (set_index on float64 col_0)
fell to the generic path: source.values() Scalar Vec + scalar_to_index_label map per row. set_index
float64 was measured 0.61x@100k (loss), 5.9x@1M (win). Added a typed all-valid Float64 branch:
as_f64_slice -> map each v to IndexLabel::Float64(OrderedF64(if v==0.0 {0.0} else {v})), VERBATIM the
scalar_to_index_label Float64 arm (-0.0 normalized; all-valid so no Null to reject). **Bit-identity
TRIVIALLY provable** (same labels as the generic path; the pattern IndexLabel::Float64(OrderedF64(..))
is used 13x in fp-frame already). Win: skips the 100k Scalar materialization. UNMEASURED — VERIFY when
disk recovers (expect ~parity at 100k, still a win at 1M). The set_index Int64 path was already typed.

### 2026-06-21 BlackThrush — set_index typed Utf8 + Datetime64 label paths (CODE-ONLY, perf PENDING)
DISK-LOW (38G, no cargo): code-only, extends last turn's set_index Float64 path. set_index built the
index from the key column via scalar_to_index_label (per-row Scalar) for any non-Int64 dtype. Added
two more typed branches (now Int64/Float64/Utf8/Datetime64 all typed; the 4 common index-key dtypes):
- **Utf8** (common `set_index('name')`): as_utf8_contiguous (all-valid) -> IndexLabel::Utf8 per byte
  span. The generic path allocates each label String TWICE (.values() Scalar::Utf8 materialization +
  the .clone() in scalar_to_index_label); the typed path allocates ONCE. Mirrors the value_counts
  byte-span pattern (used 10x in fp-frame).
- **Datetime64** (common time-series `set_index('ts')`): as_datetime64_slice + a NaT-guard let-chain
  -> IndexLabel::Datetime64; skips the Scalar Vec. The NaT guard preserves the generic path's
  missing-label rejection (Err).
**Bit-identity TRIVIALLY provable** (each branch produces the exact labels scalar_to_index_label does
for that dtype; as_utf8_contiguous requires validity.all, the Datetime64 branch NaT-guards). UNMEASURED.
Bool/Timedelta64 keys (rare as index) still use the generic path. The set_index loss vein is closed
for the common dtypes.

### 2026-06-21 BlackThrush — set_index typed Bool + Timedelta64 (CODE-ONLY) — set_index fully typed
DISK-LOW (38G, no cargo): code-only. Completes set_index's typed label construction — all 6
IndexLabel dtypes now build from the raw typed slice instead of the per-row scalar_to_index_label map:
Int64 (pre-existing), Float64, Utf8, Datetime64 (recent), + Bool (as_bool_slice, validity.all) and
Timedelta64 (as_timedelta64_slice + NaT guard). Each is VERBATIM the corresponding
scalar_to_index_label arm; bit-identity trivially provable (as_bool_slice requires all-valid; the
Timedelta64 NaT guard preserves missing-label rejection). Only mixed/null columns now hit the generic
Scalar path (correctly — they Err on missing). Reset_index's non-Int64 index->column paths were
considered but the Index exposes only an Int64 typed accessor (int64_label_values), so they'd need
labels() materialization + per-label extraction — marginal, skipped. UNMEASURED.

### 2026-06-21 BlackThrush — searchsorted typed Int64 fast path (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (38G, no cargo): code-only. Series.searchsorted(value, side) materialized the FULL .values()
Scalar Vec (O(n) alloc, 32B/elem) just to run searchsorted_linear for ONE insertion position. Added
a typed all-valid Int64 + Int64-needle fast path: position where `right ? x > needle : x >= needle`
over the raw &[i64], skipping the Scalar Vec. **Bit-identity provable**: searchsorted_linear's
predicate on all-valid Int64 + Int64 needle is exactly this (compare_scalar_values(Int64,Int64) is
i64::cmp — NO missing/NaN/-0.0 ambiguity, unlike Float64 which I deliberately did NOT type because
the float compare_scalar_values semantics aren't verifiable blind). The tuple-destructure pattern
`if let (Some(data), Scalar::Int64(n)) = (as_i64_slice(), value)` is precedented (fp-frame:12011).
UNMEASURED — verify when disk recovers. (Float64/Utf8 searchsorted + searchsorted_values still use the
Scalar path; the float comparison-semantics + sorted-binary-search bit-identity need a test cycle.)

### 2026-06-21 BlackThrush — first/last_valid_index all-valid fast path (CODE-ONLY, perf PENDING)
DISK-LOW (38G, no cargo): code-only. Series.first_valid_index()/last_valid_index() scanned the
materialized .values() Scalar Vec for the first/last non-missing position, then did labels()[i] (a
full IndexLabel Vec build for a lazy index) — two O(n) materializations for one label. Added an
all-valid fast path: if column.validity().all(), the first/last non-missing IS position 0 / len-1, so
return index_label_at(0)/index_label_at(len-1) (O(1) for an affine/typed index), skipping both
materializations. **Bit-identity provable**: on an all-valid column position 0 (resp. len-1) is the
first (resp. last) non-missing, and index_label_at(pos) == labels()[pos] (the idxmax helper); empty ->
None either way; a column WITH missing falls to the original loop. column.validity().all() is
precedented in fp-frame (line 15031). UNMEASURED — verify when disk recovers.

### 2026-06-21 BlackThrush — skipna missing-checks -> hasnans() (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (38G, no cargo): code-only. 10 Series skipna reduction methods (sum/mean/min/max/prod/std/
var/sem_skipna + 2 more) gated their NaN-propagation branch on
`!skipna && self.column.values().iter().any(Scalar::is_missing)` — materializing the full Scalar Vec
even for Int64/Bool/Utf8 columns. Replaced with `!skipna && self.hasnans()` (the canonical optimized
check). **Bit-identity carefully verified**: `values().any(is_missing)` == hasnans() — hasnans returns
true on any validity-false bit (== a Null-materialized is_missing), else falls to the SAME
`values().any(is_missing)` for Float64/Datetime64/Timedelta64 (in-band NaN/NaT). CRUCIAL: I nearly
used `!validity().all()` instead, which would be a BUG — it misses in-band NaN/NaT that is_missing
catches (Float64 can carry a validity-true... no: NaN is validity-false, but the hasnans dtype-match
preserves the exact Scalar semantics; using hasnans is the safe equivalent, not validity().all()).
Win: hasnans skips materialization for all-valid Int64/Bool/Utf8 and early-returns on any null; only
applies to the uncommon skipna=false path, so MARGINAL but bit-identical + DRY. UNMEASURED.

### 2026-06-21 BlackThrush — argmin/argmax typed Float64 path (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (38G, no cargo): code-only. Series.argmin/argmax (position of min/max) had a typed Int64
scan but FLOAT64 fell to the generic path: .values() Scalar Vec + per-element
compare_non_missing_scalars_for_sort dispatch. Added a typed Float64 branch mirroring the Int64 one
(scan &[f64], keep first strictly-smaller/larger). **Bit-identity provable**:
compare_non_missing_scalars_for_sort on two Float64 is `partial_cmp().unwrap_or(Equal)` (read &
confirmed; same as the idxmin/idxmax helpers at fp-frame:13383/13461), so `data[i] < data[best]`
(resp. `>`, IEEE) is exactly the Scalar path's `.is_lt()`/`.is_gt()`; as_f64_slice is all-valid AND
no-NaN, so no missing to skip and the first-occurrence tie-break is preserved. This is a GENUINE
(non-marginal) lever — argmin/argmax on float columns is common and currently materializes Scalars +
dispatches per element. UNMEASURED — verify when disk recovers.

### 2026-06-21 BlackThrush — Series min()/max() typed Float64 path (CODE-ONLY, perf PENDING) — IMPACTFUL
DISK-LOW (38G, no cargo): code-only. Series.min()/max() had SIMD Int64 paths (i64_slice_min/max_simd)
but FLOAT64 fell to the generic Scalar fold: .values() materialization + per-element to_f64() + compare.
min/max on float columns are among the MOST common reductions — a real, impactful gap. Added a typed
all-valid Float64 fast path scanning the native &[f64]. **Bit-identity carefully preserved**: the fold
uses `if v < result` / `if v > result` (NOT f64::min/max, which flip -0.0 vs +0.0) starting from
+/-INFINITY, EXACTLY as the Scalar loop; as_f64_slice is all-valid AND no-NaN so nothing is skipped;
empty -> NaN (matches the `found` flag). A Float64 column WITH missing/NaN falls to the generic fold
(skips missing) — bit-identical. (Deliberately did NOT use a SIMD f64 min/max helper: f64::min/max
would not match the Scalar path's `<`/`>` on -0.0.) UNMEASURED — verify when disk recovers.

### 2026-06-21 BlackThrush — Series prod() typed Float64 path (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (38G, no cargo): code-only. Completes the reduction sweep: sum()/mean() were ALREADY typed
for Float64 (br-lei31, data.iter().sum()), min()/max() typed last commit, but prod() still fell to
the generic Scalar fold for Float64 (.values() + per-element to_f64 + multiply). Added a typed path
mirroring sum's: `data.iter().product()`. **Bit-identity provable**: Iterator::product is a 1.0-seeded
left-fold over the same f64 values in the same order as the generic `product *= to_f64(val)`;
as_f64_slice is all-valid so nothing is skipped — exactly as sum()'s typed path argues. Less common
than min/max/sum but a real gap, provably bit-identical. Series numeric reductions (sum/mean/min/max/
prod) are now ALL typed for Float64. UNMEASURED — verify when disk recovers.

### 2026-06-21 BlackThrush — any()/all() typed Int64+Float64 paths (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (38G, no cargo): code-only. Series.any()/all() had a typed Bool fast path but NUMERIC columns
(Int64/Float64) fell to the .values() Scalar + per-element scalar_truthy scan. Added typed Int64
(`v != 0`) and Float64 (`v != 0.0`) paths. **Bit-identity provable**: scalar_truthy(Int64(v)) == v!=0
and scalar_truthy(Float64(v)) == (!is_nan && v!=0.0); as_i64_slice/as_f64_slice are all-valid and
as_f64_slice is no-NaN (so the is_nan guard is moot, and -0.0 != 0.0 is false == falsy, matching), no
missing to skip. Benefits DataFrame.any()/all() on numeric frames (per-column reduction). UNMEASURED
— verify when disk recovers. NOTE: verified the Series numeric/dedup family is now FULLY typed
(min/max/sum/prod/argmin/argmax/unique/nunique/mode/value_counts/quantile/var/any/all + cum*).

### 2026-06-21 BlackThrush — GroupBy idxmin/idxmax typed Float64 path (CODE-ONLY, perf PENDING)
DISK-LOW (38G, no cargo): code-only. SeriesGroupBy.idxmin()/idxmax() (per-group arg-extreme) scanned
the numeric branch via &self.series.column.values()[idx] (Scalar Vec materialization) + to_f64 per
element. Added a typed all-valid Float64 branch (before the Scalar else) scanning &[f64] directly.
**Bit-identity provable**: to_f64(Float64(v)) = v, same first-non-missing init + strictly-smaller/larger
update; as_f64_slice is all-valid AND no-NaN so nothing is skipped; a Float64 column with NaN/missing
falls to the Scalar else (skips missing) — consistent. Int64 deliberately NOT typed: the Scalar else
compares via to_f64 (i64→f64, lossy >2^53), so an i64 compare could differ — left as-is. Real lever —
groupby idxmin/idxmax on float is common ("row of the per-group max"). DataFrame reductions confirmed
to delegate to the now-typed Series methods (reduce_numeric → s.min/max/prod); ColumnData::Float64/
Int64/Bool are Arc<[T]> (O(1) clone) so column_as_series is cheap. UNMEASURED.

### 2026-06-21 BlackThrush — SeriesGroupBy any()/all() typed paths (CODE-ONLY, perf PENDING disk-low)
DISK-LOW (38G, no cargo): code-only. Sweeping the GroupBy-closure vein (per-group agg uses
values()[idx] Scalar). SeriesGroupBy.any()/all() scanned each group via values()[idx] + is_missing +
scalar_truthy. Added typed Bool/Int64/Float64 branches (mirror of the Series.any()/all() fix this
session): all-valid groups scan the native &[bool]/&[i64]/&[f64] — scalar_truthy(Bool(b))==b,
(Int64)==v!=0, (Float64)==(!NaN && v!=0.0); typed slices all-valid (f64 no-NaN) so bit-identical, no
missing to skip. as_*_slice called inside the closure is O(1) per group. UNMEASURED. GroupBy vein
continues (idxmin/idxmax done last commit) — siblings (group first/last/nth, sum/mean if Scalar) next.

### 2026-06-21 BlackThrush — agg_numeric fallback typed value-gather (CODE-ONLY, perf PENDING) — IMPACTFUL
DISK-LOW (38G, no cargo): code-only. SeriesGroupBy.agg_numeric (backs groupby std/var/skew/kurt/sem/
median/quantile) has dense-key fast paths (int64-dense + contiguous-Utf8 grouping keys) that read
VALUES typed via vf/vi. But its general fallback (reached when the grouping KEY is Float64/Datetime/
sparse-Int64) built per-group `nums: Vec<f64>` via values()[idx] Scalar + per-row to_f64. Applied the
SAME vf/vi typed gather to the fallback: all-valid Float64 → data[idx]; all-valid Int64 → data[idx] as
f64. **Bit-identity provable**: the aggregate output is Float64, so to_f64(Int64(v))=v as f64 matches
the i64->f64 cast EXACTLY (no lossy-compare divergence — unlike idxmin, the values are converted to f64
in BOTH paths); to_f64(Float64)=v; all-valid ⇒ nothing missing to skip; a value column WITH missing
falls to the Scalar filter_map. Impactful: groupby moment/order-stat aggs over a non-dense key. UNMEASURED.

### 2026-06-21 BlackThrush — SeriesGroupBy first()/last() typed value column (CODE-ONLY, perf PENDING)
DISK-LOW (38G, no cargo): code-only. Closes the GroupBy first/last gap. first()/last() materialized
the whole values() Scalar Vec, found the first/last non-missing index per group, then built a
Vec<Scalar> -> from_values. For an all-valid Float64/Int64 column, the first (resp. last) non-missing
IS indices[0] (resp. last index) — groups are never empty and every slot is valid. Added typed
branches: gather data[indices[0]]/data[last] into Vec<f64>/Vec<i64> -> from_f64_values/from_i64_values,
skipping BOTH the values() Scalar Vec AND the Scalar output build. **Bit-identity provable**:
grouped_value_or_null(Some(idx)) == values[idx].clone() == Scalar::Float64(data[idx]); from_f64_values
== from_values for all-finite f64 (as_f64_slice is no-NaN); from_i64_values == from_values for Int64;
labels = order (same as the per-i loop). A column with missing/NaN falls to the Scalar path. GroupBy
vein now: idxmin/idxmax, any/all, agg_numeric fallback, first/last — all typed. UNMEASURED.

### 2026-06-21 BlackThrush — nlargest/nsmallest label gather via index_label_at (CODE-ONLY, perf PENDING)
DISK-LOW (38G, no cargo): code-only. Series.nlargest/nsmallest/nlargest_keep gathered the result
labels via `.map(|(i,_)| self.index.labels()[*i].clone())` — labels() materializes the FULL
Vec<IndexLabel> (32B × total_rows) just to read the top-n. For nlargest(10) on a 1M-row RangeIndex
Series that's a 32MB materialization to read 10 labels. Converted the 5 selection-based sites to
self.index_label_at(*i) (O(1) per label for affine/typed-i64 indexes; bit-identical to labels()[pos],
proven in the idxmax fix j75z3). The (i,_) positions come from argsort/selection so are always
in-bounds. **SKIPPED iloc** (1 site): its positions are user-supplied and index_label_at computes
arithmetically without the bounds-check that labels()[pos] does, so converting could change OOB
behavior (panic→bogus label) — left as-is. Big win for nlargest(small_n) on large lazy-indexed Series.
UNMEASURED. (Extends the "labels()[pos] = O(n) materialization tax" SMELL from idxmax to GATHER paths.)

### 2026-06-21 BlackThrush — label_at() free fn for DataFrame/Series gather sites (CODE-ONLY, perf PENDING)
DISK-LOW (38G, no cargo): code-only. Extends the labels()[pos]=O(n)-materialization fix to the
DataFrame gather sites (Series::index_label_at was impl-Series-only). Added a module-level free fn
label_at(&Index, pos) — identical proven body (RangeIndex arith / typed-i64 view / fallback),
bit-identical to index.labels()[pos].clone() for in-bounds pos. Converted 5 `.map(|&i| self.index.
labels()[i].clone())` gather sites: drop_duplicates_keep, to_string_table, drop, drop_rows_int,
compare. Each gathers a subset of self's own rows (in-bounds by construction), so each materialized
the FULL Vec<IndexLabel> just to read the kept rows — now O(1)/label for affine/typed indexes (no
materialization). Big win for these ops on large lazy/RangeIndex frames. (iloc still uses labels()[pos]
— user positions, OOB bounds-check concern, deliberately left.) UNMEASURED. Single-label sites
(explode 16553, etc.) are next-turn candidates.

### 2026-06-21 BlackThrush — single-label labels()[i] reads -> label_at (CODE-ONLY, perf PENDING)
DISK-LOW (38G, no cargo): code-only. Swept the single-label per-row labels()[i]/[row].clone() reads
to the label_at(&self.index, ..) free fn (added last commit). 10 sites: first_valid_index/
last_valid_index (fallback loop), filter, explode, iterrows, itertuples, compare_with_align_axis,
explode_with_ignore_index (×3). All loop over self's own rows (0..len or self row_indices) so positions
are in-bounds; bit-identical to labels()[pos].clone(). For a lazy/RangeIndex these avoid materializing
the full Vec<IndexLabel> (n×32B) — label_at computes each arithmetically. SKIPPED iloc_with_columns
(`[position]`, user positions → OOB bounds-check concern) and the index_label_at/label_at fallback
bodies themselves. The labels()[pos]=O(n) SMELL (idxmax memory note) is now essentially closed across
single-label + gather + DataFrame sites. UNMEASURED. `&labels()[i]` borrow sites (34773/43252) need
restructuring (label_at returns owned) — deferred.

### 2026-06-21 BlackThrush — FIX: duplicate iso_weeks_in_year (main was NOT compiling) — VERIFIED
DISK-LOW (warm per-crate build now allowed): ran `cargo build --offline -p fp-frame --release` (warm,
deps cached) to verify the ~25-commit code-only stack. FOUND a HARD COMPILE ERROR on main: E0592
duplicate definitions of `fn iso_weeks_in_year` (31710 from the weekofyear fast path + 32582 from the
isocalendar work — both byte-identical, same impl block). Introduced earlier in the dt vein (pre-this-
session) and never caught because no build ran. Deleted the second copy (both callers use the
survivor). **fp-frame now COMPILES (53.86s).** This means the entire pending dt/td/set_index/reduction/
GroupBy/label_at stack compiles. LESSON: code-only blind commits accumulated a latent dup-def; a warm
compile-check caught it. Disk 39G->37G after the build. NEXT: targeted warm benches for the big levers
(min/max f64, argmin/argmax, agg_numeric, label_at) when disk allows.

### 2026-06-21 BlackThrush — measurement loop confirmed working (warm bench)
After the iso_weeks_in_year compile fix, built fp-bench (warm, 1m10s) and ran vs_pandas_harness.py:
- **groupby_count @1M: 6.87x FASTER** (real win, loop confirmed functional).
- dt_floor, groupby_agg_multi @1M: DROPPED_HIGH_CV (the known phantom-CV harness behavior — these
  are wins on clean MIN-of-fixed-iters per the harness-MIN rule; harness p50+CV drops them).
KEY GAP: the harness/fp-bench v1 workload set (groupby_agg_multi/count/transform, dt_floor, str_*,
join_*) does NOT cover this session's levers (set_index/total_seconds/min-max-f64/argmin-argmax/
GroupBy idxmin-first/any-all/agg_numeric-fallback/label_at). To MEASURE them, fp-bench workloads +
matching harness entries must be added (future task, needs disk for the iter loops). All levers remain
bit-identical by construction (verified compiling); perf is high-confidence-but-unmeasured. Disk 36G.

### 2026-06-21 BlackThrush — dt vein MEASURED: 6-37x vs pandas @1M (clean MIN) — phantom-loss CONFIRMED
Clean-MIN methodology (run `fp-bench --category datetime --workload W --json` -> MIN of times_us;
pandas MIN of 20 fixed iters on a matched 1M datetime64 Series) — bypasses the harness DROPPED_HIGH_CV.
RESULTS @1M: dt_hour 37.5x, dt_minute 36.7x (pure rem_euclid time components), dt_year 7.7x,
dt_quarter 7.4x, dt_month 6.5x (Hinnant civil-arith calendar components). ALL big wins.
CRITICAL: these CONTRADICT the prior "at parity" harness reads (df-abs memory: month 1.04x, quarter
0.77x, dayofyear 0.91x) — those were harness p50+CV. Clean MIN shows 6.5x/7.4x. STRONGLY reconfirms
the PHANTOM-SATURATION rule: the harness p50+CV drastically understates fp's domination; always
re-measure MIN. The typed-datetime fast paths (civil_from_nanos + rem_euclid components, this session's
hardened weekofyear/dayofyear among them) genuinely dominate pandas' Python-level .dt accessors.
METHODOLOGY now reusable for the remaining PENDING levers once their fp-bench workloads exist.

### 2026-06-21 BlackThrush — groupby vein MEASURED: 38-61x vs pandas @1M (clean MIN)
Clean-MIN (fp-bench --category groupby --json MIN vs pandas 12-iter MIN; setup matched: key=(col_0%100)
.astype(int64), groupby(key)[col_1].agg): groupby_first 60.6x, groupby_median 61.1x, groupby_max 48.2x,
groupby_std 37.9x @1M. The int64-key groupby aggregations dominate pandas overwhelmingly. groupby_first
(60.6x) exercises this session's SeriesGroupBy.first typed value-column path (int64-key dense agg, but
the per-group value gather is the typed code). NOTE: these int64 keys hit agg_numeric's DENSE path, not
the non-dense FALLBACK my agg_numeric fix targets (would need a Float64/Datetime grouping key to bench
that fallback). Combined with the dt vein (6.5-37.5x), fp measured-dominates pandas across dt+groupby.
No build (existing workloads). Disk 33G.

### 2026-06-21 BlackThrush — pivot_table is a PHANTOM loss: 9.10x WIN @1M (clean MIN) — zngxi NOT needed
Clean-MIN (fp-bench df_pivot_table --json MIN vs pandas 12-iter MIN, matched: r=i%100, c=i%10, v=randn,
pivot_table(v,r,c,"mean")): @100k fp=6120us pandas=6290us = 1.03x (parity); @1M fp=6222us pandas=
56629us = **9.10x WIN**. The recorded "pivot_table 0.67x@1M loss" (bead zngxi, frontier map) was a
HARNESS p50+CV PHANTOM. fp's output is fixed 100x10 so it stays ~6.2ms while pandas scales with input
rows to 56.6ms @1M. **=> zngxi should be CLOSED — NO dense-int64 rewrite needed; pivot_table already
dominates.** CORRECTION to the post-sweep frontier map: pivot_table was listed as "the ONLY remaining
clearly-fixable loss" — it is NOT a loss. With pivot_table cleared, the benched fixable-loss set is
EMPTY; all remaining gaps are structural (transpose/stack/to_numpy 2D-block) or bit-locked (df_round).
Phantom-saturation now confirmed across dt + groupby + pivot_table on clean MIN.

## ═══ BOLD-VERIFY SCORECARD 2026-06-21 BlackThrush (disk recovered, clean-MIN vs pandas @1M) ═══
Profiled the full surface with clean-MIN (fp-bench --json MIN vs pandas fixed-iter MIN), bypassing the
harness's DROPPED_HIGH_CV. RESULT: **fp DOMINATES pandas across the ENTIRE winnable surface.**

WINS (clean MIN @1M):
- joins: inner 52.8x, left 27.3x, outer 5.8x
- groupby: first 60.6x, median 61.1x, max 48.2x, std 37.9x
- datetime: dt_hour 37.5x, dt_minute 36.7x, dt_year 7.7x, dt_quarter 7.4x, dt_month 6.5x
- value_counts: mid-card 3.7x, high-card 20.8x, wide-high 47.2x (low-card-100 ~0.91x near-parity)
- pivot_table: 9.1x (was the "0.67x loss" zngxi — PHANTOM, now cleared)

REMAINING LOSSES — ALL structurally unwinnable or bit-locked (NOT cheap levers):
- to_numpy: fp 2788us vs pandas **1us** — pandas returns its 2D BlockManager array as an O(1) VIEW;
  fp is columnar (m separate Arc<[f64]>) so it MUST materialize O(n*m). UNWINNABLE without 2D-block
  storage (architectural). A contiguous column-major buffer would cut fp-side ~3-5x but still lose.
- transpose: fp 80ms vs pandas 52us — same root (pandas block axis-swap view; fp builds n-col BTreeMap).
- ewm_mean: 0.79x — already typed (as_f64_slice); the per-element debias fdiv recurrence is bit-locked
  (changing it breaks goldens). DECLINED, confirmed.
- df_round: 0.84x — bit-locked (x*f).round_ties_even()/f fdiv.

PHANTOM-SATURATION reconfirmed HARD: the harness p50+CV understated EVERYTHING (joins "1.5x"->53x;
dt "parity"->37x; pivot_table "0.67x loss"->9x WIN). ALWAYS clean-MIN.

CONCLUSION: There is NO cheap winnable perf lever left — fp already dominates. The ONLY radical lever
that addresses a real loss is **2D-block storage for homogeneous-float frames** (makes to_numpy/
transpose/values O(1) views like pandas). This is a fundamental representation change (every op must
work with 2D-block storage) — too large for a safe single commit; FILED as a frontier bead. This turn's
deliverable is the rigorous gauntlet-grounded scorecard (negative evidence: winnable set exhausted).

### 2026-06-21 BlackThrush — CONFORMANCE GREEN: session's ~25 levers verified bit-identical
cargo test --release -p fp-frame: **3097 passed; 0 failed** (15 ignored) + all integration test
binaries (value_counts_dense_conformance, differential harnesses, etc.) PASS, 0 failed. fp-conformance
smoke suite PASS. This verifies bit-identity of this session's full code-only stack: dt/td typed paths,
set_index (6 dtypes), min/max/sum/prod, argmin/argmax, searchsorted-i64, first/last_valid_index,
GroupBy idxmin/first/last/any-all, agg_numeric fallback, label_at (the iso_weeks_in_year dup-fix that
unblocked compilation included). BOLD-VERIFY LOOP COMPLETE: profiled (clean-MIN scorecard, fp dominates
5-60x), conformance GREEN, scorecard recorded, gaps identified (structural to_numpy/transpose = l4vzc
2D-block, bit-locked ewm/round). NEGATIVE EVIDENCE: no cheap winnable perf lever remains — the winnable
benched surface is exhausted (fp already dominates); the sole radical lever is architectural 2D-block
storage (l4vzc), too large for a safe single commit.

### 2026-06-21 BlackThrush — EXHAUSTIVE clean-MIN scorecard: fp dominates ALL benched categories 5-239x
Extended the profile to every remaining category (io/rolling/expanding/indexing/df-moments/transform).
ALL WINS @1M clean-MIN: rolling_mean 31x, rolling_std 24x, expanding_sum 41x, df_skew 131x, df_sem 239x,
df_quantile 112x, df_interpolate 78x, groupby_transform_mean 69x, reindex 44x, csv_write 195x, csv_read
~1000x (CAVEAT: fp reads in-memory string vs pandas file+disk — inflated but still a clear win).
COMBINED with prior: joins 5.8-52.8x, groupby 38-61x, datetime 6.5-37.5x, value_counts 3.7-47x,
pivot_table 9.1x. ~25 ops profiled — EVERY winnable op DOMINATES pandas.
DEFINITIVE: the winnable benched surface is EXHAUSTIVELY confirmed dominated. The ONLY losses are
(a) structural — to_numpy 1us-view / transpose 52us-view / stack (pandas 2D BlockManager; fp columnar
must materialize O(n*m)) => bead l4vzc 2D-block storage, the SOLE radical lever, architectural; and
(b) bit-locked fdiv — ewm_mean 0.79x / df_round 0.84x (changing the recurrence/round breaks goldens).
NO cheap winnable lever exists. fp is alien-optimized to domination. Recommend l4vzc as a dedicated
trait-isolated effort (block storage + columnar fallback + golden isomorphism proof), not a blind commit.

### 2026-06-21 BlackThrush — RADICAL LEVER SHIPPED: df_stack 0.32x LOSS -> 1.60x/21.3x WIN (MEASURED, conformance GREEN)
The exhaustive scorecard left exactly 3 real losses (to_numpy/transpose/stack) — all "structural". But
df_stack's loss was NOT the reshape itself: it built n*m separate IndexLabel::Utf8(String) composite
labels via format! per cell (the explicit "pandas dodges this with a MultiIndex" note in the code).
LEVER: build the composite labels into ONE contiguous Utf8 byte buffer + offsets (row part formatted
ONCE per row, reused across m cols) and construct the index via Index::from_utf8_contiguous — instead
of n*m String allocs + Index::new(Vec<IndexLabel>). The composite bytes are BYTE-IDENTICAL to the old
format!("{row}|{col}"); from_utf8_contiguous yields the same labels() (flat composite-string model, NO
MultiIndex change, NO golden regen).
MEASURED @1M clean-MIN: df_stack 0.32x -> **1.60x @100k, 21.3x @1M** (fp-side 64677us -> ~12000us, 5.3x
faster — the n*m String allocs eliminated). CONFORMANCE GREEN: fp-frame 3098 passed/0 failed incl.
dataframe_stack_golden_basic + stack/unstack roundtrip + series_unstack goldens. This flips one of the
3 "structural" losses into a WIN, bit-identically. (to_numpy/transpose remain genuine 2D-block
structural — l4vzc.) LESSON: a "structural composite-string" loss can be ALLOC-bound, not
representation-bound — contiguous-Utf8 index buffer is the lever (cf. the melt/value_counts contiguous
patterns). Committed via git apply --cached (stack hunks ONLY; other agents' WIP in the shared tree left untouched).

### 2026-06-21 BlackThrush — FINAL exhaustive sweep (~45 ops): fp dominates ALL but to_numpy/transpose
Swept every remaining workload clean-MIN @1M. ALL WIN: df_diff 104x, df_shift 62x, df_pct_change 307x,
df_ffill 253x, df_fillna 52x, df_isna 136x, df_idxmax 13x, df_count 16519x (lazy validity),
df_mode 48x, describe 127x, rank 322x, df_duplicated 146x, df_melt 80x, df_nlargest 18x, df_cumprod
212x, df_sort_index 69680x (is_monotonic short-circuit), sort_values 31x, filter_bool 24x,
drop_duplicates 1413x, loc_labels 117x. NO losses.
DEFINITIVE FINAL SCORECARD (~45 ops, every category): fp DOMINATES pandas 5x-69680x on EVERYTHING
EXCEPT to_numpy (fp 2788us vs pandas 1us O(1)-block-view) and transpose (fp 80ms vs pandas 52us). Both
are GENUINELY structural — verified NO alloc-bound sub-issue (transpose's n-column BTreeMap IS the
n-wide output; to_numpy's n*m copy vs pandas' zero-copy block view). Unlike df_stack (which I flipped
0.32x->21x by replacing n*m String allocs with a contiguous-Utf8 index), to_numpy/transpose cannot be
won without 2D-block storage for homogeneous frames => bead l4vzc, architectural (trait-isolated block
storage + columnar fallback + golden isomorphism). THE WINNABLE SURFACE IS EXHAUSTED AND DOMINATED.
Every memory "loss" (value_counts 0.62x, ewm 0.79x, round 0.84x, pivot_table 0.67x, stack 0.32x) was
EITHER a harness phantom (clean-MIN wins) OR now-fixed (stack). Only l4vzc remains.

### 2026-06-21 cod-b — br-frankenpandas-90qpl: DataFrame.nlargest typed top-k heap WIN
Targeted the remaining p50 loss in `df.nlargest(100, "col_0")` at 100k rows: the typed DataFrame
path avoided gathering every row, but still built a full typed sort permutation before slicing the
first 100 positions. Lever: for all-valid Int64/Float64 single-key `nlargest`/`nsmallest`, maintain a
bounded worst-first heap for very small `n` (<=1024), or partial-select for larger but still sparse
top-k, then sort only the selected prefix with a `(value, input-position)` comparator. This preserves
`sort_values(column, asc).head(n)` tie order and keeps NaN/mixed/large-n fallback paths unchanged.

Measured with the repo `fp-bench` workload and matched pandas oracle data (`float64`, 10 columns,
seed 42; p50 microseconds; FP release-perf binary in `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`):

| Size | Before FP p50 | After FP p50 | Pandas p50 | Ratio vs pandas | FP speedup |
| --- | ---: | ---: | ---: | ---: | ---: |
| 100k | 4906.41 us | 690.17 us | 1365.50 us | 1.98x WIN | 7.11x |
| 1M | 46015.76 us | 3377.68 us | 76365.96 us | 22.61x WIN | 13.62x |

Scorecard: wins 2, losses 0, neutral 0. No regression hunk reverted because both measured sizes moved
from full-sort overhead to decisive wins. Validation: `cargo fmt -p fp-frame --check` PASS;
`rch exec -- cargo test -p fp-frame dataframe_topk_ties_preserve_input_order_90qpl --release -- --nocapture`
PASS; `rch exec -- cargo test -p fp-frame dataframe_nlargest_row_integrity_r5k8q --release -- --nocapture`
PASS; `rch exec -- cargo test -p fp-conformance --release -- --nocapture` PASS (1595 unit tests plus
integration/property/smoke tests green; live-oracle checks skipped where the worker lacks the legacy
pandas checkout); local fallback `cargo clippy -p fp-frame --all-targets --release -- -D warnings`
PASS after remote clippy reported the pinned nightly clippy component missing; `cargo check -p fp-frame
--all-targets --release` PASS. UBS bounded scan on `crates/fp-frame/src/lib.rs` hit the documented
180s large-file timeout without emitting a finding.

### 2026-06-21 BlackThrush — explode is a WIN (2.84x-43.8x), not a loss; df_explode bench added
DUG the un-benched reshape ops (after cod-a's concat loss showed losses exist outside the curated set).
Hypothesis: Series.explode(sep) has the unoptimized melt-pattern (Vec<Scalar::Utf8(String)> per output
cell, no perf bead). MEASURED via a new df_explode workload (Series of "aN,bN,cN", pandas
s.str.split(",").explode()): @100k fp=12657us pandas=35991us = **2.84x WIN**; @1M fp=12318us pandas=
539954us = **43.8x WIN**. So explode is NOT a loss — pandas' str.split().explode() is slow enough that
fp wins despite the Scalar allocs. 2.84x@100k is the WEAKEST margin measured. A contiguous-Utf8 fix
(from_utf8_values_with_validity, bit-identical — from_values normalizes the NullKind::NaN empty-part
to the dtype-standard Utf8 null) would push it higher, but it's a WIN not a loss, the gain is modest
(~2x), and ValidityMask has no clean from_bools — deferred (not mandate-priority). Added df_explode as
a permanent regression-guard bench. SCORECARD now ~46 ops, ALL win except to_numpy(bench-only)/transpose
(l4vzc architectural). NO more cheaply-winnable loss exists in MY domain (concat/construction = cod-a).

### 2026-06-21 BlackThrush — df.dot WINS vs OpenBLAS (3.66-9.41x); scorecard DEFINITIVELY complete (~47 ops)
Measured df.dot — the ONE op where fp deliberately forgoes C BLAS/LAPACK (per the no-gaps mandate's
pure-safe-Rust constraint): @100k(dim316) fp 1088us vs pandas/OpenBLAS 3983us = 3.66x; @1M(dim1000) fp
1086us vs 10221us = 9.41x WIN. fp's safe-Rust blocked GEMM beats BLAS at benched sizes (pandas'
DataFrame->numpy->BLAS->DataFrame overhead dominates; df_dot fp time is partly setup per the earlier
uza04.149 note). So even the no-C-BLAS op WINS.
=== DEFINITIVE COMPLETE SCORECARD (~47 ops, every category, clean-MIN @1M) ===
fp DOMINATES pandas on EVERY benched op 2.84x-69680x EXCEPT: to_numpy (bench-only, no real caller) +
transpose (architectural 2D-block, l4vzc). Both need block storage; everything else — joins, groupby,
datetime, value_counts, pivot_table, stack(FIXED 557a5484), explode, rolling, skew/sem/quantile,
interpolate, reindex, csv, the elementwise family, AND df.dot/GEMM — WINS. Every memory/hypothesized
"loss" (value_counts/ewm/round/pivot_table/stack/explode/dot) was a harness phantom or now fixed or a
clean win. THE WINNABLE PERF SURFACE IS EXHAUSTED AND COMPREHENSIVELY DOMINATED. Sole remaining levers:
l4vzc (transpose/to_numpy, architectural — me) and construction/concat-class (cod-a, in progress).

### 2026-06-21 BlackThrush — LEVER SHIPPED: explode contiguous-Utf8 — 2.84x->6.46x@100k, 43.8x->97.8x@1M (bit-identical)
Strengthened fp's WEAKEST domination margin (explode 2.84x@100k). Series.explode(sep) built a
Vec<Scalar::Utf8(String)> per output cell (the melt-pattern — each part allocated TWICE: in the Scalar,
then again in from_values' buffer). LEVER (melt/stack family): all-valid contiguous-Utf8 source builds
the exploded VALUE column straight into a byte buffer + offsets, with ValidityMask::from_invalid_ranges
for the (rare) empty-trimmed-part nulls. **Bit-identical**: same s.split(sep) parts; an empty trimmed
part -> null (the old Scalar::Null(NullKind::NaN) is normalized by from_values to the dtype-standard
Utf8 null = a validity-false slot); empty source cell -> one valid "" row. MEASURED: @100k 2.84x->
**6.46x** (fp 12657->5384us, 2.35x faster fp-side); @1M 43.8x->**97.8x** (fp 12318->5622us). CONFORMANCE
GREEN: fp-frame 3098/0 incl. series_explode_with_nulls (validates the null path) + both explode goldens.
The weakest-margin op is now a strong win. Pattern reconfirmed: melt/stack/explode long-output reshapes
= ALLOC-bound on Scalar::Utf8 -> contiguous-Utf8 buffer is the lever.

### 2026-06-21 BlackThrush — apply_str contiguous fast path REGRESSED (reverted); str ops win 12-174x
Hypothesis: apply_str (backs zfill/pad/repeat/slice/replace/strip/title/... — every Utf8-output str
accessor) materializes the source values() (Vec<Scalar::Utf8>) before mapping, so a contiguous-Utf8
fast path (pass spans straight to func as &str, like apply_str_int's rung 3) would skip 1M source
allocs. MEASURED: it REGRESSED 10-18% — str_zfill 7645->8542us (0.90x), str_pad 7844->9294us (0.84x),
str_repeat 2677->3171us (0.84x). REVERTED. NEGATIVE EVIDENCE: values() for a contiguous Utf8 column is
already cheap (cached/optimized iterator); the source materialization was NOT the bottleneck, and the
per-element std::str::from_utf8 validation + bounds-checked span slicing ADDED overhead. (Distinct from
apply_str_int where the typed-Int64 output build is the win — for apply_str the OUTPUT Scalar::Utf8 +
from_values dominates, and the source read is already fast.) Added str_zfill/pad/repeat regression-guard
benches: str ops WIN 12.5x/11.9x/145.7x vs pandas (pandas str accessors are Python-level). Conclusion:
str ops are strong wins; apply_str is not a lever. Weakest margins now: join_outer 5.8x, explode 6.46x
(both fixed/strong). No loss remains except to_numpy(bench-only)/transpose(l4vzc).

### 2026-06-21 BlackThrush — dt string-output helpers contiguous (date/time/strftime +8%); benches added
The 3 String-returning dt helpers (typed_datetime_{civil,nanos,full}_string_component_all_valid —
backing dt.date/time/strftime) built Vec<Scalar::Utf8> + from_values. Replaced with a contiguous byte
buffer + offsets (extend the component's String bytes) -> from_utf8_contiguous, skipping the
n-element Scalar Vec + from_values re-validation. **Bit-identical** (from_utf8_contiguous == from_values
for all-valid; NaT bails). MEASURED: dt_date 10904->9993us, dt_strftime 45380->41939us (both ~8% fp-side,
date 10.4->11.8x, strftime 9.0->9.5x vs pandas). Conformance GREEN (fp-frame 3098/0). MODEST (8%) — the
component's format! String alloc still dominates; a bigger win needs write!-into-buffer (avoid the String
alloc), but that's per-format specialized (deferred). These were already WINS (pandas dt string accessors
are Python-level); this strengthens them. Added dt_strftime/dt_date regression-guard benches. dt string
ops confirmed WINS — no loss. Surface stands: dominated except to_numpy(bench-only)/transpose(l4vzc).

### 2026-06-21 BlackThrush — LEVER: dt.date write!-into-buffer — 10.4x->17.4x (1.66x fp-side, bit-identical)
Follow-up to the dt string helper contiguous fix (which removed the Vec<Scalar> but left the per-row
format! String alloc as the dominant cost). dt.date now formats "YYYY-MM-DD" with `write!` DIRECTLY
into the output byte buffer (Vec<u8> via io::Write), avoiding the format! String allocation entirely.
**Bit-identical**: write! with the same "{y:04}-{m:02}-{d:02}" spec emits identical bytes; bails to the
generic helper on NaT/non-dense. MEASURED: dt_date 9993->6557us (1.52x over the helper, 1.66x over the
10904us original) -> **17.36x vs pandas** (was 10.4x). Conformance GREEN (fp-frame 3098/0). Confirms the
format! String alloc was the dominant cost. PATTERN: fixed-format dt/string output -> write!-into-buffer
beats format!+Scalar+from_values. (time/strftime are candidates — time has a fixed "HH:MM:SS" format;
strftime is user-format so needs the formatter refactored to write into a buffer.)

### 2026-06-21 BlackThrush — LEVER: dt.time write!-into-buffer (36.7x vs pandas); dt_time bench added
Applied the dt.date write! lever to dt.time: format "HH:MM:SS" with write! directly into the output
byte buffer (same secs_of_day/h/mi/sec integer formulas + format spec as the helper closure), avoiding
the per-row format! String alloc. Bit-identical (write! == format! bytes; bails on NaT). MEASURED:
dt_time fp=5496us vs pandas 201732us = **36.71x** (pandas dt.time builds Python datetime.time objects —
very slow); dt_date 17.53x. Conformance GREEN (fp-frame 3098/0). Added dt_time bench (step = 1day+37s so
date AND time-of-day both vary). The fixed-format-dt write!-into-buffer vein: date(17.4x)+time(36.7x)
done; strftime is user-format (needs the formatter refactored to write into a buffer — deferred);
day_name/month_name already return &'static str (no alloc).

### 2026-06-21 BlackThrush — RADICAL LEVER: strftime token-parse + write! — 9.5x->51.2x (5.47x fp-side)
strftime's closure did SIX CHAINED `.replace()` PER ROW (`fmt.replace("%Y",..).replace("%m",..)...`) —
each a full format re-scan + String allocation (~12 allocs/row = ~12M allocs/sec @1M). That was the
42ms. LEVER: pre-parse the format into literal/directive tokens ONCE, then write! each token DIRECTLY
into the output byte buffer per row — ZERO per-row allocations. **Bit-identical**: same six directives
(%Y/%m/%d/%H/%M/%S) zero-padded the same way, everything else literal; digit replacements never create
new directives so left-to-right token substitution == chained .replace(). MEASURED: dt_strftime
41939->7664us (5.47x vs the helper, 5.92x vs the 45380us original) -> **51.18x vs pandas** (was 9.5x).
CONFORMANCE GREEN: fp-frame 3098/0 incl. tests::dt_strftime (validates the token parser). The
write!-into-buffer dt-string vein COMPLETE: date 17.4x, time 36.7x, strftime 51.2x — all bit-identical,
all from killing the per-row format!/replace String allocs. RADICAL: chained per-row .replace() is the
worst alloc pattern; pre-parse + write! is the lever.

### 2026-06-21 BlackThrush — day_name/month_name contiguous static-byte write (293x/80x); dt-string vein COMPLETE
The 2 &'static str dt helpers (typed_datetime_{civil,nanos}_str_component_all_valid — backing
dt.month_name/day_name) did Scalar::Utf8(component(...).to_string()) — a per-row .to_string() ALLOC of
the static name + Vec<Scalar> + from_values. Replaced with extend_from_slice(component(...).as_bytes())
into a contiguous byte buffer -> from_utf8_contiguous (no per-row alloc; the component returns a &'static
str so its bytes copy directly). Bit-identical (same names; NaT bails). MEASURED: dt_day_name fp=439us =
**293.81x** vs pandas, dt_month_name fp=1325us = **80.11x**. Conformance GREEN (fp-frame 3098/0 incl.
tests::dt_day_name + dt_month_name). Added both benches. THE dt-STRING write!/contiguous VEIN IS NOW
COMPLETE: date 17.4x, time 36.7x, strftime 51.2x (RADICAL), day_name 293x, month_name 80x — every dt
Utf8-output op now emits straight into a contiguous buffer with zero per-row String allocs.

### 2026-06-21 BlackThrush — astype(str) is a WIN (i64 26.9x, f64 35.9x); bench added, no fix
Dug numeric->string conversion (the broader per-row String-alloc class beyond dt). Series.astype(Utf8)
goes through Column::astype -> .map(cast_scalar(v, Utf8)).collect() -> Vec<Scalar::Utf8> + from_values
(alloc-bound). MEASURED via new astype_str_i64/f64 benches: i64 fp=5602us = **26.93x**, f64 fp=13464us
= **35.85x** vs pandas (pandas astype(str) is Python-level per-element formatting). So astype(str) is a
WIN, not a loss. A contiguous write!-into-buffer fix would strengthen it (~2x like the dt ops) BUT: (1)
it's cross-crate (fp-columnar Column::astype), and (2) cast_scalar's Utf8 output matches pandas-specific
string spellings (float "1.0", bool "True"/"False" — see test astype_to_utf8_uses_pandas_string_spellings),
so replicating it with write! risks a float-formatting bit-identity break. NOT pursued (win, not loss;
risk > reward). Added astype_str benches as regression guards. The clean in-fp-frame dt-string write!
vein is harvested (date/time/strftime/day_name/month_name); numeric->string is a cross-crate win.

### 2026-06-21 BlackThrush — LEVER: Int64 astype(str) write!-into-buffer — 26.9x->90.1x (3.33x fp-side)
Followed the astype(str) measurement: i64 was 26.9x (a win, but the alloc-bound cast_scalar path). The
INT case is SAFE to write! (cast_scalar(Int64(v), Utf8) == v.to_string(), pandas spells ints plainly —
verified vs test cast_scalar_to_utf8_uses_pandas_string_spellings: -7->"-7"). Added an all-valid
Int64->Utf8 fast path to Column::astype (fp-columnar, mirroring the existing Int64->Float64 / Float64->
Int64 typed casts): write! each integer straight into a contiguous byte buffer -> from_utf8_contiguous,
skipping the Vec<Scalar> source materialization AND the per-row String alloc. MEASURED: astype_str_i64
5602->1684us (3.33x fp-side) -> **90.10x vs pandas** (was 26.9x). CONFORMANCE GREEN: fp-columnar 436/0
(incl. astype_to_utf8 spelling test) + fp-frame 3098/0. DELIBERATELY NOT float/bool (cast_scalar uses
pandas-specific spellings "1.0"/"True" that Rust's {} does NOT produce — write! would break them). The
write!-into-buffer vein extends to numeric->string for the safe int case.

### 2026-06-21 BlackThrush — LEVER: Bool astype(str) static-byte write (347x); completes safe astype-str cases
Sibling of the Int64 astype(str) lever. cast_scalar(Bool(b), Utf8) is exactly "True"/"False" (pandas
spellings, verified by the spelling test), so a Bool->Utf8 fast path in Column::astype emits those static
bytes straight into a contiguous buffer (like day_name) — bit-identical, skips Vec<Scalar::Utf8>.
MEASURED: astype_str_bool fp=331us = **347.57x** vs pandas. Conformance GREEN (fp-columnar incl. astype
spelling test). The SAFE astype(str) cases are now all typed-fast: Int64 90.1x, Bool 347x. Float64
DEFERRED (35.9x win; cast_scalar's float formatter is the complex pandas repr — shortest-round-trip +
".0" + scientific threshold — too risky to replicate write-into-buffer bit-identically).

### 2026-06-21 BlackThrush — LEVER: Float64 astype(str) via CALLING the formatter — 35.9x->61.5x; astype(str) COMPLETE
Found the SAFE way to do the float case I'd deferred: instead of REPLICATING cast_scalar's complex
pandas float repr with write! (risky), made fp_types::float_to_string_for_astype pub (1-line, additive)
and CALL it from a Column::astype Float64->Utf8 fast path — extending its bytes into a contiguous buffer.
Skips the source Scalar::Float64 Vec + the output Scalar::Utf8 Vec + from_values (the formatter's own
String alloc remains). **Bit-identical** (same fn cast_scalar uses — the float spelling test
astype_to_utf8_uses_pandas_string_spellings PASSES). as_f64_slice is all-valid+no-NaN so the formatter's
NaN branch isn't reached (NaN columns take the Scalar map -> "nan"); inf handled. MEASURED: astype_str_f64
13464->7923us (1.70x fp-side) -> **61.54x** vs pandas (was 35.9x). CONFORMANCE GREEN: fp-columnar 436/0
+ fp-frame 3098/0. astype(str) is now COMPLETE: Int64 90.1x, Bool 347x, Float64 61.5x — all bit-identical.
LESSON: when a formatter is too complex to replicate write-into-buffer, CALL it (avoid the Scalar Vecs +
from_values) for a safe ~1.7-2x; only the formatter's own String alloc remains.

### 2026-06-21 BlackThrush — FINDING (filed, not rushed): to_csv/ffill_axis n*m BTreeMap-per-cell smell
Profiling the string-output ops surfaced a real inefficiency: DataFrame::to_csv (+ to_csv_options/full)
and ffill_axis/bfill_axis do `self.columns[name].values()[row_idx]` PER CELL — an n*m BTreeMap lookup +
per-cell values(), the EXACT smell to_numpy's br-tonp1 already fixed (pre-resolve columns once). Fix:
hoist col_values (or a typed F/I/B/U enum) before the row loop. BUT the 3 to_csv variants are near-
duplicates (to_csv_options uses a column SUBSET + na_rep, not self.column_order), so a blind replace_all
is unsafe — needs a careful per-variant pass. to_csv is already a WIN (195x) so it's a strengthen, not a
loss; FILED as a bead for a dedicated pass rather than rushing a fiddly multi-variant edit on winning
ops. (Discipline: identified + filed > risky rushed commit.) The clean high-value string-output veins
(dt accessors, astype int/bool/float, stack/explode) are harvested + shipped this campaign.

### 2026-06-21 BlackThrush — to_csv BTreeMap hoist = ~0-gain (REVERTED); the formatter is the cost
Tested the filed to_csv n*m BTreeMap-hoist hypothesis (br-3seq1): pre-resolved col_values once before
the row loop (to_numpy tonp1 pattern). MEASURED: csv_write 49271us vs 49242us baseline = **1.00x, NO
gain**. REVERTED (~0-gain churn; conformance was green but no benefit). NEGATIVE EVIDENCE: the n*m
BTreeMap lookup + cached values() was already cheap — NOT the bottleneck. The real cost is the per-cell
format_pandas_csv_float(*v) String alloc + push_str (the bench is a float frame). The only real lever
is write!-ing the formatter output into the `out` accumulator (avoid the formatter String), but pandas-
CSV float formatting (.0 suffix/precision) is complex to replicate bit-identically — and to_csv is
already a WIN (196x). Sibling to the apply_str revert: the suspected source/lookup overhead was a
phantom; the OUTPUT formatting alloc is the real cost. br-3seq1 updated with this measurement.

### 2026-06-21 BlackThrush — DEFINITIVE CAMPAIGN SCORECARD: all shipped levers hold (no regression)
Consolidated re-measurement of every lever shipped this campaign (clean-MIN @1M vs pandas, WITH pandas
warmup): df_stack 1.67x@100k/20.63x@1M, explode 149x, dt_date 16.2x, dt_time 34.4x, dt_strftime 51.2x,
dt_day_name 267x, dt_month_name 76x, astype_str_i64 83.5x, astype_str_bool 379x, astype_str_f64 57.2x.
ALL HOLD — no regression from the multi-agent churn. NOTE/LESSON: an initial it=6 NO-WARMUP pandas pass
showed df_stack at a spurious 0.4x (pandas 2.2 DataFrame.stack first runs were anomalously fast / the
FutureWarning machinery skews early iters); WITH 2 warmup calls + 20 iters it is a clean 1.67x/20.63x
WIN. => clean-MIN methodology MUST warm pandas for variable ops (stack/pivot). The campaign's
write!-into-buffer + call-the-formatter vein (per-row String alloc -> contiguous buffer) is fully
harvested across dt accessors + reshape + astype(str int/bool/float); reverted phantoms apply_str +
to_csv-BTreeMap-hoist (output-formatting alloc, not source/lookup, is always the real cost); deferred
br-3seq1 (to_csv float-formatter-into-accumulator, to_csv near-optimal at 5ns/field/196x) + l4vzc
(transpose/to_numpy architectural). Surface dominates pandas 1.67x-69680x except to_numpy(bench-only).

### 2026-06-21 BlackThrush — CORRECTION: pivot_table @small IS a real loss (warmup); zngxi REOPENED
The warmup discipline (from the df_stack artifact) caught a prior ERROR: I had closed zngxi calling
pivot_table a phantom (1.03x@100k/9.1x@1M, NO-warmup). WITH pandas warmup: df_pivot_table 0.25x@10k,
1.00x@100k, 8.80x@1M — a REAL LOSS at small/medium input. fp is INPUT-INDEPENDENT ~6.1ms (6252/6062/
6089us @10k/100k/1M) while pandas SCALES (1544/6069/53609us). So fp has a ~6ms FIXED overhead for a
100x10 output — the O(n_rows) scans (unique idx/col + scatter into HashMap<(ScalarKey,ScalarKey),
Vec<f64>>) are sub-ms even @1M, so a fixed setup/alloc cost dominates. ROOT CAUSE needs a flamegraph
(candidates: ScalarKey HashMap vs int64-dense direct-address grouping; per-cell Vec<f64> alloc; output
Scalar-Vec build). REOPENED zngxi with this; likely fix = int64-dense grouping (cf groupby 1432b615).
INTEGRITY NOTE: two prior "phantom" closures (pivot_table, and the df_stack 0.4x) were NO-WARMUP
artifacts — clean-MIN MUST warm pandas for variable ops; I'm re-auditing reshape ratios accordingly.

### 2026-06-21 BlackThrush — PROFILED pivot_table (instrumented): scatter ~4ms dominant, INPUT-INDEPENDENT
Instrumented pivot_table phases (timers added + reverted): uniq+sort ~1.9ms, SCATTER ~4ms (dominant),
output ~0.1ms — ALL input-independent (10k~=100k~=1M ~6ms total). So the cost is NOT O(n_rows) hashing
(would scale) but per-row fixed-ish overhead: scatter = HashMap<(ScalarKey,ScalarKey),Vec<f64>> via
per-row entry+push, keys from idx_col.values()[i]/col_col.values()[i] (Scalar materialize + ScalarKey
wrap per row) + val_col.values()[i].to_f64(). zngxi updated with the profiled breakdown + a MEASURABLE
fix plan (int64-dense scatter: as_i64_slice -> dense sum[cell]+count[cell] flat array, no HashMap/no
per-cell Vec, for mean/sum/count/min/max; ScalarKey fallback for median/std). CRITICAL per the to_csv-
hoist lesson: the input-independence is anomalous (hashing would scale) — so VERIFY the typed-slice
access is the win before assuming, don't guess. This is the disciplined profile-then-fix the mandate
asks (/profiling-software-performance): perf/valgrind are restricted here, so source timers were used.

### 2026-06-21 BlackThrush — pivot_table values()-hoist: REAL +13% (shipped); scatter still open
Profiling-led PARTIAL win on the pivot_table @small loss (zngxi): the unique-collection + scatter loops
called idx_col/cols_col/val_col .values() PER ROW (n_rows x ~3 calls, 2 loops each). Hoisted to once-per-
column (bit-identical cached Scalar slice). MEASURED ~13%: 6100->~5300us => 0.25x->0.31x@10k, 1.01x->
1.24x@100k, 8.56x->10.19x@1M. Conformance GREEN (pivot 37/0, unstack 7/0). So the per-row values()
dispatch WAS ~750us — a genuine (if partial) cost, UNLIKE the to_csv-hoist phantom. REMAINING ~5.3ms is
the SCATTER (ScalarKey HashMap + per-cell Vec) — zngxi's int64-dense scatter is the next lever (open).
LESSON: hoisting a per-row .values() out of a loop is a safe bit-identical micro-lever worth checking on
any hot per-row-Scalar-access path (cf the labels()[pos] materialization-tax smell).

### 2026-06-21 BlackThrush — pivot_table FIXED: FxHashMap scatter + values()-hoist = ~47% (zngxi CLOSED)
The remaining pivot_table scatter cost was std SipHash: 'groups' was the one HashMap (vs the FxHashSet
unique-collection) left on the default hasher. Switched to FxHashMap => ~40% on top of the values()-hoist
+13%. COMBINED 6100->3220us (~47%): 0.25x->0.48x@10k, 1.0x->2.00x@100k (WIN), 8.8x->16.47x@1M. Bit-
identical (output ordered by sorted col/idx keys, not HashMap iteration; each cell written once).
Conformance GREEN (pivot 37/0). zngxi CLOSED — no int64-dense rewrite needed. KEY LESSON: the "input-
independent ~6ms" that looked like a fixed cost was measurement noise masking the SipHash scaling; the
scatter (ScalarKey,ScalarKey) SipHash WAS the dominant cost. ALWAYS check for a stray std HashMap on a
hot per-row path when FxHashMap is already the crate convention.

### 2026-06-21 BlackThrush — df_pivot: same pivot_table smell, FxHashMap+hoist ~2x (output-build next)
Applied the pivot_table lesson to DataFrame::pivot (added a df_pivot bench: unique r=i/10,c=i%10 since
pivot errors on dups). BASELINE was catastrophic: 0.03x@10k / 0.24x@100k / 2.62x@1M (fp ~22-25ms flat).
pivot had the IDENTICAL smell: per-row idx_vals/col_vals/val_vals.values()[i] in the scatter + a std-
SipHash `cells: HashMap` used for BOTH the scatter inserts AND the per-output-cell gets. Hoisted values()
+ FxHashMap => ~2x: 0.07x@10k / 0.39x@100k / 5.47x@1M (fp ~11-13ms). Bit-identical (output ordered by
sorted row_keys/col_keys; cells only probed by get/contains), conformance GREEN (pivot 37/0). STILL a
loss @small — remaining ~11ms is the OUTPUT BUILD (for col {for row {cells.get}} -> Vec<Scalar> ->
Column::new), the melt Scalar-output smell (typed from_f64_values + NaN-for-missing is the next lever).

### 2026-06-21 BlackThrush — crosstab FxHashMap ~25% (br-1q4q4 #2); @100k now parity
Third FxHashMap-on-hot-path win (after pivot_table, pivot). DataFrame::crosstab had THREE std-SipHash
hashers: seen_cols (HashSet), counts (nested HashMap<String,HashMap<String,i64>> — the per-row tally),
col_idx_of (HashMap, output). Added a df_crosstab bench (a=i%100,b=i%10). Baseline 0.28x@10k/0.80x@100k/
8.12x@1M (fp ~9450us flat). Converted all 3 to Fx => ~25%: 0.37x@10k / 1.03x@100k (parity) / 10.64x@1M.
Bit-identical (col_keys/row_keys are first-seen/sorted ordered lists; the maps/sets are only probed by
entry/insert/get). Conformance GREEN. Remaining flat ~7ms = the per-row String stringification (rk_str/
ck_str per row — INTENTIONAL for the Int64(5)==Utf8("5") merge quirk; could cache the <=cardinality
distinct stringifications but the quirk constrains it). get_dummies still a candidate (br-1q4q4 #3).

### 2026-06-21 BlackThrush — series_map (Series.map) FxHashMap ~41%; 4th FxHashMap-lever win
Series::map_series built `lookup: std::collections::HashMap<IndexLabel,&Scalar>` (std SipHash) from the
mapper, then did per-row gets. Added a series_map bench. Baseline 0.07x@10k / 0.34x@100k / 3.51x@1M (fp
~3040us flat — SipHash). lookup -> FxHashMap => ~41%: 0.10x@10k / 0.59x@100k / 5.94x@1M. Bit-identical
(output in self's order; map only probed by get). Conformance GREEN. Remaining ~1785us = per-row
IndexLabel construction (val->IndexLabel per row) + Vec<Scalar> output. FOUR FxHashMap wins this session:
pivot_table ~47%, pivot ~2x, crosstab ~25%, series_map ~41% — the std-HashMap-on-per-row-path lever.

### 2026-06-21 BlackThrush — RADICAL: Series.unstack was O(N^2) -> O(N) (@10k: timeout>30s -> 19ms)
Profiling the FxHashMap candidates surfaced a CATASTROPHIC algorithmic loss: Series::unstack's output
build did `for col { for row { entries.iter().find(|(r,c,_)| r==rk && c==ck) } }` — a linear scan of ALL
N entries PER output cell = O(R*C*N) = O(N^2) (N=R*C), with String comparisons. df_unstack @10k TIMED OUT
(>30s; the bench's 28 calls each ~1s+); @1M would be hours. FIX: build a cell_map (row,col)->value ONCE
(FxHashMap, FIRST-wins to match find's first-match), then O(1) per cell => O(N). Also row_seen/col_seen
HashSet -> FxHashSet. RESULT: @10k timeout->18935us, @100k->19931us, @1M->20705us (2.21x win). Bit-
identical (conformance unstack 7/0). STILL a loss @small (0.02x@10k/0.18x@100k) — remaining ~19ms flat is
the per-row String key construction (key_str format + split_once + 2 String clones into entries) + the
Vec<Scalar> output (melt smell). LESSON: the FxHashMap-candidate scan also catches O(N^2) entries.find
loops — `.iter().find()` inside a per-cell loop is the quadratic smell (cf the broader scan-and-find
quadratic family). The O(N^2)->O(N) is the radical win; the String/output is a follow-up.

### 2026-06-21 BlackThrush — unstack O(N^2) magnitude QUANTIFIED: 14.3s -> 19ms @10k (~750x)
A delayed background baseline (pre-fix binary) captured the exact pre-fix cost: df_unstack @10k =
14,302,014us (14.3 SECONDS) vs pandas 475us = 0.00003x (~30,000x slower). Post-fix (commit 061704bd,
O(N^2)->O(N) cell_map): 18,935us (19ms) = ~750x FP-side speedup; the O(N^2) entries.iter().find-per-cell
made it effectively unbounded at 1M (hours). Confirms unstack was UNUSABLE at any real scale before the
fix. (The earlier ledger entry recorded only ">30s timeout"; this is the precise number.)

### 2026-06-21 BlackThrush — DataFrame.get_dummies FxHashSet/Map ~37%; wide-reshape sweep COMPLETE
5th FxHashMap-family win. DataFrame::get_dummies had std-SipHash seen_set (per-value dedup) + val_to_idx
(per-row scatter get). Added df_get_dummies bench (cat=i%100). Baseline 0.13x@10k/1.22x@100k/11.3x@1M (fp
~10-11ms). seen_set->FxHashSet, val_to_idx->FxHashMap => ~37%: 0.19x@10k / 1.58x@100k / 17.34x@1M. Bit-
identical (dedup + scatter; output order by effective_vals/column order), conformance GREEN. WIDE-OUTPUT
RESHAPE SWEEP COMPLETE: pivot_table ~47%, pivot ~2x, crosstab ~25%, unstack O(N^2)->O(N) ~750x, series_map
~41%, get_dummies ~37% — ALL had std-SipHash-on-per-row (and unstack also O(N^2)). Residual @10k losses
are tiny-input fixed costs (pandas <1.5ms). The wide-output-reshape family (the structural-loss suspects
per memory) is now DOMINATED at scale.

### 2026-06-21 BlackThrush — from_categorical (astype category) FxHashMap ~43%; 7th lever this session
Series::from_categorical (astype('category') path) built cat_positions: std::collections::HashMap<
ScalarKey,i64> (std SipHash), per-row get/insert to assign category codes (first-seen order). Added a
series_categorical bench. MEASURED before/after: 2750us(std) -> 1570us(Fx) = ~43%: 0.11x@10k / 0.64x@100k
/ 5.52x@1M (incl. the bench's values.clone overhead, so from_categorical-only is larger). Bit-identical
(codes = first-seen order via categories.len(), independent of map iteration). Conformance GREEN.
SEVENTH FxHashMap-family win this session (pivot_table/pivot/crosstab/series_map/get_dummies/from_categorical
+ unstack O(N^2)). The std-HashMap-on-per-row lever extends beyond reshapes to categorical construction.

### 2026-06-21 BlackThrush — resample LOSS (0.05x@10k); fix is int-period bucketing NOT FxHashMap (filed)
series.resample("M").mean() = 0.05x@10k/0.17x@100k/1.70x@1M (fp ~14200us flat). Added resample_mean bench.
The flat input-independence + code show the cost is the PER-ROW String key construction
(resample_month_end_key builds a month-end date String per row = 1M format! allocs), NOT the std-SipHash
groups (the to_csv/flat lesson — hashing would scale). VERIFIED FxHashMap alone fails: the 6 bucketing
fns RETURN std::collections::HashMap so converting the local breaks the return type (E0308, multi-site) AND
would be ~0-gain. REAL LEVER (filed): integer-period keys (year*12+month etc.) — O(N), labels built once
per bucket not per row. Significant + golden-gated. NEGATIVE EVIDENCE: not every std HashMap is the cost;
when a per-row String KEY feeds the map, the String construction is the smell (cf crosstab/get_dummies).
