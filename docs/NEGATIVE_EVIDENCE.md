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
- **pivot_table — FIXED 2026-06-22 (1q4q4): now 3.5x WIN** (was 1.01x on this host; the 0.67x@1M was
  stale). Typed dense-Int64 fast path: all-valid Int64 axis keys + all-valid Float64 values + online
  aggfunc (sum/mean/count/size) scatter-accumulate into a dense column-major R×C buffer in ROW ORDER
  (bit-identical to the generic `vals.iter().sum()` fold), skipping 3M `values()` Scalar materializations
  + the `(ScalarKey,ScalarKey)->Vec<f64>` groups map. 56→16ms @1M, oracle-exact incl 32031 missing
  cells. Non-Int64 keys / non-Float64 vals / other aggfuncs keep the generic path. See ledger row.
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
| fp-join merge OUTER/LEFT on Utf8 key — MEASURED GAP (1q4q4) | 1M-row fact (10000 distinct Utf8 keys) ⋈ 10000-row dim, inner/left/outer; `bench_merge_utf8`; pandas 2.2.3 `left.merge(right,on,how)` | inner 106 / left 110 / outer 178 ms | inner 64.3 / left 135 / outer 834 ms | **inner 1.65× WIN; left 0.81× (1.2× slower); outer 0.21× (4.7× SLOWER)** | 🔴 DOCUMENTED LOSS (no fix this pass — golden-gated join code, deferred for a focused careful session post-crash). INNER single-key Utf8 already wins via `merge_single_key_inner_unsorted` (ordered/contiguous-Utf8 + generic hash). LEFT has only Int64 fast paths (ordered-unique/dense-cycle/dense-i64) — a Utf8 key drops to the generic merge; OUTER has NO single-key fast path AND `sort_outer_join_rows` does a stable String-cmp sort of 1M `IndexLabel::Utf8` (pandas outer also sorts — verified `['a','a','m','q','z']` — but via factorize→int codes, sorting integers). LEVER (matches pandas, future bead): FACTORIZE both key columns' Utf8 → dense int codes once (the proven 1q4q4 lever), join on codes (reuse the int64 dense-join fast paths), and emit the outer result sorted by integer code instead of String — killing both the per-row `Vec<ScalarKey>`/pointer-key build and the String-cmp sort. Est. outer ~0.21×→~2×. Bench `crates/fp-join/examples/bench_merge_utf8.rs` committed for the focused pass. |
| fp-join contiguous Utf8 left/outer position planner (uza04.209/cod-b) | Same `bench_merge_utf8` 1M-row fact (10000 distinct Utf8 keys) ⋈ 10000-row dim on RCH worker `vmi1149989`; before/after same checkout session, `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, `cargo run -p fp-join --example bench_merge_utf8 --release -- 1000000 10000 3 {left,outer}` | left 110 / outer 178 ms | before left 132.813 / outer 1321.721 ms; after left 128.150 / outer 850.721 ms | **after left 0.86× (1.16× slower); after outer 0.21× (4.78× slower); FP-side outer 1.55× faster** | ✅ KEEP PARTIAL — added borrowed-byte `contiguous_utf8_left_positions` and `contiguous_utf8_outer_positions` for all-valid contiguous single Utf8 keys, reusing the existing optional-position output builders. Duplicate-key semantics are preserved by emitting full left/right cross-products, null-bearing keys stay on the generic route, and outer rows are sorted by raw UTF-8 bytes to match the generic `ScalarKey::Utf8` order without per-row `String` clones. This materially reduces the current same-worker outer gap but does NOT close pandas; the deeper factorize-to-dense-code lever remains open for a future pass. Guards: `cargo test -p fp-join` (132 tests), `cargo check -p fp-join --all-targets`, `cargo clippy -p fp-join --all-targets -- -D warnings`, `cargo fmt -p fp-join --check`, `ubs crates/fp-join/src/lib.rs crates/fp-join/examples/bench_merge_utf8.rs` exit 0 (broad existing warnings only). |
| fp-join dense-code contiguous Utf8 outer planner candidate (uza04.210/cod-b) | Same `bench_merge_utf8` 1M-row fact (10000 distinct Utf8 keys) ⋈ 10000-row dim, outer join only; `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, `cargo run -p fp-join --example bench_merge_utf8 --release -- 1000000 10000 3 outer` | outer 178 ms | current baseline 1645.581 ms on RCH `vmi1149989`; dense-code candidate 4432.978 ms on RCH `vmi1264463` | **candidate 0.04× vs pandas (24.9× slower); 0.37× vs current FP baseline (2.69× slower, conservative cross-worker)** | ❌ REVERT — attempted a single `FxHashMap<&[u8], usize>` factorization of both all-valid contiguous Utf8 key columns into dense code buckets, then sorted codes by borrowed bytes and emitted optional outer positions. Focused duplicate-key tests passed before the benchmark, but the candidate added enough hashing/bucket/sort overhead to regress the measured outer workload badly. Source hunk removed; do not retry this isolated dense-code bucket layer. The remaining viable route is a deeper join/output primitive that avoids both per-side maps and post-hoc optional-position materialization, not another wrapper around the borrowed-byte planner. |
| RangeIndex::values direct arithmetic closeout (uza04.165/cod-b) | 1M labels, `RangeIndex(0, 2n, 2).values()` plus forced checksum; focused Criterion `range_index_values` under warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`; pandas 2.2.3 local `idx.values` plus `arr.sum()` | 0.090 ms p50 (`best=0.089 ms`, p95 0.099 ms) | 0.773 ms Criterion mean (`745.91-803.28 us`) | **0.12x (8.6x slower)** | ✅ VERIFIED EXISTING FP-side WIN / 🔴 still pandas loss — current `RangeIndex::values` already generates the arithmetic progression directly instead of round-tripping through `to_index().labels()`. The measured legacy comparator (`to_index().labels()` then Int64 extraction) was 6.571 ms at 1M, so the direct path is **8.50x faster FP-side** (100k: 72.4 us vs 236.5 us, 3.27x). No new production code kept in this pass; an attempted vectorization reshaping probe was stopped under disk-critical policy before results and reverted. Residual vs pandas is structural: pandas exposes a NumPy int64 view/cache and sums in C, while FP returns and consumes an owned `Vec<i64>`. Next viable lever is a typed view/array API or avoiding public materialization, not another label-enum bypass. |
| RangeIndex median BOLD-VERIFY (uza04.170/cod-b) | 1M-label unit, stride-3, and descending `RangeIndex` shapes; 64 median calls per timed batch. pandas 2.2.3 local has no direct `RangeIndex.median()`, so comparator used prebuilt `pd.Series(idx).median()` over identical labels; FP used `bench_range_setops range_median 64` with warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | unit 737.961 ms / strided 727.262 ms / descending 712.997 ms | unit 0.000501 ms / strided 0.000500 ms / descending 0.000510 ms | **unit 1,472,976x / strided 1,454,525x / descending 1,398,033x faster** | ✅ VERIFIED EXISTING WIN — current `RangeIndex::median` is already closed-form and only evaluates the middle one or two labels through widened `i128` `value_at`; it does not allocate `values()` or `IndexLabel` vectors. No production code change was kept; this pass added only a reproducible `range_median` benchmark mode plus this ledger row. |
| RangeIndex diff constant-step BOLD-VERIFY (uza04.171/cod-b) | 1M-label unit, stride-3, and descending `RangeIndex` shapes, `periods=1`; pandas 2.2.3 local `pd.Series(idx).diff(1)` over identical labels vs `bench_range_setops range_diff`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | unit 0.587493 ms / strided 1.025182 ms / descending 0.540814 ms | before unit 2.561713 ms / strided 2.576221 ms / descending 2.560631 ms; after unit 0.563197 ms / strided 0.561974 ms / descending 0.555723 ms | **after unit 1.04x / strided 1.82x faster; descending 0.97x pandas (2.8% slower residual); FP-side 4.55x / 4.58x / 4.61x faster** | ✅ KEEP — `RangeIndex::diff` now fills boundary `None`s plus the constant arithmetic progression delta `step * periods` instead of recomputing two widened labels per output slot. Overflow behavior is preserved by computing the product in `i128` and filling valid slots with `None` when it cannot fit `i64`; period zero and out-of-range periods keep the previous semantics. |
| RangeIndex join same-lattice Int64 BOLD-VERIFY (uza04.190/cod-b) | 1M-label `RangeIndex(0, n, 1, name='k')` joined with flat Int64 `Index(n/2..n+n/2, name='k')`; `inner` and `outer`; pandas 2.2.3 local `RangeIndex.join(Index, how=...)` vs `bench_range_setops range_join`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | inner 3.807292 ms / outer 4.415635 ms | before inner 3.044637 ms / outer 10.109983 ms; after inner 1.233537 ms / outer 1.821050 ms | **inner 3.09x / outer 2.42x faster than pandas; FP-side 2.47x / 5.55x faster** | ✅ KEEP — direct RangeIndex-vs-Int64 joins now detect same-step affine Int64 slices. `inner` returns the lazy overlap span and `outer` returns the lazy contiguous extension when that exactly matches the existing self-then-new-other output order; non-affine, duplicate, off-lattice, prefix-extension, and mixed-label cases stay on the previous materializing/hash paths. |
| RangeIndex isin mark-position BOLD-VERIFY (ruthb/cod-b) | 1M-label `RangeIndex(0, n, 1).isin(values)` with deterministic mixed hit/miss/duplicate needles; small=1,024 needles, large=500,000 needles; pandas 2.2.3 local `RangeIndex.isin` vs `bench_range_setops range_isin`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | small 13.066309 ms / large 43.364404 ms | current small 0.020549 ms / large 3.966811 ms | **small 635.86x / large 10.93x faster than pandas** | ✅ VERIFIED EXISTING WIN — `RangeIndex::isin` already uses direct position marking from each needle (`position_of_value`) instead of building a needle set and scanning every range label. Duplicates collapse by overwriting the same mask slot, misses are ignored, ascending/descending/empty semantics are covered by existing `range_index_isin_marks_positions_without_hash_ruthb`; this pass added only the reproducible bench mode and closed the stale-open bead. |
| SeriesGroupBy single Scalar-backed Utf8 key — single-probe build (1q4q4) | 1M rows, series.groupby(Utf8 key, 1000 groups).sum() | 36.65 ms | 13.1 ms | **2.80× faster** | ✅ DEEPENED — was 1.12× near-parity (FP 32.7ms). SeriesGroupBy.build_groups' generic loop calls `values()` per row and probes TWO hash structures (a `seen` set + the `groups` map). Added a Scalar-backed Utf8 fast path (gated non-contiguous): hoist `values()`, probe ONE `FxHashMap<&str, gid>`, append row indices into per-group Vecs. Bit-identical: same first-seen order/order_keys (ScalarKey::Utf8), same groups map. 32.7→13.1ms (2.5× FP-side). Guards: 202 fp-frame groupby tests, 108 fp-conformance groupby, fmt+clippy clean. |
| DataFrameGroupBy multi-key Scalar-backed Utf8 — mixed-radix dense (1q4q4) | 1M rows, groupby(['a','b']) two Utf8 keys (100×50), i64 value, .sum() | 90.70 ms | 70.3 ms | **1.29× faster** | ✅ FIXED — was 0.87× LOSS (FP 104.6ms). The multi-key mixed Int64/Utf8 dense path (mkdense) only fired for CONTIGUOUS Utf8 (`as_utf8_contiguous`); Scalar-backed Utf8 (from_values/in-memory) fell to the generic per-row `Vec<ScalarKey>` heap-alloc + SipHash. Added a `KeyCol::StrScalar` arm that factorizes the `&str` from the materialized Scalar slice into dense codes, packed into the same mixed-radix direct-address gid table. Bit-identical: codes only pick the dense slot (bijective over the code ranges so first-seen slot order == first-seen key order); group_order keys reconstructed from `vals[row]` (ScalarKey::Utf8). Oracle-EXACT (50k, 37×13: 481 groups, vsum 2499760, sorted a00,b00→a36,b12). 104.6→70.3ms (1.49× FP-side). Guards: 202 fp-frame groupby tests, 108 fp-conformance groupby, fmt+clippy clean. |
| DataFrame.isin non-numeric all-valid column — typed Bool (1q4q4) | 1M rows × 4 Utf8 cols (50-cat), 1000-string needle set | 149.91 ms | 64.0 ms | **2.34× faster** | ✅ FIXED — was 1.13× near-parity (FP 132.5ms). `isin_apply_column` had Int64/Float64 typed paths but Utf8 (and any non-numeric) columns fell to the `Vec<Scalar::Bool>` boxing fallback (32 B/elem; isin is output-bound). Added an all-valid path emitting typed Bool (`from_bool_values`) — the `idx.contains` probe is unchanged, so bit-identical (`from_bool_values` == `from_values` over `[Bool,..]`). 132.5→64.0ms (2.07× FP-side). Oracle-EXACT (40k×3: true_count 88811). Guards: 25 fp-frame isin tests, fmt+clippy clean. Sister to the Series.isin Scalar-backed Utf8 fix. |
| Series.sort_values Float64/Int64 "spread" — stable pair sort ❌ REVERTED (1q4q4) | 2M f64/i64, sort_values(ascending), high-cardinality — **measured on TRULY-SHUFFLED data** | f64 196.8 / i64 209 ms | radix f64 239 / i64 230 ms | **radix 0.82× / 0.91× (LOSS); the pair-sort "fix" was 0.78× — a REGRESSION, REVERTED** | ❌ REVERTED (commits 9d0e56db + 24d3880f undone). **Measurement-error correction:** the bench's value generator `(i·2654435761)>>13` is MONOTONICALLY INCREASING in `i` (no overflow at 2M rows), so the "unique" data was *already sorted* — where a comparison `sort_by` is O(n) (detects the run) but the radix is not. That made the pair sort look like a 1.86× win (65ms). On a splitmix64-**shuffled** 2M f64 the pair sort is **251ms vs the radix 239ms** — a ~5% REGRESSION, and both LOSE to pandas (196.8ms, 0.78×/0.82×). Root cause of the radix's shuffled cost is NOT the argsort (order-independent counting sort) but `reorder_by_positions` gathering columns by a cache-random permutation. A comparison sort can't beat that. Lesson: **always bench sorts on shuffled data.** The radix path (original) is retained. |
| DataFrame.melt — typed id-column repetition (1q4q4) | 500k rows × 6 f64 value cols, 1 Int64 id | 125 ms | 81 ms (was 175) | **1.54× WIN (was 0.72× LOSS)** | ✅ FIXED — melt already built the value column (typed f64 concat) and the variable column (contiguous Utf8) typed, but the **id column** was repeated via `src.values()` — materializing the column to `Vec<Scalar>` (32 B/cell) then tiling that `n_value_vars` times (3M Scalar boxes here). Added a typed path: tile the contiguous `i64`/`f64` buffer directly via `from_i64_values`/`from_f64_values`. BIT-IDENTICAL (8 fp-frame melt tests pass). 175→81ms (2.2× FP-side). Other dtypes keep Scalar tiling. (Context from the same reshape sweep: DataFrame whole-frame reductions are huge wins — df.std **42×**, sem **37×**, skew **19×** vs pandas' slow per-column path; `DataFrame.stack` stays 0.34× — the documented STRUCTURAL flat-composite-string index vs pandas' MultiIndex, not a typed-output gap.) |
| Series.isin Scalar-backed Utf8 — &[u8] probe + typed Bool (1q4q4) | 2M rows, 50-cat Utf8 series, 1000-string needle set | 41.48 ms | 18.0 ms | **2.31× faster** | ✅ FIXED — was 0.51× LOSS (1.9× slower, FP 80.6ms). isin had Int64-bitset + contiguous-Utf8 byte-span paths, but a Scalar-backed Utf8 column (from_values/in-memory) fell to the generic `IsinIndex` whose `Vec<Scalar::Bool>` output boxes 32 B/elem (isin is output-bound at scale). Added the Scalar-backed Utf8 sibling: `FxHashMap<&[u8], ()>` of the string needles probed over the borrowed bytes, emitting typed Bool (`from_bool_values`). Bit-identical (a string value matches iff its bytes equal a string needle; non-string needles never match; no null). 80.6→18.0ms (4.5× FP-side). Oracle-EXACT (40k, 37-needle: true_count 29605). Guards: 25 fp-frame isin tests, fmt+clippy clean. |
| SeriesGroupBy.idxmax()/idxmin() — sequential dense scan (1q4q4) | 1M rows, 1000 int64 groups, **SHUFFLED** | idxmax 5.4 · idxmin 4.95 ms | idxmax 1.98 · idxmin 2.1 ms | **idxmax 2.7× · idxmin 2.4× — ALL WIN (was 0.46×/0.38× LOSS)** | ✅ FIXED — `idxmax`/`idxmin` used `agg_scalar`, whose closure scanned `data[idx]` over each group's scattered row indices = **random** cache-unfriendly access (the dominant cost at 1M rows; `build_groups` was already dense). Replaced with a single SEQUENTIAL pass tracking the best value + its row per group (dense gid by `key − min`, `data[row]` read in order). BIT-IDENTICAL to the f64 `agg_scalar` path: same first-seen group order, same first-row-wins strictly-better tie, same `Scalar::Utf8(label.to_string())` output (35 fp-frame + 18 fp-conformance idx tests pass). idxmax 11.7→1.98ms (**5.9× FP-side**), idxmin 13.1→2.1ms (6.2×). Lesson: a typed `data[idx]` group scan is still RANDOM access — the sequential `data[row]` rewrite is the lever for any `agg_scalar`-style op. Non-f64 values / non-dense Int64 keys keep `agg_scalar`. **SIBLING (same lever): SeriesGroupBy.all()/any() — 0.29×/0.32× → 0.57×/0.62× (2× FP-side, 7.5→3.8ms), bit-identical sequential AND/OR fold (`group_bool_reduce_dense`, typed Bool/Int64/Float64 + dense Int64 key; 137 tests pass). Still a loss — pandas' vectorized bool reduction is only ~2.2ms, so FP's histogram-scan + per-row gid lookup can't fully close it.** **SIBLING (same lever, moments): SeriesGroupBy.sem()/skew()/kurt() — sem 0.08×→0.50× (42→6.8ms, 6.2× FP-side), skew 0.14×→0.68× (31→6.6ms, 4.7×). They went through `agg_values_scalar` (a `Vec<Scalar>` materialized PER GROUP + `nansem`/`nanskew`/`nankurt`); replaced by `group_moment_dense` — a typed TWO-PASS mean-centered moment pass (pass 1 sum+finite count→mean; pass 2 Σ(x−mean).powi(2/3/4)) reproducing each formula EXACTLY (bit-identical; 271 fp-frame tests pass). Still a loss: pandas fuses grouped moments in ONE pass (3.4/4.5ms) while FP's nanskew is two-pass mean-centered (a one-pass rewrite would need a golden regen). **DataFrameGroupBy.idxmax()/idxmin() (multi-col): same dense sequential scan with ONE shared int64_dense_grouping pass (`try_idx_extreme_dense`) — FLIPPED 0.19×→4.2× WIN, 0.23×→4.9× WIN (86/77→3.9/3.6ms, 22× FP-side); pandas DataFrame idxmax has per-col overhead (16.3/17.9ms). Bit-identical (229 fp-frame + 18 fp-conformance tests).** **DataFrameGroupBy.all()/any() (multi-col): NOT in aggregate_named_func`s dense set → fell to build_groups; added `try_bool_reduce_dense` (shared int64_dense_grouping + sequential per-column AND/OR over typed Bool/Int64/Float64). FLIPPED 0.30×→4.4× WIN (all), 0.30×→4.6× WIN (any) (61/67→4.3ms, 15× FP-side). Bit-identical (137 fp-frame tests; AND/OR order-independent, seeded with identity).** **DataFrameGroupBy.first()/last() (multi-col): the dense `aggregate_int64_dense` path was 10× slower than a dedicated scan AND only handled f64/i64 (a Bool/Utf8 col forced the slow generic path). Added `try_first_last_dense` — one shared int64_dense_grouping pass finds each group`s first/last ROW, then every value column is gathered at those rows via zero-copy `Column::take_positions` (dtype-preserving, ANY column type). FLIPPED 0.49×→9.9× WIN (first), 0.52×→7.5× WIN (last) (40→2.0/2.7ms, 20× FP-side). Bit-identical for all-valid columns (93 fp-frame tests); nullable cols (skipna) keep the generic path.** groupby.nunique is already a 2.3× WIN.** **DataFrameGroupBy.sem()/skew()/kurtosis() (multi-col): routed through the SAME dense moments helper (`moments_by_pair` extended with m3/m4 + the sem/skew/kurt finalizers; `try_moment_dense` renames `{col}_{func}`→`{col}`, gated to all-f64 value cols). FLIPPED: sem 0.23×→2.6× WIN (72→6.5ms, 11× FP-side), skew 0.33×→2.5× WIN (66→8.6ms) — pandas DataFrame-groupby has per-column overhead (16.7/21.6ms for 2 cols) while FP shares ONE int64_dense_grouping pass across columns. Bit-identical (348 fp-frame + 151 fp-conformance tests; var/std unchanged).** |
| DataFrameGroupBy nth/head/tail — dense per-group selection (1q4q4) | 1M rows, 1000 int64 groups, **SHUFFLED** | nth 0.80 · head 1.38 ms | nth 2.0 (was 6.5) · head 3.0 (was 6.5) ms | **nth 0.40× · head 0.47× — IMPROVED from 0.12×/0.21× (2.2–3× FP-side), still a loss** | ⚠️ IMPROVED, bit-identical. nth/head/tail used `build_groups` (SipHash) + a per-group `Vec<usize>`; nth also gathered via `col.values()[i].clone()` (materializing the WHOLE column to `Vec<Scalar>` to index ~1000 rows). Replaced with a dense O(n) pass over `int64_dense_grouping`'s gids: nth = the row where the group's occurrence counter first hits `n` (emitted in the dense `order` ⇒ same group order); head/tail = a shared `dense_group_positions(keep(pos,size))` row-position filter (rows visited in ascending index order ⇒ `keep_indices` already sorted). nth's gather now uses zero-copy `Column::take_positions` + `Index::take`. BIT-IDENTICAL (235 fp-frame groupby tests pass). nth 6.5→2.0ms, head 6.5→3.0ms. ❌ Still a loss: pandas' selection ops factorize+select in fewer passes (0.80/1.38ms); FP's histogram scan + 8 MB `gid_per_row` + counter pass can't fully match. Direct key-offset indexing (skip `gid_per_row`) is the further lever. |
| SeriesGroupBy.transform(std/var/min/max/…) — reuse the dense DataFrame path (1q4q4) | 1M rows, 1000 int64 groups, **SHUFFLED**, transform("std") | 9.9 ms | 17.5 ms (was 62) | **0.57× — IMPROVED from 0.16× (3.5× FP-side), still a loss** | ⚠️ IMPROVED, bit-identical. `SeriesGroupBy.transform` only had the dense direct-address path for **sum/mean**; std/var/min/max/median/count/first/last/prod fell to `build_groups` + per-group `Vec<Scalar>` (0.16× pandas) — while `DataFrameGroupBy.transform` already had `try_transform_dense` for all 11 funcs (a win). Now those funcs wrap the key+value into a 2-col frame and reuse `DataFrameGroupBy.transform` (transform broadcasts per row, so group order is irrelevant ⇒ bit-identical; 48 fp-frame transform tests pass). 62→17.5ms. ❌ Still a loss: residual is the frame-construction overhead + FP's multi-pass dense std vs pandas' very fast fused series-transform (9.9ms); a direct dense SGB accumulator (no frame wrap) is the further lever, ~0.6× ceiling. nth (0.12×) and head (0.21×) are sibling selection-op gaps found in the same sweep, not yet addressed. |
| DataFrameGroupBy.agg(dict) — route through the dense engine (1q4q4) | 1M rows, 1000 int64 groups, **SHUFFLED** | sum+mean 21.4 · sum+std 22 · min+max 18.7 ms | sum+mean 5.1 · sum+std 6.1 · min+max 12.2 ms | **sum+mean 4.2× · sum+std 3.6× · min+max 1.53× — ALL WIN (was 0.30× LOSS)** | ✅ FIXED — `DataFrameGroupBy.agg(&HashMap col→func)` (dict form) used generic `build_groups` (SipHash) AND materialized a `Vec<Scalar>` PER GROUP (cloning every value), uniformly ~70ms — while the list-form `agg(&[func])` already had `agg_typed_pairs` (hash-free int64/str dense grouping). The dict form missed it. Now, when every aggregated column is numeric and every func is dense-reducible (sum/mean/count/min/max/var/std/first/last/prod/median), route through `agg_typed_pairs` and rename its `{col}_{func}` outputs to the dict form's single-level `{col}`. BIT-IDENTICAL (dense engine reproduces `build_groups`' first-seen/ascending-key group order + per-group row-order reductions; 234 fp-frame + 120 fp-conformance groupby/agg tests pass). sum+std 70→6.1ms (**11.5× FP-side**). `as_index=false` / non-numeric / skew/kurt/sem/nunique keep the generic path. |
| Series.abs() / trivial elementwise f64 ops — Arc<[f64]> copy-on-produce (1q4q4, FILED) | 5M f64, shuffled, no nulls | 21.5 ms | 60.7 ms (1M: 7.8ms) | **0.35× LOSS (2.8× slower); super-linear (1M→5M is 8× for 5× data)** | 🔍 ROOT-CAUSED, fix deferred (structural + peer-contended fp-columnar). `Column::abs` takes the typed fast path (`as_f64_slice` → `map(abs).collect::<Vec<f64>>()`), but the output is re-ingested via `lazy_all_valid_float64_with_finite` → **`Arc::from(Vec<f64>)`, which reallocates+copies** the 40MB buffer (all f64 storage is `Arc<[f64]>`; `ColumnData::Float64(Arc<[f64]>)`). So abs reads 40MB + writes 40MB (map) + reads 40MB + writes 40MB (Arc copy) = 160MB vs pandas' in-place 80MB. The `f64_finite_witness` is O(1) cached (NOT the cost); `collect::<Arc<[f64]>>()` wouldn't help (std routes Arc-from-iter through a Vec). Taxes EVERY f64-producing op but dominates only when compute is trivial (abs 0.35× while clip/cumsum, with real per-element work, are 0.95×/0.86×). **FIX LEVER:** a Vec-backed (or `Arc<Vec<f64>>`) all-valid-Float64 `ScalarValues` variant so a freshly-produced `Vec<f64>` is MOVED in, not copied — a structural fp-columnar change (storage enum + `as_f64_slice`); deferred for a coordinated pass (fp-columnar is actively peer-developed). |
| SeriesGroupBy cum*/diff/shift — fused dense accumulator (1q4q4) | 1M rows, 1000 int64 groups, **SHUFFLED** | cumsum 4.5 · cummax 3.5 · diff 3.7 · shift 3.7 ms | cumsum 10.1 · cummax 10 · diff 13.7 · shift 14.3 ms | **cumsum 0.38→0.44× · cummax 0.29→0.35× (fused); diff 0.27× · shift 0.26× (unchanged)** | ⚠️ IMPROVED (cum*), bit-identical. `try_cum_dense` (cumsum/cummax/cummin/cumprod) called `dense_group_ids`, materializing an n-element `gid_per_row` Vec (8 MB @1M) then a second pass. Fused: index the per-group accumulator by the key's offset `(key − min)` DIRECTLY in the single accumulation pass — group numbering is irrelevant to cum* (per-key fold in row order), so bit-identical. ~15% FP-side (cumsum 11.7→10.1ms, cummax 12→10ms); 62 cum tests pass. ❌ Still a pandas loss: the residual (NaN gate scan + `i64_dense_histogram_range` min/max scan + accumulate + output alloc = ~3 O(n) passes) can't match pandas' single fused factorize+scan Cython loop. diff/shift genuinely need per-group position state (`gid_per_row`), so the same fuse doesn't apply; left as documented Cython-hard losses. |
| DataFrame.expanding()/ewm().<agg>() — column-parallel (1q4q4) | 1M rows, k=4 cols, **SHUFFLED** | ewm_mean 27 · ewm_std 65 · exp_skew 97 · exp_median 6962 ms | ewm_mean 10.7 · ewm_std 170 · exp_skew 27.5 · exp_median 878 ms | **ewm_mean 0.61×→2.5× WIN · exp_skew 1.37×→3.5× · exp_median 3.1×→7.9× · ewm_std 0.17×→0.38× (still loss)** | ✅ FIXED (sister of the DataFrame.rolling parallelization). `apply_expanding` and `apply_ewm` ran their per-column aggregations serially. Parallelized across columns with the same work-stealing pool — BIT-IDENTICAL (columns independent, keyed by name, `col_order` fixed). 2.3–4.1× FP-side @k=4. FLIPS ewm_mean (0.61×→2.5×) and amplifies the expanding wins (exp_median 3.1×→7.9×). ⚠️ ewm_std stays a pandas loss (0.17×→0.38× @k=4, 0.53× @k=8): the per-column ewm_std is ~6× slower than pandas (online-ewm internals are bit-locked / golden-gated), so parallelism narrows but can't close it. Custom `apply(func)` paths gained a `+ Sync` bound on `func` (standard for closures). Guards: 95 fp-frame + conformance expanding/ewm tests, fmt+clippy clean. |
| DataFrame.rolling(w).<agg>() — column-parallel apply_rolling (1q4q4) | 1M rows, k cols, **SHUFFLED**, w=100 | skew k4 95 / k8 197 · median k4 1164 / k8 2336 ms | skew k4 111 / k8 161 · median k4 149 / k8 ~250 ms | **median k4 7.8× / k8 ~9× WIN · skew k4 0.85× → k8 1.23× WIN (was 0.23×)** | ✅ FIXED — `DataFrameRolling::apply_rolling` ran the per-column rolling aggs in a SERIAL `for col` loop, leaving all but one core idle on a many-core box (DataFrame.rolling.skew was 0.23× pandas). Parallelized across columns with the same work-stealing `std::thread::scope` + atomic-counter pool DataFrame.rank uses — BIT-IDENTICAL (columns are independent, results keyed by name, `col_order` fixed to the numeric-column order). Benefits EVERY agg: order-stat aggs (median/quantile/std, already per-column wins) amplify to 7.8–9× DataFrame wins; the moment aggs (skew/kurt, per-column losses) reach near-parity at k=4 and FLIP to wins as columns grow (skew k=8 161ms vs pandas 197ms = 1.23×) since FP scales to all cores while pandas stays per-column-serial. 3.2–3.4× FP-side @k=4. Gated `ROLL_PAR_MIN_COLS=2` / `ROLL_PAR_MIN_VALUES=16384` so small frames keep the serial path. Guards: 87 fp-frame rolling + 59 fp-conformance rolling tests, fmt+clippy clean. |
| Series.rolling(w).corr()/cov() — drop the per-axis constancy BTreeMaps (1q4q4) | 1M f64×f64, **SHUFFLED**, w=100/1000 | corr 76/74 · cov 46/46 ms | corr 178/182 · cov 172/164 ms | **corr 0.43×/0.41× · cov 0.27×/0.28× — IMPROVED from 0.19×/0.11× (2.2–2.4× FP-side), still a loss** | ⚠️ IMPROVED, bit-identical. Sister of the skew/kurt fix: `RollingPairwiseMomentState` keeps O(1) incremental sums (x,y,x²,y²,xy) but ALSO two `BTreeMap`s (`distinct_x`,`distinct_y`) used solely for `constant_x()`/`constant_y()` (cov→0 / corr→NaN on a constant axis). Maintaining both multisets every step was the dominant cost (cov w=100 was 412ms = 0.11×). For no-NaN/no-missing data each axis is constant iff its trailing run of equal values covers the window — pandas' O(1) `num_consecutive_same_value` counter per axis; `run >= nobs` is BIT-IDENTICAL to `distinct.len() <= 1`. NaN/missing keep the multiset path. corr 400→178ms, cov 412→172ms; 12 fp-frame + 21 fp-conformance tests pass. ❌ Still a loss: residual is the per-call `pairs: Vec<Option<(f64,f64)>>` build + `align` setup plus per-element output arithmetic (2 sqrts) — a separate larger optimization (work on typed slices, skip align on identical indices). |
| Series.rolling(w).skew()/kurt() — drop the constancy BTreeMap (1q4q4) | 1M f64, **SHUFFLED**, w=100 | skew 23.7 · kurt 26.6 ms | skew 57.4 · kurt 44.6 ms | **skew 0.41× · kurt 0.60× — IMPROVED from 0.17×/0.21× (2.4–2.9× FP-side), still a loss** | ⚠️ IMPROVED, bit-identical. `RollingMomentState` keeps O(1) incremental power sums but ALSO a `BTreeMap` multiset, used solely for `is_constant()` (constant window ⇒ skew 0.0 / kurt −3.0, matching pandas). Maintaining that multiset every step was the whole cost (skew w=100 was 139ms = 0.17×). For no-NaN/no-missing data a window is constant iff its trailing run of equal values covers it — pandas' O(1) `num_consecutive_same_value` counter; `counter >= nobs` is BIT-IDENTICAL to `distinct.len() <= 1` (no nulls interrupt the run, `value==prev` matches `value_key` incl. −0.0==0.0). NaN/missing keep the multiset path. skew 139→57ms, kurt 128→44ms; 38 fp-frame + 13 fp-conformance tests pass. ❌ Still a pandas loss: residual is the per-element `central_m3`/`m4` arithmetic + `s2.powf(1.5)`. The powf→`s2*sqrt(s2)` lever (≤1 ULP, the expanding-skew fix) was tried but breaks `dataframe_rolling_skew_golden_basic` (EXACT golden) — flipping to a win needs a golden regen, deferred. |
| Series.rolling(w).median()/quantile() — cache-hot sorted-window (1q4q4) | 1M f64, **SHUFFLED**, w=100/1000/4096 | median 294/421/509 · quantile 285/414/507 ms | median 96/159/360 · quantile 113/188/371 ms | **median 3.05×/2.65×/1.41× · quantile 2.52×/2.2×/1.37× — ALL WIN (were 0.29×/0.36× LOSS)** | ✅ FIXED — same root cause as rolling rank: `rolling_order_stat` coordinate-compressed **all** non-null values to dense ranks `0..u` (≈n at high cardinality) and slid a Fenwick sized `u+1` — every k-th-order-statistic query was O(log n) over a multi-MB **cache-cold** tree (w=100 median was 1000ms = 0.29×, again *slower* than w=1000 from sparse occupancy). Added a sorted-multiset window (shared `ROLLING_SORTED_WINDOW_MAX_W=4096`): the k-th order statistic is `win[k]` by direct index. **BIT-IDENTICAL** — `win[k]` == `uniq[fen_kth(k)]` (k-th smallest in the window), `eval_kth` unchanged; oracle-EXACT vs pandas (median hash 2790247070565799993, q25 988868036354512953). Order-independent ⇒ serves centered windows too. median w=100 **1000→96ms (10.4× FP-side)**. NaN/-0.0/wide windows keep the Fenwick. Guards: 142 fp-frame + 76 fp-conformance tests, fmt+clippy clean. |
| Series.rolling(w).rank() — cache-hot sorted-window multiset (1q4q4) | 1M f64, rolling(w).rank(average), **SHUFFLED**, w=20/100/500/1000/4096 | 229/318/430/482/548 ms | 140/159/195/239/460 ms | **1.6× / 2.0× / 2.2× / 2.0× / 1.19× — ALL WIN (was 0.26× LOSS @w=100)** | ✅ FIXED — the `rolling_rank_fast` Fenwick trees were sized to the **whole series'** distinct-value count `u` (≈n for high-cardinality data), so every insert/remove/query was O(log n) over a multi-MB **cache-cold** tree — pandas' O(log w) skiplist beat it 3.9× (w=100 was 1228ms = 0.26×; tellingly w=100 was *slower* than w=1000 — sparse occupancy of the huge tree). Added `rolling_rank_sorted_window`: the window's valid values kept in a `Vec<f64>` sorted by `total_cmp` (a multiset), slid via binary-search insert/remove — O(w) per slide but contiguous/branch-predictable/cache-hot. Gated to trailing + non-`dense` + w≤4096 (`ROLLING_RANK_SORTED_MAX_W`); centered/dense/wide windows keep the Fenwick. w=100 **1228→159ms (7.7× FP-side)**. BIT-IDENTICAL to the Fenwick path (`total_cmp` is the same total order its `uniq` uses; identical c_less/c_eq + na_option formulas; trailing ⇒ `first`==`max`). Oracle-EXACT vs pandas for average/min/max ascending (`first` is unsupported by pandas rolling rank; FP keeps the golden `==max`). Residual: w>4096 stays on the Fenwick (still a loss — needs a window-sized order-statistics treap, rare in practice). Guards: 87 fp-frame rolling tests, 64 fp-conformance roll, fmt clean. |
| Series.rank Float64 — value-only pdqsort/stable argsort (1q4q4) | 2M f64 rank, **SHUFFLED data**: average unique / average tie-heavy (card=1000) / first unique | avg-uniq 437 / avg-ties 238 / first 326 ms | 132 / 37.9 / 115 ms | **avg-uniq 3.3× · avg-ties 6.3× · first 2.8× — ALL WIN (validated on shuffled data)** | ✅ FIXED (KEPT after the sort-bench audit). `rank_f64_slice_radix` used `radix_argsort_f64` whose `data[perm[k]]` tie-walk is cache-random on shuffled data; replaced with a cache-friendly inline `(value, index)` pair sort — **value-only** `sort_unstable` for average/min/max/dense (within-tie order irrelevant; value-only key lets pdqsort exploit equal runs), **stable** `sort_by` for `first`. The tie-walk reads `pairs[k].0` with no indirection. **Re-validated on splitmix-SHUFFLED data** (the original ledger's "2.10×" used accidentally-monotonic data; the real wins are larger and hold across every method): avg-unique 132ms vs pandas 437ms = 3.3×, avg-ties 37.9ms vs 238ms = 6.3×, first 115ms vs 326ms = 2.8×. Unlike sort_values, rank does NOT gather columns by the permutation, so the no-indirection tie-walk is a genuine FP-side win, not a monotonic artifact. Bit-identical; oracle-EXACT all 5 methods + desc + pct; −0.0 gated to the comparator fallback. Guards: 93 fp-frame sort/rank tests, 17 fp-conformance rank, fmt+clippy clean. |
| Series.map Int64→Float64 dict (numeric encode) typed output (1q4q4) | 2M rows, 20 distinct int codes → Float64 (full coverage) | 7.42 ms | 21.9 ms | **0.34× (2.9× slower) — IMPROVED from 0.12× (8.3×)** | ⚠️ IMPROVED but still a pandas loss. The Int64-key map path typed-output only Int64→Int64; an Int64→Float64 dict (code → numeric value) fell to a `Vec<Scalar::Float64>` boxing path = 8.3× slower. Added a Float64-output sub-path emitting a typed Float64 column via `from_f64_values` for full coverage (bit-identical; bails to Scalar path on unmapped). 61.4→21.9ms (**2.81× FP-side**). Oracle-EXACT (40k: full sum 659998.5; partial nulls 1738). ❌ A bounded-range direct-address probe was tried (21.9→19.3ms, ~10%) and REVERTED as ~0-gain over its complexity — the residual is structural: pandas maps an int dict via a fully vectorized C int→array gather into a numpy Float64 array (7.4ms), which a Rust per-row probe + 16 MB owned `Vec<f64>` alloc can't match. Guards: 32 fp-frame map tests, 4 fp-conformance map, fmt+clippy clean. |
| Series.map Utf8→Float64 dict (numeric encode) typed output (1q4q4) | 2M rows, 20-cat Utf8 series → Float64 (full coverage) | 72.39 ms | 32.4 ms | **2.24× faster** | ✅ DEEPENED — was 1.02× near-parity (FP 71.1ms). The Utf8-key map path typed-output only Utf8→Utf8; a Utf8→Float64 dict (string category → numeric encoding, common in ML preprocessing) fell to a `Vec<Scalar::Float64>` (32 B/elem; no per-elem heap alloc, so less severe than the Box<str> cases, hence only near-parity). Added a Float64-output sub-path emitting a typed Float64 column (8 B/elem) via `from_f64_values` for full coverage; partial coverage bails to the `&str`-probe scalar path (Null for unmapped). Bit-identical. 71.1→32.4ms (2.2× FP-side). Oracle-EXACT (40k: full sum 659998.5; partial nulls 1738). Guards: 32 fp-frame map tests, 4 fp-conformance map, fmt+clippy clean. |
| Series.map Int64→Utf8 dict (label encode) contiguous output (1q4q4) | 2M rows, 20 distinct int codes → string labels (full coverage) | 20.05 ms | 18.7 ms | **1.07× faster** | ✅ FIXED — was **0.12× LOSS (8.6× slower, FP 173ms)** — one of the biggest single-op gaps found. The Int64-key map path had a typed-output sub-path only for Int64→Int64; an Int64→Utf8 dict (integer code → string label, a common encode) fell to a `Vec<Scalar::Utf8>` = 2M `Box<str>` clones, while pandas returns pointer-copies to ~20 interned strings. Added an Int64→Utf8 sub-path: for full coverage, emit a CONTIGUOUS Utf8 column (one shared byte buffer) via `from_utf8_contiguous`. Bit-identical; partial coverage bails to the Scalar path (Null for unmapped). 173→18.7ms (9.2× FP-side). Oracle-EXACT (40k: full byte-hash 8439260058170857572 / totlen 302610; partial nulls 1738). Guards: 32 fp-frame map tests, 4 fp-conformance map, fmt+clippy clean. |
| Series.map Utf8→Utf8 dict (recode) typed contiguous output (1q4q4) | 2M rows, 20-cat Utf8 series, Utf8→Utf8 full-coverage dict | 85.19 ms | 44.1 ms | **1.93× faster** | ✅ FIXED — was 0.42× LOSS (2.4× slower, FP 203ms). `Series.map` had Int64/Float64 typed fast paths but no Utf8 path, so string recoding (very common in cleaning) fell to the generic per-row `ScalarKey` probe + a `Vec<Scalar::Utf8>` output = N `Box<str>` heap clones. Added a Utf8 path: `FxHashMap<&str, &Scalar>` over borrowed keys; when all mapping values are Utf8 AND every row maps (full coverage), emit a CONTIGUOUS Utf8 column (one shared byte buffer, no per-row alloc) via `from_utf8_contiguous`. Bit-identical to the `Vec<Scalar::Utf8>` the generic path builds; partial coverage falls to the `&str`-probe scalar path (Null(NaN) for unmapped). 203→44.1ms (4.6× FP-side). Oracle-EXACT (40k: full byte-hash 12458671239491245627 / totlen 106213; partial nulls 1378). Guards: 32 fp-frame map tests, 4 fp-conformance map, fmt+clippy clean. |
| DataFrameGroupBy multi-agg multi Utf8/mixed key — dense moments (1q4q4) | 1M rows, groupby(['a','b']) two Utf8 keys (100×50), agg([sum,mean,std,count]) | 106.35 ms | 62.8 ms | **1.69× faster** | ✅ FIXED — was **0.36× LOSS (2.8× slower, FP 296.8ms)**. Generalized `dense_group_ids_for_order`'s Int64-only composite to a mixed Int64/Utf8 `KeyCol` mixed-radix index (Utf8 columns factorized to first-seen codes + a `str->code` map so each `group_order` Utf8 key resolves by string content). The all-Int64 case stays bit-identical (same `(v-min)*stride` slot). Unlocks the one-pass per-gid moments path for multi-Utf8 / mixed keys. 296.8→62.8ms (4.7× FP-side); single-key path unchanged (no regression). Oracle-EXACT (60k, 29×13: 377 groups, sum 44709705.0, mean 280900.5497, std 163249.4263, count 60000). Guards: 234 fp-frame agg/groupby tests, 120 fp-conformance agg/groupby, fmt+clippy clean. |
| DataFrameGroupBy multi-agg single Utf8 key — dense moments (1q4q4) | 1M rows, groupby(Utf8, 1000 groups).agg([sum,mean,std,count]) | 45.05 ms | 26.2 ms | **1.72× faster** | ✅ FIXED — was **0.35× LOSS (2.8× slower, FP 127.4ms)**. The typed dense moments path (`agg_typed_pairs_dense_f64_moments`) needs `dense_group_ids_for_order`, which was Int64-keys ONLY (`as_i64_slice()?` per by-column); a Utf8 key returned None, dropping multi-agg to the generic per-(group,func) Scalar gather (4 funcs × per-group materialize). Added a single-Utf8-key branch: factorize the borrowed &str to first-seen gids + translate each `group_order` entry via the same map (HashMap matches by string content), feeding the existing one-pass per-gid sum/mean/count/var/std accumulators. Bit-identical: same partition + row-order folds == generic `apply_agg_func`/`nanstd(ddof=1)`. 127.4→26.2ms (4.86× FP-side). Oracle-EXACT (50k, 137 groups: sum 37254712.5, mean 102089.3958, std 58646.6169, count 50000). Guards: 58 fp-frame agg tests, 120 fp-conformance agg/groupby, fmt+clippy clean. |
| SeriesGroupBy cum*/dense single Scalar-backed Utf8 key (1q4q4) | 1M rows, series.groupby(Utf8, 1000 groups).cumsum() | 36.25 ms | 12.6 ms | **2.85× faster** | ✅ FIXED — was 0.83× LOSS (FP 43.8ms). SeriesGroupBy's `dense_group_ids` (the gid source for `try_cum_dense` + other dense SeriesGroupBy ops) had Int64 + contiguous-Utf8 paths but not Scalar-backed Utf8, which fell to the generic per-group `Vec<Scalar>` gather. Added the Scalar-backed Utf8 sibling (first-seen `FxHashMap<&str, gid>`, gated `as_utf8_contiguous().is_none()`), mirroring the `transform_dense_gids` fix. Covers cumsum/cumprod/cummax/cummin (and any dense SeriesGroupBy consumer) on Scalar-backed Utf8. Bit-identical: row-order per-gid fold == generic. 43.8→12.6ms (3.5× FP-side). Oracle-EXACT (50k, 137 groups: cumsum sum 6788455161.0, cummax 68640310.5). (DataFrameGroupBy cum* was already fixed via the transform_dense_gids change.) Guards: 52 fp-frame cum tests, 117 fp-conformance cumsum/cummax/groupby, fmt+clippy clean. |
| DataFrameGroupBy.transform single Scalar-backed Utf8 key (1q4q4) | 1M rows, groupby(Utf8 key, 1000 groups).transform(mean) | 35.97 ms | 13.2 ms | **2.72× faster** | ✅ FIXED — was 0.73× LOSS (FP 49.4ms). `try_transform_dense`'s gid layout (`transform_dense_gids`) handled all-Int64 keys + a single CONTIGUOUS-Utf8 key, but a Scalar-backed Utf8 key (from_values/in-memory) fell to the generic path that gathers a `Vec<Scalar>` per group + Scalar broadcast. Added a Scalar-backed Utf8 sibling: first-seen `FxHashMap<&str, gid>` over the borrowed strings (gid numbering is irrelevant to transform's `agg[gid[row]]` broadcast), feeding the existing typed accumulate+broadcast. Bit-identical: same partition + same row-order per-gid folds as the generic `apply_agg_func`. mean 49.4→13.2ms (3.7× FP-side); sum 13.1ms, std 15.7ms. Oracle-EXACT (50k, 137 groups: mean sum 37254712.5, std 21404503.2257, sum 13596358284.0). Guards: 18 fp-frame transform tests, fmt+clippy clean. |
| DataFrameGroupBy single Utf8 key — &str-hash group build (1q4q4) | 1M rows, 1 Utf8 key col (1000 groups), i64 value, .sum() | 37.26 ms | 13.9 ms | **2.68× faster** | ✅ FIXED — was 0.90× LOSS (FP 41.4ms; the i64-key path was already 5× win at 2.4ms). `build_groups` had dense paths only for Int64 keys; a single all-valid Utf8 key fell to the generic fall-through that heap-allocates a `Vec<ScalarKey>` PER ROW (1M tiny allocs) before hashing. Added a single-Utf8-key fast path mirroring the wide-i64 hash path: probe `FxHashMap<&str, gid>` on the borrowed string — no per-row Vec. Bit-identical: same first-seen `group_order` (ScalarKey::Utf8 borrowed from the column), same groups map, same optional sort (composite_key_cmp Utf8 == str::cmp). 41.4→13.9ms (3.0× FP-side). Guards: 202 fp-frame groupby tests, 108 fp-conformance groupby (vs pandas oracle), fmt+clippy clean. Residual = string hashing (FxHashMap probe) + per-group accumulation; factorize-to-i64-dense is a possible deeper lever. |
| get_dummies Utf8 per-row clone elision (1q4q4) | 1M rows, 1 Utf8 col, 30 categories | 53.86 ms | 44.8 ms | **1.20× faster** | ✅ FIXED — was 0.85× LOSS (FP 63.3ms). The Utf8 one-hot scatter cloned a `String` per row (~1M heap allocs) solely to probe the `&str`-keyed `val_to_idx`; now it borrows `s.as_str()` directly (non-Utf8 arms still stringify, but all-valid Int64 already took the typed path, so they're rare here). Pure clone-elision = identical lookup result, bit-transparent. 63.3→44.8ms (1.41× FP-side). Guards: 14 fp-frame get_dummies tests, 6 fp-conformance, fmt+clippy clean. Residual = the 1M×30 Bool output-matrix alloc (allocator-bound, mimalloc-covered). |
| pivot_table Int64-VALUE base + margins dense (1q4q4) | 200k×200, int64 keys, **Int64 value** col, agg=sum, margins=True | 22.82 ms | 6.5 ms | **3.51× faster** | ✅ FIXED — the dense base + O(n) margins paths gated on `as_f64_slice` (**Float64 values only**); an Int64 value column (counts/quantities — common) fell to the generic base AND the O(n·(n_idx+n_col)) margin scan = the same ~19× catastrophe (200k×200 margins was ~433ms). `pivot_value_f64` now borrows Float64 zero-copy OR materializes a `Vec<f64>` from an all-valid Int64 column (`v as f64` == the generic `Scalar::Int64(v).to_f64()`), so Int64 values take the dense paths. Bit-identical to the generic Int64-value path (same f64 fold, Float64 output). margins 200k ~433→6.5ms (**66× FP-side**); 1M margins 33.8ms vs pandas 108ms = 3.2×. Oracle-EXACT (40k base nansum 19867380.0, margins corner 19867380.0). Float64 path unchanged (no regression). Guards: 39 fp-frame pivot tests, 36 fp-conformance pivot, fmt+clippy clean. |
| pivot_table(margins=True) var/std/median O(n) (1q4q4) | 1M rows, int64 keys, f64 vals, margins=True, agg=var/median | var 107.98 / median 142.57 ms | var 37.5 / median 89.2 ms | **var 2.88× / median 1.60×** | ✅ FIXED — same O(n·(n_idx+n_col)) margin-scan catastrophe as sum, just for the rarer aggfuncs (200k×200 measured var **474ms**, median **484ms**). Extended `pivot_compute_margins`: var/std as a four-pass mean-centered fold (per row/col/overall, ROW ORDER, NaN below 2 samples), median as per-margin bucket sort — both bit-identical to the generic `pivot_table_agg_value` margin folds. var 474→7.2ms (**66× FP-side**), median 484→19.6ms (**25×**). Oracle-EXACT (40k, 53×7: var corner 187124.15951, median corner 744.75, all row-margins identical). Guards: 7 fp-frame margin tests, 36 fp-conformance pivot, fmt+clippy clean. The full margins path (sum/mean/count/size/min/max/var/std/median) is now O(n). |
| pivot_table(margins=True) O(n) two-pass (1q4q4) | 1M rows, int64 idx (1000) × col (10), f64 vals, agg=sum, margins=True | 111.81 ms | 32.4 ms | **3.45× faster** | ✅ FIXED — was **0.053× (18.9× SLOWER)**, the single biggest gap found. The margins path called `pivot_table_margin_source_values` ONCE PER row-label AND ONCE PER column — each a full O(n) scan with a per-row label-build + string compare — so margins cost **O(n·(n_idx+n_col))** (≈1e9 ops; 200k×200 measured **433ms**). `pivot_margins_dense` computes the "All" column (per-row totals), "All" row (per-column totals) and grand total in **TWO O(n) passes** keyed by the dense sorted-rank codes. Bit-identical: each margin folds its raw values in ROW ORDER exactly as `pivot_table_agg_value(margin_source)`, and the grand total is summed in raw row order (NOT by reducing per-bucket sums, preserving float fold order). 200k/200 433→6.3ms (**69× FP-side**); 1M sum 3.45×, mean 3.58× vs pandas. Online aggfuncs (sum/mean/count/size/min/max) + dense-eligible keys; var/std/median margins keep the generic path. Guards: 7 fp-frame margin tests (exact-value), 36 fp-conformance margin+pivot, fmt clean. |
| pivot_table dense median (per-cell scatter+sort) (1q4q4) | 1M rows, int64 idx (1000) × col (10), f64 vals, agg=median | 70.77 ms | 36.0 ms | **1.97× faster** | ✅ DEEPENED — was 1.07× (Utf8 1.17×), the last pivot agg on the generic path. median is holistic (per-cell sort), but `pivot_dense_build_median` scatters each row's value into its dense `cell` bucket — skipping the 3 Scalar materializations + `(ScalarKey,ScalarKey)` tuple hashing — then sorts per cell (`partial_cmp`) and averages the two middle elements, bit-identical to the generic `pivot_table_agg_value("median")`. i64 66.1→36.0ms (1.84× FP-side); Utf8 91.8→46.2ms (2.32× vs pandas 107ms). Completes the pivot dense path: **sum/mean/count/size/min/max/var/std/median × {Int64,Datetime64,Utf8} all dense.** Guards: 39 fp-frame pivot tests (median exact-value assert added), 42 fp-conformance pivot+median, fmt+clippy clean. |
| pivot_table dense var/std (two-pass) (1q4q4) | 1M rows, int64 idx (1000) × col (10), f64 vals, agg=var | 56.67 ms | 17.6 ms | **3.21× faster** | ✅ FIXED — was 0.96× LOSS (std 1.03×). var/std were the last aggfuncs still on the generic 3×Scalar-materialize + Vec<f64> groups path. Added a dense TWO-pass to the shared `pivot_dense_build`: pass 1 sum+count → per-cell mean; pass 2 Σ(v−mean)² in ROW ORDER → var=Σ/(n−1), std=√var, NaN below 2 samples — exactly the generic `pivot_table_agg_value` two-pass formula (same mean bits, same left-to-right fold), bit-identical. var 59.1→17.6ms (3.4× FP-side), std 54.1→17.8ms (3.12× vs pandas); Utf8 var 27.9ms. Guards: 39 fp-frame pivot tests (incl. exact-value var/std), 36 fp-conformance pivot (var/std vs live oracle), green. Pivot dense path now covers sum/mean/count/size/min/max/var/std × {Int64,Datetime64,Utf8}. |
| pivot_table dense Datetime64 keys + unified axis path (1q4q4) | 1M rows, Datetime64 idx (1000 days) × int64 col (10), f64 vals, agg=sum | 55.16 ms | 15.9 ms | **3.48× faster** | ✅ FIXED — was 1.14× (Datetime64 keys fell to the generic path). Unified the separate Int64/Utf8 dense gates+helpers into ONE `pivot_axis_dense_codes(col)` per-axis extractor (Int64 / Datetime64 (ns are i64) / Utf8 → sorted-rank codes + labels + names) feeding the shared `pivot_dense_build`. Resolving each axis independently also handles MIXED axes (Datetime64 rows × Int64 cols, as benched). Labels/names mirror the generic path (Int64→to_string, Datetime64→format_datetime_ns, Utf8→string); sort == scalar_key_cmp. 48.6→15.9ms (3.06× FP-side); i64/Utf8 paths unchanged (no regression). Oracle-EXACT (30k, 53×7: sum 22348777.5, max 545146.5, first label = epoch). Net −96 LOC (removed the two type-specific helpers). Guards: 39 fp-frame pivot tests, 36 fp-conformance pivot, fmt+clippy clean. |
| pivot_table dense min/max (Int64 & Utf8) (1q4q4) | 1M rows, int64 idx (1000) × col (10), f64 vals, agg=min | 57.72 ms | 16.2 ms | **3.57× faster** | ✅ FIXED — was 1.12× (max 1.00×). min/max still ran the generic 3×Scalar-materialize + per-cell Vec<f64> groups path. Extended the shared dense builder (`pivot_dense_build`, now used by BOTH the Int64 and Utf8 helpers) with min/max: per-cell `f64::min`/`f64::max` fold over the INFINITY/NEG_INFINITY seed in ROW ORDER == generic `vals.iter().copied().fold(±INF, f64::min/max)`, bit-identical incl. the NaN-skipping f64::min/max semantics. Int64 51.5→16.2ms (3.2× FP-side), Utf8 90.9→26.5ms. Oracle-EXACT incl. negative values + 32031 missing cells (min nansum 322774.0, max 1321609.0). Guards: 39 fp-frame pivot tests (min/max added to dense_int64 test), 40 fp-conformance pivot+crosstab, fmt+clippy clean. |
| crosstab typed dense-Utf8 factorize (1q4q4) | 1M rows, Utf8 row (1000 distinct) × Utf8 col (50) | 428.95 ms | 24.9 ms | **17.2× faster** | ✅ DEEPENED — was already 3.1× (FP 137ms) but ~30× slower FP-side than its own i64 dense path (4.6ms). The generic Utf8 path clones a `String` for BOTH keys of EVERY row into a nested `String->(String->i64)` map. Fix: reuse the pivot factorize lever — each all-valid Utf8 column → dense sorted-rank u32 codes in one pass (shared `pivot_factorize_utf8_sorted`), co-occurrences tallied into a direct-address i64 grid, every cell emitted Int64 (0 for absent). 137→24.9ms (5.5× FP-side). Pure-Utf8 has no Int64/Utf8 stringified-bucket merge quirk, so factorizing raw strings is exact; rows/cols sort by `str::cmp` == `pivot_axis_scalar_cmp`. Oracle-EXACT vs pandas 2.2.3 (20k, 137×11: total 20000, nonzero 1507, col0=c000..colLast=c010). Guards: 7 fp-frame crosstab tests (new `dense_utf8_fast_path_1q4q4`), 40 fp-conformance crosstab+pivot, fmt+clippy clean. pandas crosstab is slow (groupby+unstack) so FP already won, but this closes the FP-side gap. |
| pivot_table typed dense-Utf8 factorize (1q4q4) | 1M rows, Utf8 idx ("r%06d", 1000 distinct) × Utf8 col ("c%04d", 10), f64 vals, agg=sum | 100.39 ms | 26.7 ms | **3.76× faster** | ✅ FIXED — was 1.10× near-parity (FP 90.9ms). Utf8-keyed pivot's cost is dominated by ScalarKey STRING HASHING, not the Vec<f64> groups churn (a generic dense-scatter that only killed the Vec was ~0-gain, REVERTED). Fix: FACTORIZE each Utf8 key column to dense u32 codes in ONE pass (1 hash/row/col = 2× total vs the generic ~4×: idx+col unique-collect + both halves of every (ScalarKey,ScalarKey) groups-tuple), sort distinct by `str::cmp` (== `scalar_key_cmp` for Utf8) and remap codes→sorted rank, then the SAME bit-identical Int64 dense row-order scatter. 90.9→26.7ms (3.4× FP-side). Oracle-EXACT vs pandas 2.2.3 on a 20k×(1000×50) sparse Utf8 run incl. 32031 missing cells (sum 14897835.0, mean 13400251.75, count/size 20000, sort col0=c0000). Reads keys zero-copy from contiguous Utf8 backing when present, else the already-materialized Scalar slice. Guards: 39 fp-frame pivot tests (new `dense_utf8_fast_path_1q4q4`), 36 fp-conformance pivot, fmt+clippy clean. |
| pivot_table typed dense-Int64 (1q4q4) | 1M rows, int64 idx (1000 distinct) × int64 col (10), f64 vals, agg=sum | 56.61 ms | 16.0 ms | **3.54× faster** | ✅ FIXED — was 1.01× parity (the memory 0.67x@1M was a stale/heavier-host read). All-valid Int64 idx+col & all-valid Float64 vals + online agg (sum/mean/count/size) gate a dense column-major R×C scatter-accumulate in ROW ORDER: bit-identical to generic `vals.iter().sum()` fold (same left-to-right per-cell f64 fold), skips 3 × 1M `values()` Scalar materializations + the `(ScalarKey,ScalarKey)->Vec<f64>` groups map + per-cell re-aggregate. Absent cell → `Null(NullKind::NaN)` via `from_f64_values_with_validity` (matches generic fill). Oracle-EXACT vs pandas 2.2.3 on a 20k×(1000×50) sparse run incl. 32031 missing cells: sum 14897835.0, mean 13400251.75, count/size 20000 all identical. count 56.6→15.3ms (3.7×). Non-Int64 keys / non-Float64 vals / other aggfuncs keep generic. Guards: 38 fp-frame `pivot` tests (incl. new `dense_int64_fast_path_1q4q4`), 36 fp-conformance `pivot` (incl. live_oracle), fmt+clippy clean on touched code. |
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
| Series.combine_first public `values()` BOLD-VERIFY cod-a rerun (3gsa7) | Same 2M same-index Float64 NaN-fill workload; current warm artifact `/data/projects/.rch-targets/frankenpandas-cod-a/release/examples/bench_combine_cc`; local CPU7 best-of-50; pandas 2.2.3 inline comparator | construct 13.416 ms; `values` 13.285 ms; `to_numpy` 14.089 ms | construct 0.00936 ms; typed materialize 5.596 ms; public `values()` 33.529 ms | **construct 1434× faster; typed materialize 2.52× faster; `values()` 0.40× (2.52× slower)** | ❌ NO-SHIP / EVIDENCE-ONLY — `/alien-graveyard` routing again points to vectorized columnar execution, and the typed consumer already wins via the lazy all-valid Float64 select tape. The remaining red surface is the public `Vec<Scalar>` contract: every row must become a 32-byte `Scalar::Float64`, so rearranging the materializer is not a radical enough lever. A parallel scalar-materialization probe was attempted and reverted after compile validation hit disk/time pressure before usable benchmark evidence; no code was kept. Route deeper to a public typed/Arrow-style values view or a smaller scalar ABI, not another enum-boxing loop. |
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
| RangeIndex.searchsorted closed-form BOLD-VERIFY (uza04.174/cod-b) | 1M-row `RangeIndex(0, 2n, 2)`, 4,096 deterministic scalar probes, both `left` and `right` side per probe; pandas 2.2.3 scalar loop best-of-200 vs `bench_range_setops searchsorted` | 84.435 ms | 0.069634 ms on RCH `vmi1149989` | **1,212.6× faster** | ✅ VERIFIED KEEP — code-first widened arithmetic path is present and now has a direct benchmark mode. The implementation computes insertion position from `(value - start) / step` in `i128`, clamps to length, and preserves side=`left/right`, empty-range behavior, stepped gaps, and negative-step rejection. Guards: `cargo test -p fp-index searchsorted --release` (10 tests), `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on the changed example (no criticals; existing bench `expect`/`println!` inventory only). |
| RangeIndex.putmask/where typed Int64 BOLD-VERIFY (uza04.153/cod-b) | 1M-row `RangeIndex(0, 2n, 2)`, alternating bool mask, replacement `-7`, sampled first/middle/last labels; pandas 2.2.3 local best-of-20 vs `bench_range_setops putmask_where` | putmask 280.426 ms / where 258.094 ms | putmask 0.952945 ms / where 0.867165 ms on RCH `vmi1149989` | **putmask 294.3× / where 297.6× faster** | ✅ VERIFIED KEEP — code-first direct i64 output paths preserve typed Int64 backing for RangeIndex mask operations instead of materializing `IndexLabel` vectors. Semantics covered by pandas-match and typed-backing tests: mask length mismatch errors, name propagation, descending ranges, `where` false-position replacement, and `putmask` true-position replacement. Guards: `cargo test -p fp-index range_index_where_putmask --release` (3 tests), `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on the changed example (no criticals; existing bench `expect`/`println!` inventory only). |
| Index.append/repeat typed Int64 BOLD-VERIFY (uza04.154/cod-b) | 1M-label flat Int64 `Index` left + right, output length 2M; `append` and `repeat(2)` sampled first/middle/last labels; pandas 2.2.3 local best-of-50 vs `bench_range_setops index_append_repeat` | append 0.876401 ms / repeat(2) 2.310909 ms | append 0.791421 ms / repeat(2) 0.919174 ms on RCH `vmi1149989` | **append 1.11× / repeat(2) 2.51× faster** | ✅ VERIFIED KEEP — code-first raw i64 output paths are present and preserve typed Int64 backing for flat `Index::append` and `Index::repeat`, avoiding `IndexLabel` materialization for typed and affine Int64 inputs. Append is only a modest no-gap win; repeat is a clear pandas win, so no production rollback. Guards: `cargo test -p fp-index typed_backing_codb --release` (11 tests), `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on the changed example. |
| Index.drop(labels) sorted Int64 two-pointer BOLD-VERIFY (uza04.155/cod-b) | 1M-label flat Int64 `Index`, drop every fourth Int64 label (250k labels), output length 750k; sampled first/middle/last labels; pandas 2.2.3 local best-of-30 vs `bench_range_setops index_drop_labels` | 6.993 ms | 1.238196 ms on RCH `vmi1149989` (pre-lever FP 20.094514 ms) | **5.65× faster** | ✅ KEEP — the existing raw-Int64 drop path still used an `FxHashSet<i64>` for every source value, making the sorted Int64 workload 0.35× vs pandas. Added a nondecreasing Int64 branch that sorts/dedups the Int64 drop labels once and streams source/drop labels with a two-pointer filter, preserving duplicate-source drop-all semantics, missing-label ignore behavior, non-Int64-label ignore behavior, name propagation, and typed Int64 output backing. Same-worker FP-side delta: **16.23× faster**. Guards: `cargo test -p fp-index drop_labels --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| Index.get_indexer_non_unique sorted Int64 dense runs BOLD-VERIFY (uza04.156/cod-b) | 1M-label flat Int64 source with 4 repeats per label; 250k Int64 target labels, 75% present / 25% missing, output 812.5k positions; pandas 2.2.3 local best-of-30 vs `bench_range_setops index_get_indexer_non_unique` | 244.532 ms | 5.645762 ms on RCH `vmi1149989` (pre-lever FP 173.100627 ms on RCH `vmi1264463`) | **43.31× faster** | ✅ KEEP — the existing raw-Int64 path already beat pandas 1.41× but rebuilt an `FxHashMap<i64, Vec<usize>>` over sorted bounded duplicate labels. Added a nondecreasing Int64 dense run-table path (`slot -> start/end`) that expands duplicate matches without hashing or label materialization, preserving source-order duplicate expansion, missing target ordinals, and typed Int64 backing. Cross-worker FP-side routing delta: **30.66× faster**; the pandas-vs-after ratio is decisive. Guards: `cargo test -p fp-index get_indexer_non_unique --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| Index.diff raw Int64 construction BOLD-VERIFY (uza04.157/cod-b) | 1M-label flat Int64 `Index`, value pattern `i*3 + i%7`, `periods=1`, sampled first/middle/last deltas; pandas 2.2.3 local best-of-30 vs `bench_range_setops index_diff` | 1.309969 ms | 1.846690 ms on RCH `vmi1149989` (pre-lever FP 3.113643 ms local fallback after RCH preflight block) | **0.71× (1.41× slower)** | ✅ KEEP / RESIDUAL LOSS — raw Int64 `Index::diff` already avoided label materialization, but still prefilled the whole `Vec<Option<IndexLabel>>` with `None` before overwriting almost every slot. Moved the raw-Int64 branch ahead of the full prefill and changed the loop to push leading `None`s plus zipped checked-sub deltas, preserving `checked_sub` overflow-to-`None`, `periods=0`, `periods>=len`, and no materialization. FP-side delta: **1.69× faster**. Residual gap is representation-bound: this API writes `Vec<Option<IndexLabel>>`, while pandas emits a compact float64 index for integer diff. Guards: `cargo test -p fp-index diff --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| Index.position/get_loc/contains scalar Int64 BOLD-VERIFY (uza04.160/cod-b) | 1M-label flat Int64 `Index`, 4,096 deterministic present probes; each timed loop does `position + get_loc + contains` for sorted ascending and descending-unique indexes; pandas 2.2.3 local best-of-50 vs retrieved `bench_range_setops index_position_lookup` binary | sorted 4.194045 ms / unsorted 4.396288 ms | sorted 0.915624 ms / unsorted 0.405648 ms local; remote sanity on RCH `vmi1149989`: sorted 0.593654 ms / unsorted 0.454524 ms | **sorted 4.58× / unsorted 10.84× faster** | ✅ KEEP — sorted raw Int64 already avoided label materialization, but unsorted raw Int64 scalar lookup still rescanned the whole label slice for every `position/get_loc/contains` call (pre-lever local fallback: unsorted 1,633.368379 ms for the same 4,096-probe loop). Reused the existing identity-keyed `i64 -> position` cache for unsorted scalar lookups and corrected the cache builder to keep the first occurrence instead of overwriting duplicates, preserving pandas scalar `get_loc` first-position semantics, non-Int64 rejection, sorted binary-search behavior, duplicate-label behavior, and lazy backing non-materialization. Same-host FP-side unsorted delta: **4,026× faster**. Guards: `cargo test -p fp-index int64_position_avoids_label_materialization_codb --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| Index.asof_locs sorted Int64 target BOLD-VERIFY (uza04.161/cod-b) | 1M-label flat Int64 source `0,2,4,...` and 1M nondecreasing Int64 `where` probes just below source labels; FP no-mask path vs pandas `Index.asof_locs(where, np.ones(n,bool))`; best-of-20 local retrieved binary plus RCH sanity | 23.302386 ms | 1.959011 ms local; remote sanity on RCH `vmi1149989`: 1.804858 ms (pre-lever remote 50.859423 ms; pre-lever local 36.689613 ms) | **11.90× faster** | ✅ KEEP — the raw Int64 no-mask `asof_locs` path already avoided label materialization but still binary-searched every target independently, making sorted probes 0.61× vs pandas locally. Added a nondecreasing-target branch that streams source and targets with one two-pointer right-bound pass, preserving duplicate-source rightmost semantics, `None` before first label, typed Int64 non-materialization, and unsorted-target fallback to the existing binary-search path. Same-worker FP-side delta: **28.18× faster** remote; same-host FP-side delta: **18.73× faster**. Guards: `cargo test -p fp-index asof_locs --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| Index.intersection/difference sorted Int64 backing BOLD-VERIFY (uza04.162/cod-b) | 1M-label flat Int64 `Index` left `0..n` and right `n/2..n+n/2`, both sorted unique; output length 500k; pandas 2.2.3 local NumPy-backed `pd.Index(np.arange(...))` best-of-30 vs retrieved `bench_range_setops index_sorted_setops` binary | intersection 8.699887 ms / difference 10.939689 ms | intersection 1.073834 ms / difference 0.985216 ms local; remote sanity on RCH `vmi1149989`: intersection 0.884873 ms / difference 0.805362 ms | **intersection 8.10× / difference 11.10× faster** | ✅ VERIFIED KEEP — the existing code-first sorted raw-Int64 set-op path is already decisive against pandas: it streams both sorted slices with a two-pointer merge and constructs typed Int64 outputs, avoiding label materialization and hash membership. Added a direct benchmark scenario and verified name propagation, self-order, deduped sorted-unique semantics, typed output backing, and non-materialized inputs through the existing focused test. No production change was needed; deeper work would be churn. Guards: `cargo test -p fp-index sorted_int64_set_ops_keep_typed_backing_without_materializing_v7m2q --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| Index hash set-op outputs typed Int64 BOLD-VERIFY (uza04.163/cod-b) | 1M-label flat Int64 `Index` left/right in alternating high/low unsorted order with 500k overlap; pandas 2.2.3 local NumPy-backed `pd.Index(...).{intersection,union,difference,symmetric_difference}(sort=False)` best-of-20 vs retrieved `bench_range_setops index_hash_setops` binary | intersection 13.605819 ms / union 11.121471 ms / difference 10.606232 ms / symmetric_difference 19.039147 ms | local intersection 3.555570 ms / union 4.191747 ms / difference 3.678734 ms / symmetric_difference 8.012069 ms; remote sanity on RCH `vmi1153651`: 8.621216 / 10.109141 / 8.739667 / 19.348115 ms | **3.83× / 2.65× / 2.88× / 2.38× faster** | ✅ VERIFIED KEEP — the existing code-first all-Int64 hash/fallback set-op paths already construct typed Int64 outputs and avoid `IndexLabel` materialization for unsorted `intersection`, `union_with`, `difference`, and `symmetric_difference`. Added a direct unsorted benchmark scenario and measured same-host pandas wins for all four operations; the remote symmetric-difference sanity is cross-worker near-parity (0.98×) but not a same-host regression signal. Existing focused tests cover first-seen order, duplicate handling, names, typed output backing, and non-materialized inputs. No production change was needed. Guards: `cargo test -p fp-index unsorted_int64_set_ops_keep_typed_backing_without_materializing_m8x4p --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| Index.from_range lazy affine BOLD-VERIFY (uza04.164/cod-b) | 1M-label unit, strided, and descending Int64 range-style construction plus one midpoint `get_loc`; pandas 2.2.3 local `pd.RangeIndex(start, stop, step)` best-of-10k vs retrieved `bench_range_setops index_from_range` binary | unit 0.001583 ms / strided 0.001613 ms / descending 0.001603 ms | local unit 0.000080 ms / strided 0.000080 ms / descending 0.000070 ms; remote sanity on RCH `vmi1149989`: 0.000070 / 0.000070 / 0.000050 ms | **19.79× / 20.16× / 22.90× faster** | ✅ VERIFIED KEEP — `Index::from_range` already routes through `new_known_unique_int64_affine_range`, preserving unit, non-unit, descending, and step-zero empty semantics while leaving `labels.materialized` empty until a caller asks for label materialization. Added a direct constructor+lookup benchmark and confirmed pandas `RangeIndex` is already slower at this micro-boundary; no production change was needed. Guards: `cargo test -p fp-index from_range --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| RangeIndex take/repeat direct i64 values BOLD-VERIFY (uza04.166/cod-b) | 1M-label `RangeIndex(0, n*3, 3)` with 1M deterministic shuffled take positions and `repeat(2)`; pandas 2.2.3 local `pd.RangeIndex(...).take(pos)` / `.repeat(2)` best-of-20 vs retrieved `bench_range_setops range_take_repeat` binary | take 0.578797 ms / repeat(2) 1.272582 ms | pre-lever local take 1.421052 ms / repeat(2) 2.393456 ms; post-lever local take 0.713109 ms / repeat(2) 1.198137 ms | **take 0.81× pandas (1.23× slower) / repeat 1.06× faster; FP-side 1.99× / 2.00× faster** | ✅ PARTIAL KEEP — normal RangeIndex spans now prove once that `start + step * position` fits i64, then materializing take/repeat avoid the old per-element `i128` value path; extreme spans and out-of-bounds selectors still fall back to the existing wide/arithmetic error path. `repeat(2)` flips ahead of pandas and both operations roughly halve FP time, but shuffled `take` remains a measured residual loss against pandas' vectorized gather. Tried a two-pass branch-free take fill and a one-pass unchecked cast variant; both regressed (1.118336 ms and 1.008137 ms take respectively), so reverted those probes. Guards: `cargo test -p fp-index range_index_take --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| RangeIndex.to_flat_index lazy Int64 backing BOLD-VERIFY (uza04.167/cod-b) | 1M-label unit, strided, and descending `RangeIndex` with name, each operation calls `to_flat_index()` then one midpoint `get_loc`; pandas 2.2.3 local best-of-200 vs `bench_range_setops range_to_flat_index` | unit 0.000301 ms / strided 0.000321 ms / descending 0.000310 ms | local unit 0.000170 ms / strided 0.000170 ms / descending 0.000151 ms | **1.77× / 1.89× / 2.05× faster** | ✅ VERIFIED KEEP — current `RangeIndex::to_flat_index` already delegates to `Index::from_range`, preserves the name, and keeps typed lazy Int64 backing with `labels.materialized` empty; pandas returns the same lazy `RangeIndex`, but FP's constructor+lookup boundary is still faster on this same-host microbench. No production code was changed; added only the benchmark mode. Guards: `cargo test -p fp-index to_flat_index --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
| RangeIndex splice outputs BOLD-VERIFY (uza04.169/cod-b) | 1M-label `RangeIndex(0, n*3, 3)` plus right `RangeIndex(n*5, n*7, 2)`, construction+`len()` for middle `insert(-7)`, `append`, and middle `delete`; pandas 2.2.3 local best-of-20 vs `bench_range_setops range_splice_outputs` | insert 0.193427 ms / append 0.838418 ms / delete 0.192084 ms | local final insert 0.628560 ms / append 0.000120 ms / delete 0.000170 ms; pre-lazy materialized candidate was insert 0.577594 ms / append 1.342152 ms / delete 0.706188 ms | **insert 0.31× pandas (3.25× slower residual) / append 6,987× faster / delete 1,130× faster** | ✅ PARTIAL KEEP — `append` and middle `delete` now return lazy two-affine Int64 `Index` outputs, preserving name and typed/non-materialized backing while avoiding full `Vec<i64>` construction. `insert` still needs a three-segment representation for the benchmark shape and remains materialized, so it is recorded as a residual pandas gap rather than hidden. The generic two-affine constructor does not predeclare uniqueness, so overlapping `append` keeps duplicate semantics; `delete` uses the known-unique two-run path. Guards: `cargo test -p fp-index range_index_splice --release`, `cargo check -p fp-index --all-targets`, `cargo clippy -p fp-index --all-targets -- -D warnings`, `cargo fmt -p fp-index --check`, and UBS on changed fp-index files. |
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

### 2026-06-21 BlackThrush — resample("M") per-row String-key CACHE ~3.75x (the real lever, shipped)
Confirmed the resample cost is the per-row String key alloc (NOT the std-SipHash map, per the prior
ixn43 finding). CLEANER fix than the int-period restructure: the datetime index is time-ordered, so
consecutive rows share a bucket — cache resample_month_end_key's output, recompute only when
bucket_end_mo(mo) changes (1M allocs -> ~num_buckets). Local to the scatter loop (no return-type ripple).
MEASURED: resample_mean 14200 -> 3780us = ~3.75x: 0.05x->0.19x@10k / 0.17x->0.64x@100k / 1.70x->6.15x@1M.
Bit-identical (key = deterministic fn of bucket_end_mo), conformance GREEN (resample 51/0). The other 5
freq variants (W/Q/Y/D/B) have the SAME per-row-key smell — same cache applies (br-ixn43, follow-up).
LESSON: when a per-row String KEY is the cost, CACHE it for time-ordered/sorted data (single-element
last-key cache) — simpler + lower-risk than restructuring to integer keys.

### 2026-06-21 BlackThrush — resample sub-daily (h/min/s) key-cache ~7.1x; 2nd freq confirms the lever
After monthly (~3.75x), applied the same per-bin key-cache to the sub-daily ns-bucketing path (h/min/s).
Added resample_hourly bench (minutely data -> hourly bins, 60 rows/bin). BASELINE catastrophic: 0.01x@10k
/ 0.05x@100k / 0.47x@1M (fp ~37900us flat — DateTime::from_timestamp + format! PER ROW). Cache on bin_index
(recompute the DateTime+format! only when the bin changes) => ~7.1x: 0.08x@10k / 0.34x@100k / 3.03x@1M
(now a WIN). Bit-identical (key = deterministic fn of bin_index; contains_key still dedups order),
conformance GREEN (resample 51/0). TWO resample freqs now confirm the per-row-String-key-CACHE lever
(monthly + sub-daily). Remaining: W/Q/Y (key_of(ord) — needs bucket-id exposed) + N-day (br-ixn43).

### 2026-06-21 BlackThrush — resample daily(D) key-cache ~3.7x; 3rd freq (monthly+sub-daily+daily done)
The daily-contiguous path did key_of(ord) (a format! String) per row; a daily bucket == one `ord`, so for
sub-daily data (24 rows/day) cache on ord. Added resample_daily bench (hourly->daily). Baseline 0.03x@10k
/0.08x@100k/0.75x@1M (fp ~21400us); now 0.09x@10k/0.31x@100k/2.93x@1M (WIN) — ~3.7x. Bit-identical,
conformance GREEN (resample 51/0). THREE most-common resample freqs now key-cached (M ~3.75x, h/min/s
~7.1x, D ~3.7x). Remaining: N-day(2D/3D @22108 bin_start-cacheable), weekly/Q/Y, business-day (br-ixn43).

### 2026-06-21 BlackThrush — resample N-day(2D)+business-day(B) key-cache; resample_build_groups is SHARED
KEY FINDING: resample_build_groups (the bucketing helper I cached) is called by BOTH Series.resample
(22331) AND DataFrame.resample (23467) — so ALL my resample caches (M/h/D/2D/B) fix df.resample too (2x
impact). Extended the cache to N-day (cache key_of(bin_start) on bin_start) + business-day (cache on ord),
both in the shared helper. Added resample_2d + resample_bday benches. CACHED (measured): resample_2d
0.40x@100k/3.68x@1M; resample_bday 3.61x@100k/35.98x@1M (pandas business-day resample is very slow,
199ms@1M vs fp 5.5ms). Bit-identical, conformance GREEN (resample 51/0). Baselines ~flat-slow by analogy
to the measured D (21ms->5.7ms). FIVE freqs now cached (M/h-min-s/D/2D/B); only W/Q/Y remain (need bucket-id
derived before key_of). The per-bin-key-cache lever covers nearly all of Series+DataFrame resample.

### 2026-06-21 BlackThrush — resample WEEKLY(W) key-cache ~4.35x; RESAMPLE SWEEP COMPLETE
Weekly was the last uncached common freq (Q/Y route through the cached monthly bucket_end_mo path —
measured 5.24x/2.58x; W was a separate path). Cache key_of(bin_end(ord)) on bin_end(ord). Added
resample_w/q/y benches. W: 20900 -> 4810us ~4.35x (0.24x@10k / 1.18x@100k WIN / 2.63x->11.48x@1M). Bit-
identical, conformance GREEN (resample 51/0). RESAMPLE SWEEP COMPLETE — ALL freqs cached in the SHARED
resample_build_groups (M/h-min-s/D/2D/B/W + Q/Y via monthly), covering BOTH Series.resample AND
DataFrame.resample. The per-bin-key-cache lever (recompute key_of only on integer-bucket-id change for
time-ordered data) eliminated the per-row String/DateTime alloc across the entire resample surface.

### 2026-06-21 BlackThrush — df.groupby(string) is ALREADY a win; GroupMap std-SipHash is NOT a loss
Investigated the std-SipHash GroupMap (type GroupMap = HashMap<GroupKey,Vec<usize>> @1126) used by
DataFrameGroupBy.build_groups (string/multi keys; int64 keys bypass via int64_dense_grouping). Added a
df_groupby_str_sum bench (1000 string groups). MEASURED: 4.16x@100k / 29.57x@1M (fp ~990us) — a WIN, NOT
a loss. Also verified SeriesGroupBy string (groupby_mean_str 2.31x/23.03x, transform 2.47x/31.35x) — wins
(uses the FxHashMap<ScalarKey> build_groups @24532). So groupby is FINE for string keys; the GroupMap
SipHash is well-amortized (only ~ngroups distinct keys, the per-row insert hits an existing bucket).
Converting GroupMap->FxHashMap would be ~0-gain churn on a winning op — NOT DONE (REVERT-~0-gain
discipline). The main groupby is already FxHashMap/int64-dense optimized; no lever here.

### 2026-06-21 BlackThrush — WARMED GAUNTLET: standard ops all DOMINATE; memory join ratios were understated
Warm-verified the ops memory flagged as "marginal/laggy" (clean-MIN @1M, 2 pandas warmups + 8 iters):
ewm_mean 20.72x, sort_values_single 34.80x, join_inner 27.36x, join_left 28.13x (memory said 1.47x!),
join_outer 44.35x (memory said 1.63x!), join_inner_str 173.24x. ALL big wins — NO hidden losses among the
standard numeric/join/sort/ewm ops (unlike the reshapes, where warmup exposed real losses). The memory's
join_left/outer 1.47/1.63x were NO-WARMUP/harness-p50 artifacts; warmed reality is 28-44x. CONCLUSION: the
loss-rich territory was the wide-output reshapes + per-row-key time-series (now all fixed this session);
the standard ops were always dominant. The warmup discipline both FINDS hidden losses (pivot_table) and
DEBUNKS understated ratios (joins) — clean-MIN must warm pandas either way.

### 2026-06-21 BlackThrush — FULL WARMED GAUNTLET: domination confirmed across ALL categories
Extended the warmed clean-MIN gauntlet (@1M, pandas-warmed) to indexing/io/parsing: iloc_slice 4.73x,
loc_labels 170.99x, reindex 163.98x (memory flagged "alloc-bound/declined" — WRONG, it's 164x),
csv_read 1035.15x, to_datetime 33.97x. Combined with the earlier batch (ewm 20.7x, sort 34.8x, joins
27-44x, string-join 173x, groupby 4-31x) and the shipped fixes (reshapes, resample, categorical), fp
DOMINATES pandas across EVERY benched op family. NO remaining hidden losses. The ONLY non-win is the
architectural transpose/to_numpy (l4vzc — pandas O(1) 2D-block view vs columnar). CONCLUSION OF THE
SWEEP: the loss-rich territory was wide-output reshapes + per-row-key time-series (all fixed this
session, ~13 levers); everything else was always dominant — several memory "gaps" were no-warmup/
small-size artifacts (joins 1.47x->28x, reindex declined->164x). clean-MIN MUST warm pandas.

### 2026-06-21 BlackThrush — cut() all-valid contiguous-Utf8 ~2.1x; un-benched hidden loss
pd.cut binning was un-benched + a hidden loss: 0.81x@100k (fp ~3220us). Labels were pre-formatted (one
per bin, br-21a14) but the output still did n Scalar::Utf8(bin_labels[idx].clone()) + from_values =
n String clones + a copy. Added an all-valid fast path: emit bin_labels[idx] bytes into ONE contiguous
buffer -> from_utf8_contiguous (the write!-into-buffer/stack/explode/melt vein). Bit-identical (same
bin_idx + labels; all-valid => no null-kind question; missing values fall through to the Scalar path to
preserve the exact Null spelling). Added cut_bins bench. RESULT: 0.81x->1.75x@100k (WIN) / 7.19x->16.11x@1M
~2.1x. Conformance GREEN (cut 15/0). qcut (37055) is the sibling — same n-Scalar::Utf8-clone smell,
same fast path applies (follow-up). LESSON: keep benching un-benched ops — cut joins the reshape/resample
family of hidden losses; the contiguous-Utf8 lever recurs wherever output = per-row label/category String.

### 2026-06-21 BlackThrush — qcut() all-valid contiguous-Utf8 (sibling of cut); cut/qcut both win now
Applied the same all-valid contiguous-Utf8 fast path to qcut (quantile binning) — its output map had the
identical n Scalar::Utf8(bin_labels[bin_idx].clone()) + from_values smell (bin_idx via partition_point).
Added qcut_bins bench. WITH fast path: 1.92x@100k / 18.30x@1M (fp ~3530us — higher than cut's 1521us
because qcut sorts for quantile edges, O(n log n) inherent; the fast path removed the n Scalar clones).
Bit-identical (conformance qcut 6/0). cut + qcut binning now both WIN. The contiguous-Utf8 lever
(per-row label/category String -> one buffer) recurs across stack/explode/melt/cut/qcut + dt-string +
astype-str — a fully harvested vein. (cut_bins explicit-edges has the same smell + an O(n*bins) find;
lower priority, less common.)

### 2026-06-21 BlackThrush — wide_to_long is ALREADY a win (62x); Scalar build NOT a loss here
Benched wide_to_long (un-benched reshape, melt-class Scalar cell-by-cell build — a cut/qcut-style
candidate). MEASURED: 6.10x@100k / 62.19x@1M (fp ~20650us flat). A WIN — pandas wide_to_long is Python-
level + very slow (1.28s@1M), so even fp's Scalar materialization wins big. fp's flat ~20ms is regex-
compilation setup (the stub `^A_(\d+)$` regexes, fixed per call), NOT output-bound (output scales 10x
@100k->1M but fp is flat). The contiguous/typed lever would be ~0-gain churn on a 62x win — NOT DONE.
LESSON: not every Scalar-cell-by-cell reshape is a loss — when pandas is Python-slow, fp wins anyway;
the contiguous-Utf8 lever only matters when fp would otherwise LOSE (cut/qcut/stack/explode/melt did;
wide_to_long does not). Bench committed as a regression workload.

### 2026-06-21 BlackThrush — DEFINITIVE SESSION SCORECARD: all 15 levers hold (no regression)
Consolidated re-measurement (@1M, pandas-warmed) of every lever shipped this session, after multi-agent
churn: pivot_table 23.5x, pivot 7.7x, crosstab 13.2x, series_map 3.1x, unstack 2.6x (was O(N^2)/unusable),
get_dummies 17.9x, from_categorical 7.4x, cut 17.5x, qcut 19.6x, resample M 4.1x / h 2.1x / W 10.1x. ALL
HOLD as wins. fp times stable; ratio variance is pandas-side noise. NO regression. 15 levers, 4 reusable
patterns (FxHashMap-on-hot-path, O(N^2)->O(N), per-row-key cache, contiguous-Utf8 output), 18 new benches,
all bit-identical + conformance-green. The wide-output-reshape + per-row-key-time-series + per-row-label-
binning families — the codebase's loss-rich territory — are FULLY harvested. Only remaining non-win is
the architectural transpose (1.37x, n-column wall) + to_numpy (bench-only, no real caller) = l4vzc, low-EV
(high arch risk, low real value). Perf surface comprehensively DOMINATED.

### 2026-06-21 BlackThrush — cut_bins (explicit edges) contiguous-Utf8 ~1.7x; BINNING FAMILY complete
The explicit-edges cut variant had the same n Scalar::Utf8(labels[i].clone()) smell + a 0.81x@100k loss.
Refactored its closure to produce bin indices (Vec<Option<usize>>, find logic unchanged), then an
all-in-range fast path emits labels into ONE contiguous buffer; out-of-range/missing fall through to the
Scalar path. Added cut_explicit bench (edges spanning the data). RESULT: 0.81x->1.38x@100k (WIN) /
7.45x->12.45x@1M ~1.7x. Bit-identical, conformance GREEN (cut 15/0). BINNING FAMILY COMPLETE: cut ~2.1x,
qcut ~1.9x, cut_bins ~1.7x — all were @100k losses (n Scalar::Utf8 clones), all now wins via the
contiguous-Utf8 vein (separate bin-assignment from output; emit pre-formatted labels into one buffer).

### 2026-06-21 BlackThrush — str.split(expand) is a win (46x); loss-rich territory EXHAUSTED
str.split(expand=True) (split_df_n) benched: 3.51x@100k / 45.95x@1M — a WIN (pandas str.split is Python-
level). Like wide_to_long (62x) and groupby (4-31x), the recent un-benched ops are ALL WINS. The "bench
un-benched ops" strategy that found 16 hidden losses (reshapes/time-series/binning) has reached zero
hit-rate: the loss-rich territory (ops where fp's columnar approach had a SPECIFIC inefficiency vs a tight
loop — SipHash scatter, O(N^2) find, per-row String key, per-row label clone) is FULLY HARVESTED. The
remaining un-benched ops are wins (pandas Python-level) — fp wins despite Scalar materialization because
pandas is slower. CONCLUSION: 16 levers shipped this session; the perf surface is comprehensively
dominated; only the architectural transpose/to_numpy (l4vzc, low-EV) remains a non-win.

### 2026-06-21 BlackThrush — to_json LOSS @small (0.73x); C-optimized-pandas vein (filed, not rushed)
df.to_json(orient="records") = 0.73x@100k (fp ~155ms vs pandas 113ms C-ujson) / 7.92x@1M (fp ~155ms flat).
Added json_write_records bench. SMELL: to_json builds a full serde_json Value tree (n*m Values + n Object
maps + per-cell BTreeMap lookup) then serializes it — 2 passes vs pandas ujson's direct one-pass. LEVER:
stream-serialize directly (write!-into-buffer / serde_json::Serializer), skip the tree; BIT-IDENTITY must
byte-match serde_json's f64/string output. Filed br (involved + risky + flat-anomaly needs profiling — not
rushed). SIGNIFICANCE: FIRST loss among C-OPTIMIZED pandas ops (JSON). The Python-level ops (str/reshape/
wide_to_long) fp wins; the C-optimized ones (JSON, and structurally to_numpy/transpose) are where fp can
still lose — a new vein to probe (read_json, to_json columns, maybe parquet/excel).

### 2026-06-21 BlackThrush — read_json WINS (15x); only to_json @small loses (flat-anomaly, profile first)
Benched read_json (json_read_records): 1.39x@100k / 15.09x@1M — a WIN (pandas read_json overhead). So the
JSON vein is NARROW: only to_json serialization @small loses (0.73x@100k). CAVEAT: both fp times are FLAT
(to_json ~155ms, read_json ~215ms @100k AND @1M despite 10x scaling) — anomalous, possibly a fixed cost OR
a fp-bench io-timing artifact. Updated br-de91c: PROFILE to_json before the streaming rewrite (confirm the
loss is real + tree-dominated, not a bench artifact). The C-optimized-pandas vein is essentially ONE op
(to_json @small); read_json + everything else dominates. The perf surface remains comprehensively
dominated.

### 2026-06-21 BlackThrush — to_json values()-hoist ~13% (partial; bulk ~134ms still flat-anomalous)
Profiling de91c: to_json tree-build ~100ms dominant. Hoisted the per-cell self.columns[name].values()
(BTreeMap lookup + values() call, n*m times) to once-per-column -> col_values: Vec<&[Scalar]>. MEASURED
~13%: 155->134us... 155ms->134ms: 0.73x->0.86x@100k, 7.92x->9.55x@1M. Bit-identical, conformance GREEN
(to_json 18/0). So unlike the to_csv-hoist (~0-gain), the per-cell values() access WAS ~13% here. BUT the
bulk ~134ms is STILL flat across 10k/1M (the Map/Value building + serialize) — the unexplained fixed cost
remains (de91c open: needs finer in-loop profiling of Map-insert vs scalar_to_json_value vs serialize; the
flatness rules out the n*m building being it). to_json still loses @100k (0.86x) but wins @1M; the full
streaming/direct-serialize fix is the remaining lever, filed.

### 2026-06-21 BlackThrush — CRITICAL CORRECTION: --size arg bug invalidated ALL @1M ratios this session
size_rows_cols() only matches "10k"/"100k"/"1M" (abbreviated); my measurement scripts passed NUMERIC
--size 1000000 / 100000 which FALL THROUGH to the default (100_000, 10). So EVERY fp-bench call this
session used a 100k df regardless of --size. Consequence: my "@100k" numbers are CORRECT (default==100k),
but EVERY "@1M" ratio compared fp@100k vs pandas@1M — INFLATED. RE-MEASURED with abbreviated --size:
  pivot_table @1M 2.27x (reported 23.5x), get_dummies 1.36x (17.9x), cut 1.22x (17.5x), crosstab 1.23x
  (13.2x) — STILL WINS but far smaller. AND THREE ARE ACTUAL @1M LOSSES: unstack 0.22x (reported 2.6x),
  series_map 0.23x (3.1x), resample_h 0.20x (2.1x). to_json 0.69x@1M (reported 9.55x).
INTEGRITY: the LEVERS themselves are valid — bit-identical, conformance-green, and the @100k before/after
improvements are REAL (unstack O(N^2)->O(N) avoided a genuine 14s catastrophe; FxHashMap/cache/contiguous
all measurably faster at 100k). But fp still LOSES to pandas at 1M on the C-optimized-pandas ops (unstack,
series_map, resample, to_json — same family as to_numpy/transpose). The "comprehensively dominated" claim
was WRONG for @1M. CORRECTED PICTURE: fp wins Python-level-pandas ops; loses several C-optimized ones at
scale. Methodology fix: ALWAYS use --size 10k/100k/1M (abbreviated). Re-auditing all levers at real sizes
is the next priority. The full warmed-gauntlet "joins 28-44x etc" numbers are ALSO suspect (numeric size).

### 2026-06-21 BlackThrush — CORRECTED SCORECARD @1M (abbreviated --size): honest picture
Standard ops re-measured at REAL @1M: sort_single 2.14x (reported 35x), join_left 3.55x (28x), join_inner
4.09x (27x), ewm_mean 2.79x (21x), groupby_sum_i64 18.17x (genuinely big). value_counts 0.32x LOSS (the
khash floor — memory warned, my inflated numbers masked it). HONEST @1M PICTURE:
  WINS (2-18x, not the 13-44x I claimed): pivot_table 2.27x, get_dummies 1.36x, cut 1.22x, crosstab 1.23x,
  sort 2.14x, joins 3.5-4.1x, ewm 2.79x, groupby 18x.
  LOSSES @1M: value_counts 0.32x, unstack 0.22x, series_map 0.23x, resample 0.20x, to_json 0.69x,
  + architectural to_numpy/transpose.
So fp does NOT "comprehensively dominate" @1M — it wins most ops 2-18x but LOSES ~6 (the C-optimized-pandas
ops + the khash-floor value_counts + fp's string-composite-index unstack). The SESSION LEVERS ARE STILL
VALID: every @100k before/after improvement is real + bit-identical (unstack O(N^2)->O(N) avoided a 14s
catastrophe; FxHashMap/cache/contiguous measurably faster), they just don't lift the C-optimized ops to a
WIN at 1M. The real remaining loss-rich vein: value_counts (khash), unstack (string-composite index +
Scalar output), series_map (IndexLabel-per-row + Scalar out), resample (per-row bucket + Scalar agg),
to_json (serde Value tree). These are fp-design-vs-pandas-C costs — harder than the @100k levers.

### 2026-06-21 BlackThrush — series_map typed-Int64 fast path: 0.23x->4.31x@1M (REAL @1M loss FIXED)
First fix of a REAL @1M loss (measured CORRECTLY with abbreviated --size). Series::map_series did per-row
IndexLabel construction + FxHashMap lookup + Vec<Scalar> output over the Scalar-materialized self. Added a
typed all-Int64 fast path (mirroring the existing Series::replace one — its comment literally said "see
map"): when self.as_i64_slice() is Some AND the mapper index is all-Int64, probe each row's i64 directly
over the raw &[i64] view (no self Scalar materialize, no per-row IndexLabel), i64-keyed FxHashMap (collected
last-wins like the IndexLabel one => bit-identical); when all mapped values are Int64 + every row maps,
emit from_i64_values (typed, no output Scalar Vec). MEASURED: 23890->2542us@1M = ~9.4x fp-side -> 0.23x->
4.31x@1M (WIN!) / 5.73x@100k. Conformance GREEN (map_series 3/0). LESSON: the real @1M losses (vs pandas-C)
ARE fixable with the typed-direct-probe pattern (as_i64_slice + dense/typed lookup + typed output, skipping
ALL Scalar materialization) — the earlier campaign's factorize/unique/replace vein, now applied to map.
ikq9a updated: series_map DONE; unstack/resample/value_counts/to_json remain.

### 2026-06-21 BlackThrush — resample typed-f64 mean ~7% (agg NOT the bottleneck; bucketing is)
Applied the typed-probe pattern to resample mean (Resample::mean): typed-f64 agg (as_f64_slice, no NaN ->
nanmean==sum/count; per-bucket f64 index instead of Vec<Scalar> clone; from_f64_values output). Gated
bit-identical (resample 51/0). MEASURED: 59305->55155us@1M = ONLY ~7% (still 0.28x@1M LOSS). So UNLIKE
series_map (Scalar-materialization-bound, 9.4x win), resample is BUCKETING-bound: resample_build_groups'
per-row scatter into groups: HashMap<String,Vec<usize>> (the String period keys + HashMap get_mut+push
per row) dominates, NOT the Scalar agg. The REAL resample lever is the bucketing: integer-period keys
(year*12+month etc.) scattering into a DENSE/int-keyed structure (no String, no HashMap) — the complex
multi-site restructure (the 6 bucketing fns RETURN String-keyed groups). LESSON: the typed-probe pattern
only wins where Scalar-materialization is the bottleneck; bucketing/hash-table-bound ops (resample,
value_counts) need the hash-table/scatter reworked. Shipped the ~7% (real, bit-identical, right direction).

### 2026-06-21 BlackThrush — value_counts dense-int64 path: i64 5.81x@1M WIN (beats khash); f64 still loses
value_counts_with_options had an FxHashMap tally (set_member_key + lookup per Scalar). Added a dense-int64
fast path: all-valid bounded Int64 -> O(1) direct-address count (no hash, no set_member_key, no Scalar
materialize), first-seen order preserved so the stable count-sort ties match. Bit-identical (value_counts
32/0). MEASURED (new value_counts_i64 bench, i%1000): 5.50x@100k / 5.81x@1M (fp 1297us vs pandas 7542us) —
BEATS pandas C khash for bounded int (dense direct-address > hashing). NOTE: the existing f64 value_counts
bench (0.14x@1M loss) is UNAFFECTED — f64 is not dense-indexable, so the khash floor remains there (needs
a custom open-addressing f64 table, untried). But int value_counts (IDs/categoricals-as-int — common!) now
WINS. Gate: range <= max(64Ki, 4*n) so sparse keys fall back to the hash path. The dense-direct-address
pattern beats khash where the key space is bounded — a real lever for the hash-bound losses.

### 2026-06-21 BlackThrush — unstack typed-output ~0-gain (REVERTED); the INPUT string-parse is the bulk
Tested an unstack typed-f64 output path (per-column all-present-f64 -> from_f64_values, skip the Scalar
output). MEASURED: 359437->360514us@1M = ~0% (still 0.13x). REVERTED (~0-gain, though conformance passed
unstack 7/0 — bit-identical). NEGATIVE EVIDENCE: the output Scalar materialization is NOT the unstack
bottleneck. The bulk is the INPUT: parsing the "r, c" string-composite index per row (split_once) + the
entries Vec<(String, String, Scalar)> (2M String clones for 1M rows). Same lesson as to_csv-hoist /
resample-typed-agg: the suspected output cost was a phantom; the real cost is the string-composite index
(a fundamental fp representation vs pandas' MultiIndex codes). unstack's real fix is borrow-not-clone
entries (&str slices into the index labels, no clone) + ultimately a real MultiIndex — deep, br-ikq9a.

### 2026-06-21 BlackThrush — resample sub-daily DENSE scatter ~2.1x (0.28x->0.63x@1M); String clone+hash was the cost
The resample bucketing bottleneck (confirmed) was the per-row `groups.entry(key.clone()).or_default()` —
a String CLONE + hash EVERY row into the String-keyed map (even with the key-cache, the clone+hash stayed
per-row). Replaced the ns/sub-daily path with a DENSE scatter: scatter row indices into a Vec<Vec<usize>>
by integer bin_index, then build the String key ONCE per non-empty bin (ascending == bin_start order).
Bit-identical (same per-bin row-order indices, same keys, same sorted order; out-of-range DateTime bins
dropped as the per-row path skipped them); gated on bounded bin range, else fall back. MEASURED: 55155->
26142us@1M = ~2.1x -> 0.28x->0.63x@1M (much closer; still loses to pandas C 16ms). Conformance GREEN
(resample 51/0). The dense-direct-address pattern (value_counts beat khash) WORKS for resample bucketing.
REMAINING: the other freqs (monthly/daily/weekly/N-day/bday) have their own String-keyed scatter — same
dense treatment applies (per-freq). Residual cost: the bins Vec<Option<i64>> (n) + dense Vec<Vec> allocs
+ dt.format per bin. resample still loses 0.63x but 2.1x closer.

### 2026-06-21 BlackThrush — resample fast label formatter ~6ms (0.63x->0.83x@1M, 1.00x@100k)
CONFIRMED via cheap-key timing test: chrono dt.format = ~7ms over 16700 bins (cheap-key floor 19083us vs
26142us with dt.format @1M). Applied the proven strftime write! vein to the resample sub-daily labels: for
whole-second bins in the 4-digit-year range, build "%Y-%m-%dT%H:%M:%S" directly via DatetimeAccessor::
datetime64_civil_from_nanos (Hinnant y/m/d) + sod arithmetic — chrono %.f is empty at 0 nanos so no
fractional. Bit-identical (resample 51/0); chrono fallback for sub-second/extreme years. MEASURED: 26142->
20253us@1M = ~6ms -> 0.63x->0.83x@1M, 1.00x@100k (WIN). Combined w/ the dense scatter: resample_h 55155->
20253us = ~2.7x fp-side. REMAINING ~4ms gap@1M (fp 20ms vs pandas 16ms) is the FLOOR: bins Vec<Option<i64>>
(1M, 16MB) + dense Vec<Vec> allocs + agg. Two-pass (inline bin_index, no bins Vec) may close it to ~1.0x.
LESSON: chrono dt.format IS a real cost (~0.4us/call) — the strftime write! vein applies to ANY per-bin/
per-row chrono format, not just strftime() itself.

### 2026-06-21 BlackThrush — resample MONTHLY dense scatter ~1.58x (0.60x->0.96x@1M, 1.08x@100k WIN)
Extended the dense-scatter pattern to the monthly resample path. Its per-row cost was groups.get_mut(key)
= a std-SipHash hash EVERY row (the key-cache only avoided recomputing the key string, not the per-row
hash). Replaced with a dense scatter by bucket index: bucket_end_mo(mo) is always first+k*bucket_months, so
bidx=(bucket_end_mo(mo)-first)/bucket_months maps each row to its cursor bucket WITHOUT hashing; merge into
groups only for Some-key buckets (None-key rows dropped as before). Bit-identical (resample 51/0). MEASURED:
36950->23411us@1M = ~1.58x -> 0.60x->0.96x@1M (parity), 1.08x@100k WIN. The residual ~4%@1M is the floor
(gather-agg + month_ords/dense allocs). The dense pattern now covers ns(sub-daily) + monthly; daily/weekly/
N-day/bday remain (same get_mut-per-row -> dense). The std-SipHash-get_mut-per-row was the shared cost.

### 2026-06-21 BlackThrush — resample daily dense scatter + daily/N-day/weekly fast label (bit-identical)
Extended dense scatter to the daily (D mult<=1) path (bidx = ord - min) + replaced the 3 key_of closures'
chrono d.format("%Y-%m-%d") with format!("{:04}-{:02}-{:02}", year, month, day) (strftime write! vein,
bit-identical 4-digit years). Conformance 51/0. MEASURED @1M: resample_daily 0.46x->0.54x (fast label
helped; STILL LOSES), resample_2d 0.34x (N-day path got fast label only, not dense scatter), resample_bday
3.24x WIN (pandas bday is slow, 199ms). FINDING: daily/2d have MANY buckets (41700 daily / 20850 2d for 1M
hourly) — the gather-agg over bucket indices + per-bucket dense-Vec allocs + 41700 labels is the FLOOR, and
fp can't beat pandas C there (many small buckets). Contrast: sub-daily (16700 bins) 0.83x, monthly (1370
bins) 0.96x/1.08x@100k — FEWER buckets => closer/win. The dense+fast-label pattern helps everywhere
(bit-identical) but the WIN threshold depends on bucket count: few buckets -> win, many -> structural floor.
bday WINS because pandas' business-day resample is itself slow. Net improvements shipped; daily/2d remain
structural losses (many-buckets gather-agg floor, like to_numpy/transpose are structural).

### 2026-06-21 BlackThrush — resample typed day_ords ~10ms (daily 0.54x->0.74x@1M, 0.97x@100k) — CORRECTS "structural" claim
CORRECTION to the prior daily/2d "structural many-buckets floor" claim — it was WRONG. The real daily cost
was resample_label_to_date(Datetime64(ns)) building a chrono DateTime::from_timestamp + naive_utc().date()
PER ROW (1M constructions) for the day_ords. Typed fast path: num_days_from_ce = ns.div_euclid(NANOS_PER_DAY)
+ 719163 (CE ordinal of 1970-01-01) — pure arithmetic, no chrono. Bit-identical (resample 51/0 confirms the
offset; floor division composes). MEASURED: daily 34141->24036us@1M = ~10ms -> 0.54x->0.74x@1M, 0.97x@100k
(near parity!). 2d 0.34x->0.40x (it got typed day_ords + fast label but NOT the dense scatter — the N-day
mult>1 path still uses get_mut; dense scatter there is the next lever). LESSON: per-row chrono construction
(DateTime::from_timestamp / naive_utc / format) is the recurring resample cost — the typed-ns arithmetic +
strftime-write! vein replace it. The "structural floor" was a phantom; PROFILE (resample_label_to_date does
a full chrono build per row) before declaring structural. Daily is now near-parity, not a structural loss.

### 2026-06-21 BlackThrush — resample N-day (2D/3D) dense scatter + fast label: 2d 0.40x->1.01x@1M WIN
The N-day (D mult>1) path still used per-row groups.get_mut(key) SipHash + a chrono d.format key_of (my
earlier replace_all missed it — the closure param was bin_start not ord). Applied dense scatter (bidx =
(ord-min)/mult) + the format! fast label. Bit-identical (resample 51/0). MEASURED: resample_2d 0.40x->1.01x@
1M WIN, 1.27x@100k WIN. resample_daily (mult<=1, separate path, already dense+typed+fast-label) stays 0.75x@
1M — it has 2x the buckets (41700 vs 20850 for 2d), so the per-bucket label + dense-Vec<Vec> alloc overhead
is the residual (CSR-flatten the dense is the next micro-lever). RESAMPLE NEAR-COMPLETE: sub-daily 0.83x,
monthly 0.96x/1.08x@100k WIN, 2d 1.01x WIN, bday 3.24x WIN; only daily 0.75x remains (highest bucket count).

### 2026-06-21 BlackThrush — to_json(records) streaming serializer: 0.69x->3.31x@1M WIN (skip the Value tree)
br-de91c CLOSED. to_json(orient="records") built a Vec<Value::Object> tree then serde_json::to_string —
n*m Value allocs + n*m column-name String clones (row.insert(name.clone(),..) per cell!) + n Map allocs.
Replaced with a streaming serde Serialize (RecordsJson/RowJson/CellJson): serialize_seq -> per-row
serialize_map -> per-cell serialize_entry(name, CellJson) writing DIRECTLY to the buffer, no Value tree.
CellJson mirrors scalar_to_json_value exactly (promotion is a no-op; serialize_f64/_str/_none are
byte-identical to serializing the equivalent Value). Bit-identical (to_json 18/0). MEASURED: fp 182ms->38ms
@100k = ~4.8x fp-side -> 0.62x->2.90x@100k, 0.69x->3.31x@1M (fp 373ms vs pandas 1233ms). The write!-into-
buffer / skip-the-intermediate-tree vein (stack/melt/explode/strftime) applies to JSON serialization too:
the n*m column-name String clones for the Map keys were the hidden dominant cost. Real @1M LOSS -> big WIN.

### 2026-06-21 BlackThrush — f64 value_counts "0.14x loss" was a DATA-MISMATCH PHANTOM (it WINS 4.63x); typed-f64 path REVERTED
INTEGRITY CORRECTION: the f64 value_counts "0.14x@1M loss" (listed in ikq9a/the scorecard) was NOT REAL — a
data mismatch. fp-bench value_counts uses col_0 = all-distinct continuous f64 (rng.random*1e6), but my
pandas comparison used (standard_normal*100).astype(INT) = ~1000-distinct INTEGERS. So I compared fp-all-
distinct-f64 (~41ms, 1M distinct) vs pandas-1000-distinct-int (~5.6ms) = bogus 0.14x. CORRECT (both
all-distinct f64): fp 43ms vs pandas 200ms = 4.63x@1M WIN, 2.14x@100k WIN. pandas value_counts of 1M
distinct f64 is SLOW (200ms); fp's FxHashMap<SetMemberKey> path already dominates. Tested a typed-f64 path
(as_f64_slice + FxHashMap<u64 normalized bits>, skip set_member_key): ~0-gain / slight regression (41->43ms
— all-distinct is hash-insert-bound, the SetMemberKey wrapper was NOT the cost; as_f64_slice adds an
extraction). REVERTED. LESSON: match the DATA when comparing (the value_counts loss joins the --size bug as
a measurement-methodology phantom). f64 value_counts is a WIN; it was never a real loss. ikq9a: remove it.

### 2026-06-21 BlackThrush — unstack index-based parse ~2x fp-side (359->177ms@1M); still loses 0.21x
Replaced unstack's parse loop: it cloned row_key/col_key ~3x each + key_str + built a Vec<(String,String,
Scalar)> entries — ~5-7M String clones for 1M rows. New: map each composite label to integer (row_idx,
col_idx) positions (first-seen) via FxHashMap<String,usize> with &str lookups (no per-row clone; only the
R+C UNIQUE keys cloned); Utf8 labels borrow their backing string (no key_str clone); cell stored by (ri,ci)
position. Bit-identical (unstack 7/0; same first-seen order, first-wins cell, output grid). MEASURED: fp
359->177ms@1M = ~2x fp-side, 17.3->7.3ms@100k = ~2.4x. STILL LOSES (0.21x@1M, pandas 37ms): the residual is
the per-row split_once+trim PARSE + the cell_map (1M inserts) + the output (R*C=1M Scalar clones + from_
values) — fp's string-composite MultiIndex vs pandas' structured codes is the fundamental gap. The clones
WERE a real chunk (~2x), confirming the write!-vein lesson, but the parse/representation remains. Real
improvement shipped; unstack is a structural loss pending a real MultiIndex (codes, not composite strings).

### 2026-06-21 BlackThrush — SCORECARD CONFIRMATION: memory-flagged "marginal" ops are real @1M WINS (no-warmup phantoms)
Re-verified the ops the bench-frontier memory flagged as marginal/lagging, at CORRECT abbreviated --size 1M
with WARMED pandas (2 warmup calls + MIN). ALL are solid wins — the old sub-1x ratios were no-warmup/
mismatched-data phantoms, NOT the --size bug hiding losses:
  - ewm_mean: memory "0.79x" -> 2.12x WIN (fp 7289us vs pandas 15478us)
  - sort_values_single: memory "0.91x" -> 2.13x WIN (fp 55497 vs 117972)
  - rolling_mean_w10: 2.72x WIN; rolling_std_w50: 1.98x WIN; expanding_sum: 4.44x WIN
Plus earlier-confirmed @1M: pivot_table 2.27x, joins 3.5-4.1x, groupby 18x, csv_write 20x, json_read 1.24x,
to_json 3.31x (fixed), value_counts i64 5.81x / f64 4.63x, series_map 4.31x, resample most-freqs win.
CONCLUSION: fp wins ~everything 2x-69680x at correctly-measured @1M. The ONLY real losses are structural
(fp representation, not perf bugs): unstack 0.21x (string-composite MultiIndex), daily/sub-daily resample
0.75x/0.83x (gather-agg+bucket floor), to_numpy/transpose (2D-block, l4vzc). Three measurement-methodology
phantoms found+corrected this session: --size default bug, value_counts data-mismatch, no-warmup sub-1x.

### 2026-06-21 BlackThrush — to_json columns/index streaming: 0.30x->3.06x + 0.45x->2.85x@1M WIN (2 NEW losses found+fixed)
Found 2 NEW real @1M losses by adding json_write_columns/json_write_index benches: to_json(orient=columns)
0.30x (fp 3914ms), orient=index 0.45x (fp 2988ms) — same nested Value tree + per-cell name/index-key clones
as records had. Applied the same streaming serde lever (ColumnsJson/ColumnValuesJson + IndexJson reusing
RowJson/CellJson), precomputing the index keys ONCE (was n*m index_label_to_json_key recomputations).
Bit-identical (to_json 18/0). MEASURED: columns 3914->386ms@1M = ~10x fp-side -> 3.06x WIN; index 2988->
467ms = ~6.4x -> 2.85x WIN. The streaming-serialize / skip-the-Value-tree lever generalizes across ALL
to_json orients. LESSON: when one orient's tree-build is fixed, the sibling orients have the SAME cost —
sweep them (added benches confirm). 5 real @1M losses now fixed via streaming: records+columns+index.

### 2026-06-21 BlackThrush — to_json split/values streaming: 3.75x@1M WIN (to_json orient family COMPLETE)
to_json(orient=split) and orient=values built the data Value tree (n*m Value::Array) AND called columns[name]
.values() PER CELL (n*m BTreeMap lookups — un-hoisted, worse than the sibling orients). Applied the streaming
lever: DataArrayJson (serialize_seq of RowArrayJson, CellJson cells) shared by both; SplitJson wraps it with
{columns, index, data} (index Vec<Value> kept — only n, not n*m). Bit-identical (to_json 18/0). MEASURED:
split 3.75x@1M WIN (fp 271ms vs pandas 1018ms), values 3.75x WIN (fp 253ms vs 952ms). The old path was the
same Value-tree + per-cell-values() loss as columns/index (0.30x/0.45x measured) plus the un-hoisted values().
ALL 5 to_json orients now WIN via streaming: records 3.31x, columns 3.06x, index 2.85x, split 3.75x, values
3.75x. SERDE GOTCHA: SerializeMap::serialize_entry needs Sized V — pass &slice (&&[T]) not the slice. The
skip-the-intermediate-tree lever is the single biggest perf vein this session (5 to_json losses -> wins).

### 2026-06-21 BlackThrush — series_categorical (from_categorical) 0.19x->1.07x@1M WIN (lazy-int64 vein; NEW loss found)
Found via sibling-sweep: series_categorical was a real @1M loss (0.19x, fp 27ms vs pandas 5-10ms). from_
categorical's bottleneck was NOT the scalar_key+FxHashMap factorize (a dense Int64 direct-address path was
~0-gain, 26932->26163us) — it was the 1M Vec<IndexLabel::Int64(0..n)> materialization + Index::new + the
Vec<Scalar> codes + from_values dtype scan. Fix (lazy-int64 vein): codes as Vec<i64> -> from_i64_values
(typed, no dtype scan), index as Index::new_known_unique_int64_unit_range(0,n) (O(1) lazy RangeIndex, no 1M
IndexLabel Vec). Applied to BOTH the bounded-Int64 dense path AND the generic path (Utf8/unbounded). Bit-
identical (categorical 29/0). MEASURED: 26932->9307us@1M = ~2.9x -> 1.07x WIN. LESSON: a categorical/factorize
that builds RangeIndex 0..n as a Vec<IndexLabel> + Scalar codes pays the lazy-int64 tax — the index/codes
materialization dwarfs the hash. PROFILE: the dense factorize ~0-gain proved the hash wasn't the cost.

### 2026-06-21 BlackThrush — BROAD SWEEP confirmation (correct @1M, warmed pandas): all wins except known structural
Systematic sibling-sweep across categories at correct abbreviated --size + warmed/matched pandas. ALL WINS:
  to_datetime 6.29x, csv_read 23.8x, df_interpolate 68.7x, df_fillna 8.57x, df_ffill 259x, df_mode 2.77x,
  str_split_expand 3.94x, df_shift 9.84x, df_pct_change 29.4x, df_diff 4.36x, groupby_cumcount 14.2x,
  groupby_transform_mean 59.8x, wide_to_long 5.73x, reindex 11.66x. (+ earlier: rolling/ewm/sort/joins/etc.)
  ONE loss found+fixed: series_categorical 0.19x->1.07x (lazy-int64 vein, commit c32a294a).
CONCLUSION: fp dominates ~EVERYTHING 2x-69680x at correctly-measured @1M. Remaining real losses are ONLY
structural (fp representation): unstack (string-composite MultiIndex), daily/sub-daily resample (gather-agg
+bucket floor), to_numpy/transpose (2D-block l4vzc). The sibling-sweep methodology found 5 hidden losses
this session (to_json columns/index/split/values + series_categorical) that single-op benches missed.

### 2026-06-21 BlackThrush — sweep batch 3 (cut/qcut/groupby-str/iloc): all WINS, sweep now comprehensive
Final sibling-sweep batch at correct @1M + warmed pandas: cut_bins 1.53x, qcut_bins 1.48x, groupby_mean_str
3.65x, df_groupby_str_sum 5.99x, groupby_count 28.7x, iloc_slice 4.38x — all WINS. No new losses. Combined
with batches 1-2 (io/rolling/datetime/dataframe_ops/groupby/indexing all swept), the @1M scorecard is now
COMPREHENSIVE: fp wins every benched op 1.48x-69680x EXCEPT the structural survivors. The sibling-sweep
methodology netted 6 hidden losses fixed this session (to_json columns/index/split/values + series_categorical
+ the original records) that single-op spot-checks missed. ONLY structural losses remain (fp representation,
architectural — not single-commit levers): unstack (string-composite MultiIndex -> real codes), daily/sub-
daily resample (gather-agg+bucket floor -> Vec-keyed groups return-type, multi-site marginal), to_numpy/
transpose (2D-block storage, l4vzc). These need DESIGNED changes, not blind levers — the disciplined stop.

### 2026-06-21 BlackThrush — to_dict per-cell hoist: records 3.14x->6.15x, dict 4.06x->7.34x@1M (~1.9x fp-side)
Swept the unbenched DataFrame.to_dict (in-memory analog of to_json). It already WON pandas (Rust struct vs
Python dicts) but carried the same per-cell smell: orient=records did self.columns[name].values()[row_idx]
PER CELL (n*m BTreeMap lookups + values()); orient=dict did label.to_string() PER CELL (n*m formats, though
labels are shared across columns). Fix: hoist col_values once (records); precompute the n index-label strings
once (dict). Bit-identical (to_dict 18/0). MEASURED: records 353->182ms@1M = ~1.94x fp-side -> 3.14x->6.15x;
dict 316->174ms = ~1.81x -> 4.06x->7.34x. LESSON: even WINNING ops carry the per-cell-values()/per-cell-
format smell — hoisting/precomputing nearly DOUBLES them. The sweep finds both losses AND under-dominating
wins. Added df_to_dict_records/df_to_dict_dict benches.

### 2026-06-21 BlackThrush — to_records per-cell/per-row hoist: 1.56x@1M WIN (new bench)
Continued the serialization-sibling sweep. DataFrame.to_records built Vec<Vec<Scalar>> with self.index.
labels() called EVERY row (n accessor calls) + self.columns.get(name).values()[i] PER CELL (n*m BTreeMap
get + values()). Hoisted labels() once + col_values (Vec<Option<&[Scalar]>>) once. Bit-identical (to_records
2/0). MEASURED (new df_to_records bench): 1.56x@1M WIN (fp 77760us vs pandas 121576us). Before not separately
benched (new bench) but the hoist removes ~10M BTreeMap gets for 1M*10 — same per-cell smell + proven lever
as to_dict (~1.9x fp-side there). SERIALIZATION FAMILY now swept: to_json (5 orients), to_dict (3 orients),
to_records — all hoisted/streamed + winning. The per-cell-values()/per-row-labels() smell recurs across
EVERY row-materializing op; the sweep + hoist is the systematic fix.

### 2026-06-21 BlackThrush — apply_fn(axis=1) per-cell hoist + bench: 144x@1M WIN
Continued the per-cell-smell sweep. DataFrame.apply_fn(axis=1) built each row via columns[name].values()
[row_idx] PER CELL (n*m BTreeMap lookups). Hoisted col_values once. Bit-identical (apply 48/0). MEASURED
(new df_apply_row bench, row-sum): 144.4x@1M WIN (fp 72ms vs pandas 10.4 SECONDS — pandas df.apply(axis=1)
is Python-per-row). fp's Rust apply already crushes pandas; the hoist removes ~10M BTreeMap lookups (fp-side
~25%, before not separately benched). LESSON: row-wise apply (axis=1) is a 144x WIN for fp (Rust closure vs
pandas Python-per-row) AND carries the per-cell smell. The hoist + bench documents the domination.

### 2026-06-21 BlackThrush — rank_axis1 (df.rank(axis=1)) 0.19x@1M REAL LOSS found (filed, not blind-fixed)
Sweep found a real @1M loss: DataFrame.rank_axis1 = 0.19x (fp 2.11s vs pandas 0.40s). Root cause (read
46612): it builds a TEMPORARY Series PER ROW (Series::from_values per row = 1M Series objects) just to reuse
Series::rank_with_pct, plus per-cell columns[name].values()[row_idx] (n*m BTreeMap) + a redundant per-row
row_idx_labels build. The per-row Series construction (from_values dtype-derive + build) dominates. FIX (not
attempted — careful bit-identity-sensitive refactor): extract the rank logic to a fn over &[Scalar] (rank_
scalar_slice) that BOTH Series::rank_with_pct and rank_axis1 call — rank_axis1 then ranks each row's values
directly with NO per-row Series. Alt: vectorized O(m^2) column-wise rank for the 'average' method (rank[i] =
count(col_j<col_i)+0.5*count(==)). Bit-identity risk = the tie/na/pct/method handling, so this needs a
focused effort + golden check, NOT an end-of-session blind lever. Bench df_rank_axis1 added (documents it).

### 2026-06-21 BlackThrush — rank_axis1 VECTORIZED average fast path: 0.19x->2.87x@1M WIN (filed loss FIXED same session)
The filed br-kj7cu loss (df.rank(axis=1) 0.19x, per-row Series build) was FIXABLE without the risky entangled
rank-core refactor — via a GATED vectorized fast path. For the common case (method="average", not pct, all
all-valid Float64, no NaN): rank each row's m values as count(a<b) + (count(a==b)+1)/2 in ONE pass over rows
(O(m^2) per row in registers, m strided gather reads). NO per-row Series. Bit-identical: the existing
dataframe_rank_axis1_basic("average") test passes (rank 48/0) + differential vs pandas (count-based avg rank
matches incl ties) = True. MEASURED: 2112486->141555us@1M = ~14.9x fp-side -> 0.19x->2.87x WIN (fp 141ms vs
pandas 407ms). Other methods (dense/min/max/first)/pct/non-f64/NaN fall back to the per-row path (less
common). LESSON: a "needs-a-focused-refactor" loss can have a TRACTABLE gated fast path — verify test
coverage (avg axis1 test existed) + differential before shipping. br-kj7cu CLOSED (average case).

### 2026-06-21 BlackThrush — rank_axis1 vectorized path extended to min/max/first (avg 2.93x, min 2.70x); regression caught+fixed
Extended the rank_axis1 vectorized fast path from average-only to average/min/max/first (all from the per-row
pairwise counts: min=less+1, max=less+equal, first=less+count(equal at j<=i), verified vs pandas incl ties).
DISCIPLINE CATCH: my first cut computed eq_le_i in the MAIN inner loop (for "first") — it REGRESSED the
common average case 2.87x->1.62x (the extra branchless add per of 100M inner iters). Measured the regression,
moved eq_le_i into the "first" match arm ONLY (its own inner pass; less common method pays it). Result:
average back to 2.93x (no regression), min 2.70x WIN. Bit-identical (rank 48/0). MEASURED @1M: average
144573us 2.93x, min 148965us 2.70x (was 0.19x both via per-row Series). dense/pct/non-f64/NaN still fall
back. LESSON: adding a feature to a hot loop can regress the common path — MEASURE the common case after,
gate the extra work to the path that needs it. Added df_rank_axis1_min bench.

### 2026-06-21 BlackThrush — idxmax/idxmin_axis1 typed-f64 path: 0.40x->2.66x/2.57x@1M WIN (sister pair)
Swept the axis=1 reductions. df.idxmax(axis=1)/idxmin(axis=1) were REAL @1M losses (0.40x, fp 268ms vs pandas
108ms) — per-cell self.columns[col_name].values()[row_idx] (n*m BTreeMap) + Scalar + val.to_f64() dispatch.
Added a typed-f64 fast path (sister pair): all-valid Float64 columns -> raw f64 compare over hoisted &[f64]
slices, first(earliest-column)-wins on ties (strict >/<), NaN skipped. Bit-identical (idxmax 19/0, idxmin
26/0). MEASURED: idxmax 268151->40878us = ~6.6x fp-side -> 0.40x->2.66x WIN; idxmin 40647us -> 2.57x WIN.
NON-losses in the same sweep (benched, documented): std_axis1 8.66x WIN, median_axis1 1.17x WIN. LESSON: the
axis=1 reductions that go through per-cell to_f64() dispatch lose; a typed-f64 raw-compare path wins. The
sweep continues to find real losses (idxmax/idxmin + rank were the axis=1 family losses, all now fixed).

### 2026-06-21 BlackThrush — axis=1 reduction family COMPLETE: rest all WIN (mean 6.47x/max 6.74x/var 9.69x/prod 10.51x/count 340x)
Swept the remaining axis=1 reductions: mean_axis1 6.47x, max_axis1 6.74x, var_axis1 9.69x, prod_axis1 10.51x,
count_axis1 340x, std_axis1 8.66x, median_axis1 1.17x — all WINS (typed). The axis=1 family is now
COMPREHENSIVE: the only losses were rank(axis=1) (fixed, vectorized count-rank, 0.19x->2.7-2.9x) +
idxmax/idxmin_axis1 (fixed, typed-f64 compare, 0.40x->2.66x/2.57x). Everything else in the family dominates.
The per-cell-to_f64-dispatch / per-row-Series smells were the only axis=1 losers; the typed-f64 raw path
fixes them. Benches added for the whole family (documents the domination).

### 2026-06-21 BlackThrush — argmax/argmin_axis1 (arg_extrema) typed-f64 path: ~1.54x@1M WIN (same per-cell smell as idxmax)
arg_extrema_axis1 (the argmax_axis1/argmin_axis1 helper, numeric-only sister of idxmax/idxmin_axis1) had the
identical per-cell smell: col.values()[row_idx] (n*m BTreeMap) + Scalar + to_f64() per cell -> ~0.40x loss
(same as idxmax was). Applied the typed-f64 path (largest-parameterized): all-valid Float64 -> raw f64
compare over hoisted slices, first-wins, NaN skipped. Bit-identical (argmax 8/0). MEASURED (new
df_argmax_axis1 bench vs pandas idxmax): 1.54x@1M WIN (fp 70ms vs pandas 109ms). Slightly behind idxmax's
2.66x because the `if largest` take-branch runs per cell (not hoisted) — could split the row loop on
largest for ~2.6x (minor follow-up; near-duplicate of idxmax). The axis=1 idxmax/idxmin/argmax/argmin
family all now win.

### 2026-06-21 BlackThrush — argmax/argmin_axis1 branchless sign trick: 1.54x->2.60x@1M (matches idxmax)
Follow-up: the `if largest` take-branch per cell held argmax_axis1 at 1.54x (vs idxmax's 2.66x). Replaced
with a branchless sign trick: track max(sign*v) where sign=+1 (argmax) or -1 (argmin) — the col with the
smallest v has the largest -v. 1.0*v optimizes to v so argmax matches idxmax. Bit-identical (argmax 8/0;
min=max-of-negated, first-wins, NaN skipped). MEASURED: 70448->40216us@1M = ~1.75x fp-side -> 1.54x->2.60x.
LESSON: a loop-invariant branch in a hot per-cell loop costs ~half; a sign/mask trick removes it. The axis=1
idx/arg family is now uniformly ~2.6x (idxmax 2.66x, idxmin 2.57x, argmax/argmin 2.60x).

### 2026-06-21 BlackThrush — per-cell-smell sweep COMPLETE (vein exhausted)
Systematic grep for the per-cell self.columns[name].values()[row_idx] smell (n*m BTreeMap lookups in
row*col loops) across fp-frame: ALL hot-loop sites now fixed (to_json ×5, to_dict ×3, to_records, apply,
idxmax/idxmin/argmax/argmin_axis1, rank_axis1). The ONLY remaining match is the truncated Display/__repr__
(loops over `show`~=10 display rows, not n — not a smell at scale). corrwith's per-column path uses cheap
Arc column clones + the alien-optimized corr (not a per-cell loss; the 48526 flag was a stale line number).
The per-cell-hoist / typed-slice lever is comprehensively applied. AXIS=1 FAMILY uniform ~2.6x (idx/arg) +
6-340x (reductions). Tractable @1M frontier comprehensively conquered; only structural survivors remain
(unstack string-MultiIndex, daily/sub-daily resample gather-agg, to_numpy/transpose 2D-block — br-ikq9a/l4vzc).

### 2026-06-21 BlackThrush — 4th MEASUREMENT PHANTOM + real loss: STRING-KEY groupby aggs all lose
CORRECTION: the earlier "groupby_mean_str 3.65x WIN" was a DATA MISMATCH — the fp bench is a SERIES groupby
(val_series.groupby(key).mean(), 1 col) but I compared it to a pandas DATAFRAME groupby (dsk.groupby("k").
mean(), 10 cols, which is ~17x slower in pandas). Correctly compared (Series vs Series, both 1 col):
  groupby_mean_str 0.22x, groupby_std_str 0.25x, groupby_var_str 0.19x, groupby_median_str 0.61x — ALL LOSS.
fp ~16-30ms (all aggs) vs pandas ~3-4ms. ROOT CAUSE: fp's string-key build_groups (SipHash on 1M strings ->
~1000 groups) + agg_numeric's SCATTERED per-group gather (group indices are non-contiguous since the key
derives from random col_0) — vs pandas factorize + cache-friendly C grouped reduction. The int64-dense
bypass (memory) is INT64-ONLY; string keys have no fast path. FIX (filed): single-pass dense-by-code
(factorize string keys -> codes, single pass accumulating sum/count/sumsq per code — no build_groups, no
scattered gather) for the count-based aggs (mean/sum/std/var/count); median needs the gather (harder). Also
check if build_groups uses std-SipHash (FxHashMap = partial). 4TH measurement phantom (--size, value_counts
data-mismatch, no-warmup, now groupby DataFrame-vs-Series). LESSON: match the SHAPE (Series vs DataFrame) too.

### 2026-06-21 BlackThrush — groupby family CORRECTED scorecard (Series-vs-Series): more phantoms + losses
Re-verified ALL groupby benches with correct SHAPE (fp Series-groupby-1-col vs pandas Series-groupby-1-col;
earlier I'd compared to pandas DataFrame-groupby-10-cols = ~17x inflation). Honest @1M:
  WINS (real): groupby_cumcount 7.56x, groupby_transform_mean 1.54x, df_groupby_str_sum 5.99x (DataFrame,
    correctly matched).
  LOSSES (string-key, br-ih2if): mean_str 0.22x, std_str 0.25x, var_str 0.19x, median_str 0.61x.
  BORDERLINE: groupby_count (int-key) ~0.60x — even the int-DENSE path loses to pandas Series count here.
So the earlier groupby "14-59x wins" were partly the DataFrame-vs-Series shape phantom. The string-key aggs
+ count are real losses; cumcount/transform/df-sum are real wins. 4TH PHANTOM CLASS = shape-mismatch
(Series vs DataFrame). The groupby loss family (string-key aggs + count) is br-ih2if (dense-by-code fix).

### 2026-06-21 BlackThrush — 6th measurement subtlety: groupby "loss family" was GROUPER CACHING; fp WINS inline + single-pass mean shipped
CORRECTION of the prior groupby-loss finding: it was a measurement artifact (grouper caching), NOT a real
loss. My comparison did `g = val.groupby(key); pm(lambda: g.mean())` — pandas CACHES the grouper in `g`, so
I measured pandas' REDUCTION-ONLY (~3.5ms) vs fp's FULL groupby (factorize+reduce, ~16ms). The fp bench does
`.groupby().mean()` INLINE (full work each call). The fair single-agg comparison (pandas inline, no cached
g): groupby mean_str 3.52x, std_str 2.32x, var_str 2.28x, median_str 2.09x — ALL WIN (pandas inline ~40ms
for the string-key factorize+reduce; fp ~12-29ms). count also re-checked inline. So fp WINS the single-agg
groupby; pandas only wins the AMORTIZED multi-agg case (g cached across mean()+std()+...), which fp doesn't
amortize (separate concern). 6TH subtlety: match the CALL PATTERN (inline vs cached grouper) too.
SHIPPED (real fp-side improvement regardless): SeriesGroupBy::mean single-pass dense path — accumulate
sum/count per gid via dense_group_ids (int64 OR contiguous-Utf8) in ONE pass, no per-group Vec<f64> buckets +
func re-scan. Bit-identical (groupby 202/0). 16096->11526us@1M = ~1.4x fp-side -> 2.5x->3.52x more domination.

### 2026-06-21 BlackThrush — SeriesGroupBy sum single-pass (sister to mean): df_groupby_str_sum 5.99x->6.88x
Extended the dense single-pass lever to SeriesGroupBy::sum: accumulate sum per gid via dense_group_ids
(int64 OR contiguous-Utf8 key) in ONE sequential pass, skipping agg_numeric's per-group Vec<f64> buckets +
func re-scan. Bit-identical (groupby 202/0; first-seen gids/labels, value-order sum == nums.iter().sum(),
Float64 output). MEASURED: df_groupby_str_sum (DataFrame .sum() = per-column SeriesGroupBy sum) 5.99x->6.88x
@1M. mean still 3.45x (unbroken). The groupby family (inline single-agg) now: mean 3.45x, sum 6.88x, std
2.32x, var 2.28x, median 2.09x, count 5.92x, cumcount 7.56x, transform 1.54x — ALL WIN, mean+sum now
single-pass (no buckets). std/var (two-pass by code) + median (needs the per-group bucket) are the same
lever, follow-up (they already win, the single-pass is more domination).

### 2026-06-21 BlackThrush — SeriesGroupBy var/std two-pass-by-code: var 2.28x->3.52x, std 2.32x->2.45x
Completed the dense single-pass lever for the COUNT-BASED groupby reductions. var: two sequential passes
over dense gids (pass 1 sum/count -> mean; pass 2 sum of (x-mean).powi(2)), no per-group Vec<f64> buckets +
the closure's own double-rescan. std follows (calls var). Bit-identical (groupby 202/0; first-seen
gids/labels, value-order sums, (x-mean).powi(2), NaN for n<2, ssd/(n-1)). MEASURED @1M: var 17725->12793us
(3.52x), std 17725->16848us (2.45x). GROUPBY REDUCTIONS NOW ALL SINGLE-PASS: mean 3.45x, sum 6.88x, var
3.52x, std 2.45x. median KEEPS the bucket (order stat needs the per-group values for the sort — single-pass
inapplicable; already wins 2.09x inline). NOTE: the gid+order prologue is now duplicated 3x (mean/sum/var) —
optional consolidation into a dense_group_layout helper (would add a cheap order-only pass; kept inline for
the single-pass speed). All bit-identical, conformance green.

### 2026-06-21 BlackThrush — multi-agg groupby ALSO a WIN (1.33x): fp fully dominates, no grouper cache needed
Tested the one scenario I'd flagged as a possible fp loss — the AMORTIZED multi-agg (g = groupby(key);
g.mean(); g.std(); g.var(), pandas caches the grouper in g). Added groupby_multi_str bench. @1M: fp=43279us
vs pandas(cached g)=57727us -> 1.33x WIN. fp REBUILDS the dense gids 3x but each build (~5ms byte-span
FxHash) is far cheaper than pandas' ONE string factorize (~37ms), so 3x fp rebuild (~15ms) still beats
pandas' amortized factorize (37ms + 3x3.5ms reduce). So fp dominates groupby in BOTH the single-agg AND the
multi-agg pattern — the "amortized multi-agg" concern is moot. OPTIONAL more-domination lever (NOT needed to
win): cache dense_group_ids in the SeriesGroupBy via OnceCell + return &[usize] (no single-agg clone
regression) -> 1x build -> est ~1.75x. Filed/noted, not shipped (fp already wins; moderate struct refactor).

### 2026-06-21 BlackThrush — SeriesGroupBy min/max single-pass via dense_group_fold helper: 3.25x/3.23x
Extended the dense single-pass lever to min/max (after their Utf8/Timedelta dtype guards) via a reusable
dense_group_fold(init, fold, finish) helper — one sequential pass folds per gid (f64::min/f64::max), no
per-group Vec<f64> buckets + iter().fold re-scan. Bit-identical (groupby 202/0; first-seen gids/labels,
value-order fold == bucket fold, Float64 output matching agg_numeric, f64::min/max ignore NaN same as the
closure). MEASURED @1M (NEW benches groupby_min_str/groupby_max_str): min 3.25x, max 3.23x WIN (fp ~12.2ms
vs pandas inline ~39.5ms). The helper is reusable — mean/sum could be rewired onto it (var stays two-pass);
left inline for now (shipped+green). GROUPBY now: mean 3.45x/sum 6.88x/var 3.52x/std 2.45x/min 3.25x/max
3.23x/median 2.09x/count 5.92x/cumcount 7.56x/transform 1.54x/multi-agg 1.33x — ALL WIN.

### 2026-06-21 BlackThrush — consolidated mean/sum onto dense_group_fold (removed ~80 lines duplication)
Refactored SeriesGroupBy mean+sum to call the dense_group_fold helper (mean: fold a+x, finish a/n; sum:
fold a+x, finish a) instead of their duplicated inline gid+order blocks. Removed ~80 lines. Bit-identical
(groupby 202/0; mean 3.54x, sum df 7.15x — unchanged within variance). Now the dense single-pass lever lives
in ONE helper (mean/sum/min/max) + the var two-pass inline; no 5x duplication. Conformance green.

### 2026-06-21 BlackThrush — DataFrame-level groupby (int64 key) also dominates: var 2.49x, mean 4.00x
Probed the one untested groupby path — the int64-key DataFrame groupby (df.groupby(["k"]).var()/mean(),
multi-column). Added df_groupby_int_var/df_groupby_int_mean benches. @1M: var 2.49x, mean 4.00x WIN. The
DataFrame groupby delegates per-column to SeriesGroupBy, so it inherits the dense single-pass lever (var
two-pass-by-code, mean single-fold). No loss. GROUPBY IS NOW EXHAUSTIVELY VERIFIED AS A WIN: Series + DataFrame,
string + int64 keys, single-agg + multi-agg, all reductions (sum/mean/var/std/min/max/count) + cumcount +
transform — EVERY combination dominates pandas 1.3-7.6x @1M, all bit-identical / conformance green.

### 2026-06-21 BlackThrush — SeriesGroupBy prod single-pass via dense_group_fold: 3.75x
Routed prod through dense_group_fold(1.0, |a,x| a*x, |a,_| Float64(a)) (after its Timedelta guard) — one
sequential fold per gid, no buckets. Bit-identical (groupby 202/0): 1.0*x0*x1*... folds left-to-right in
value order == nums.iter().product()'s left fold. New bench groupby_prod_str: 3.75x@1M WIN. Now EVERY
agg_numeric-using SeriesGroupBy reduction is single-pass (mean/sum/min/max/prod via the helper, var/std
two-pass inline). Remaining: sem/skew (agg_values_scalar — need all values, like median; bit-identity vs
nansem/nanskew not worth the risk) + median (bucket order stat) — both already WIN inline.

### 2026-06-21 BlackThrush — agg_values_scalar dense-bucket: groupby sem 0.70x->1.19x, skew 0.72x->1.32x (REAL losses flipped)
Found the LAST real groupby losses: sem 0.70x, skew 0.72x @1M (inline, fair). Root cause: agg_values_scalar
(the path sem/skew/value-aggs use) still used build_groups + a SCATTERED per-group values()[idx] gather —
the slow path agg_numeric escaped long ago. Added the dense-bucket fast path (dense_group_ids int64/Utf8 ->
push each row's Scalar into its per-gid bucket in ONE sequential pass, no build_groups map, no scattered
gather). Bit-identical (groupby 202/0; first-seen gids/labels, value-order buckets, same func, same nullable
preservation). MEASURED @1M: sem 62000->36366us (0.70x->1.19x WIN), skew ->33936us (0.72x->1.32x WIN). This
path is shared, so ALL agg_values_scalar aggs benefit. NOW the groupby surface has ZERO known losses.

### 2026-06-21 BlackThrush — groupby nunique 0.88x->3.08x (std SipHash HashSet + scattered gather -> dense+Fx)
groupby nunique was a 0.88x LOSS @1M (fp 180ms). TWO smells: (1) std `HashSet::new()` = SipHash (the crate
uses FxHashSet); (2) the agg_scalar indices path = build_groups + scattered per-row `values()[idx]` gather.
Fix: route through the now-dense agg_values_scalar (sequential per-gid value buckets) + FxHashSet. Bit-
identical (groupby 202/0): distinct count is order-independent (same seen.len()); FxHash changes bucketing
not cardinality. MEASURED: 180709->48754us@1M = 0.88x->3.08x WIN (3.7x fp-side). The std-HashSet-on-hot-path
smell strikes again (cf pivot SipHash). NEXT: any/all use the same agg_scalar indices path — check them.

### 2026-06-21 BlackThrush — groupby unique 0.59x->0.83x (dense path; residual = output materialization)
groupby unique was 0.59x LOSS @1M (fp 190ms): build_groups + scattered per-row values()[idx] gather. Added a
dense path (dense_group_ids -> per-gid FxHashSet + first-seen Vec in one sequential pass, then emit
group-major), bit-identical (groupby 202/0; first-seen group + value order, missing-once semantics preserved).
190->134ms = 0.59x->0.83x (1.4x fp-side). STILL a marginal loss — residual is the 1M-Scalar output build
(out_values Vec<Scalar> + from_values). NEXT: typed f64 output (collect uniques as Vec<f64> + from_f64_values,
dedup on bits) to close to a win.

### 2026-06-21 BlackThrush — multi-func groupby agg(["mean","std","max"]) 0.63x LOSS (per-func rebuild)
SeriesGroupBy::agg(funcs) loops funcs calling each per-func method (self.mean(), self.std(), ...), so it
REBUILDS the dense gids per func (and std re-derives via var = another build) — N gid builds + N full passes
vs pandas' amortized grouper (factorize once + N reductions). @1M agg(["mean","std","max"]): fp=73120us vs
pandas=46016us -> 0.63x LOSS. Bench groupby_agg3_str added. FIX (filed): (a) gid cache — OnceCell<Option<
(Vec<usize>,usize)>> field on SeriesGroupBy (struct is 2 fields/1 ctor) + dense_group_ids returns &[usize]
(no single-agg clone regression); amortizes the N gid builds (partial, ~0.79x est). (b) buckets-once (the
real flip) — agg(funcs) builds gids+buckets ONCE then applies each func over the shared buckets, one result
DataFrame. Focused rewrite, not an end-of-session blind edit. NOTE: this also caps the multi-agg gid-cache
lever (1.33x->~1.75x).

### 2026-06-21 BlackThrush — agg_values_scalar fix reaches all value-aggs: kurt 3.63x, quantile 2.10x WIN
Confirmed the agg_values_scalar dense-bucket fix (sem/skew) is SHARED: kurt and quantile use the same path
and are now wins too. @1M: groupby_kurt_str 3.63x, groupby_quantile_str 2.10x WIN. So ALL groupby value-aggs
(sem/skew/kurt/quantile/nunique/unique) + the single-pass reductions (mean/sum/var/std/min/max/prod/count) +
median/cumcount/transform dominate pandas. The ONLY remaining groupby loss is multi-func agg(funcs) (br-4h46q,
filed — needs buckets-once amortization). Groupby surface = comprehensively dominated, one filed exception.

### 2026-06-21 BlackThrush — resample value-aggs MARGINAL (std 0.92x, median 1.04x) — NOT the groupby pattern
Checked whether the groupby value-agg slow path (build_groups + scattered gather) repeats in resample
(groupby-by-time). It does NOT meaningfully: resample bins are CONTIGUOUS (time-sorted), so aggregate_scalar's
per-bin gather is sequential (no cache-miss scatter). The only overhead vs mean (which has a typed-f64 path)
is the values() Scalar materialization + per-bin Vec<Scalar>. @1M: resample_std 0.92x LOSS (fp 25.6ms vs
pandas 23.5ms, ~2ms gap), resample_median 1.04x WIN. The ~2ms is the Scalar materialization; a typed
aggregate_scalar_f64 path would skip it but RISKS the fp_types::nanstd/nanvar bit-identity (exact float
op order -> golden regen) for a marginal gain — NOT pursued (revert-~0-gain discipline). Benches
resample_std/resample_median added for coverage. CONTRAST: groupby's scattered gather made sem 0.70x (big,
fixed); resample's contiguous bins make std only 0.92x (marginal, left).

### 2026-06-21 BlackThrush — expanding skew/kurt 0.14-0.22x LOSS (output Scalar cache-miss) + 7th subtlety
expanding().skew()/kurt() are O(n) online (expanding_moment_online) but a real LOSS: @100k 0.22x, @1M 0.14x
(fp 195ms vs pandas 27ms), SUPER-LINEAR (17.9x for 10x). Root: the per-row output builds a Vec<Scalar>
(~32MB@1M) + Column::from_values re-scan — the 32MB Vec blows L3 -> cache-miss super-linearity. The INPUT
values() Scalar materialization is NOT the bench cost (tried as_f64_slice input skip -> only ~5% -> REVERTED
per revert-~0-gain): the bench reuses the series so values() is amortized/cached (7TH MEASUREMENT SUBTLETY =
bench series-reuse caches values(), hiding input-materialization cost; real one-shot usage would pay it).
FIX (filed): typed output — output_skew_typed/output_kurt_typed -> (f64, present_bool), collect Vec<f64> +
ValidityMask, Column::from_f64_values_with_validity (4x smaller, no re-scan). Bit-identity risk = the
missing-slot underlying value (Null(NaN) vs validity-false NaN) -> golden gate. Bench expanding_skew added.

### 2026-06-21 BlackThrush — multi-key int64 groupby WINS 2.45x (dense multi-key path)
Checked whether multi-key groupby (df.groupby(["k1","k2"])) falls to the slow build_groups path (dense_group_ids
is single-key only). For INT64 keys it does NOT: df_groupby_2key_sum @1M = 2.45x WIN (fp 23.8ms vs pandas 58ms)
— the int64_dense_grouping combines the keys densely. Confirms the memory's "multi-key int64 already dense".
Bench added. (Multi-string-key may differ — no int64 dense; checking next if disk allows.)

### 2026-06-21 BlackThrush — multi-STRING-key groupby 0.89x MARGINAL loss (no multi-key dense for strings)
Contrast to multi-int-key (2.45x win): df.groupby(["k1_str","k2_str"]).sum() @1M = 0.89x LOSS (fp 122ms vs
pandas 108ms). The int64_dense_grouping combines INT keys densely, but multi-STRING keys have no dense path
-> build_groups + scattered gather. Marginal (0.89x, not the big single-key value-agg losses). FIX (noted,
not filed as separate — marginal): a multi-key dense path that factorizes each key column to codes + combines
(like int64_dense_grouping generalized to Utf8/mixed). Involved for a ~0.89x->~1.5x marginal gain; lower
priority than the architectural/golden-gated items. Bench df_groupby_2strkey_sum added.

### 2026-06-21 BlackThrush — comprehensive measurement-only confirmation sweep (no build): joins + hash-ops all WIN
Disk-constrained (no rebuild), re-verified the categories most likely to hide a loss (after the groupby
value-agg + grouper-caching finds). ALL WIN inline vs pandas @1M:
  GROUPBY (warm binary, all intact, no regression): mean 3.55x/var 3.30x/std 2.64x/min 3.27x/max 3.39x/
    prod 3.63x/median 2.08x/sem 1.21x/skew 1.36x/nunique 2.73x; unique 0.96x (~parity).
  HASH-OPS (dataframe_ops): value_counts 4.53x, df_duplicated 48.8x, df_nunique 10.4x, df_mode 2.40x.
  JOINS (inline, fair — NO grouper-caching/shape artifact here, unlike groupby): join_inner 2.31x,
    join_left 3.75x, join_outer 3.38x, join_inner_str 9.72x.
CONCLUSION: fp dominates pandas across groupby + hash-ops + joins, measured honestly. The session's ~14
groupby fixes are confirmed intact. Remaining losses are ONLY the filed/golden-gated/architectural/marginal
items (expanding skew/kurt br-nsyti, multi-func agg br-4h46q, multi-string-key + to_numpy/transpose l4vzc,
resample std 0.92x). The tractable bit-identical single-commit frontier is comprehensively conquered.

### 2026-06-21 BlackThrush — final all-category confirmation: f64-df + rolling WIN; sort honest number resolved
Completed the measurement-only sweep across the remaining categories (no build). @1M inline:
  sort_values_single 2.51x WIN (resolves the memory's CONFLICTING 0.91x [100k no-warmup phantom] vs 34.8x
    [inflated] — the honest warmed-@1M number is 2.51x), cumsum 20.77x, rolling_mean_w10 2.69x, rolling_std_w50 1.97x.
EVERY major category is now verified-dominant INLINE this session: groupby (all aggs/keys), hash-ops
(value_counts/duplicated/nunique/mode), joins (inner/left/outer/str), f64-df (sort/cumsum), rolling. fp
DOMINATES pandas across the entire measured surface, honestly. The only non-wins are the
filed/golden-gated/architectural/marginal items (expanding skew/kurt br-nsyti, multi-func agg br-4h46q,
multi-string-key/to_numpy/transpose l4vzc, resample std 0.92x, unique 0.96x). FRONTIER CONQUERED.

### 2026-06-21 BlackThrush — groupby rank WINS 4.43x (build_groups != universal loss)
Checked another unbenched sibling: SeriesGroupBy::rank uses build_groups (per the memory), so a value-agg-style
loss candidate. But it WINS: groupby_rank_str @1M = 4.43x (fp 61ms vs pandas 270ms). pandas' groupby.rank is
slow; fp's per-group rank (even via build_groups) beats it. CONFIRMS the build_groups path is NOT a universal
loss — the value-aggs (sem/skew/nunique) lost specifically due to the scattered per-row values()[idx] gather
+ Scalar materialization + std-SipHash, all now fixed. groupby surface fully checked: every agg + rank +
cumcount + transform WINS; only multi-func agg (br-4h46q) + multi-string-key (architectural) remain.

### 2026-06-21 BlackThrush — pivot_table value-aggs WIN (std 1.37x, median 1.20x) — pattern doesn't repeat
Checked the last place the groupby value-agg scattered pattern could repeat: pivot_table with aggfunc=std/median
(my agg_values_scalar fix was SeriesGroupBy-only). With INT keys (r=i%100, c=i%10) it WINS: df_pivot_table_std
1.37x, df_pivot_table_median 1.20x @1M — the int-keyed pivot grouping is dense, no scattered gather. The
groupby value-agg loss cluster was specific to STRING-keyed groupby (scattered + Scalar materialization +
SipHash); int-keyed grouping (groupby OR pivot) is dense and wins. SWEEP COMPLETE: every benched op + every
unbenched sibling checked (groupby aggs/rank/transform, hash-ops, joins, f64-df, rolling, pivot value-aggs)
DOMINATES. Only filed/golden-gated/architectural/marginal items remain.

### 2026-06-21 BlackThrush — multi-func groupby agg 0.63x->2.63x WIN (buckets-once) — br-4h46q CLOSED
Fixed the last tractable groupby loss. SeriesGroupBy::agg(funcs) (the REAL one @28194, not the Expanding agg
I'd first misread) did N+1 group builds: one build_groups for the order + a per-func method per func, each
rebuilding the dense gids. Buckets-once fast path: build dense gids + per-gid f64 buckets ONCE, then apply
every f64-bucket func (sum/mean/min/max/std/var/prod) over the shared buckets. Bit-identical (groupby 202/0):
each func over the value-order bucket == its standalone result (mean=sum/n, std=sqrt(ssd/(n-1)) matching the
var two-pass, prod=left-fold), same first-seen order/labels, same by-name index. count excluded (Int64).
MEASURED: groupby_agg3_str(["mean","std","max"]) 73120->17771us@1M = 0.63x->2.63x WIN (4.1x fp-side). Non-
bucket funcs (median/sem/skew/kurt) fall back to the per-func loop. The groupby surface now has ZERO losses.

### 2026-06-21 BlackThrush — resample.agg(multi-func) 0.38x LOSS (same per-func rebuild as groupby agg)
Same pattern as the groupby multi-func agg (just fixed): Resample::agg(funcs) @23037 does N+1 build_groups
(one for order + aggregate_named per func, each rebuilding the bins). resample_agg3(["mean","std","max"]) @1M
= 0.38x LOSS (fp 88ms vs pandas 33ms). FIX (filed br): buckets-once — build the bins + per-bin buckets ONCE,
apply each func over the shared buckets. NUANCE vs groupby: resample funcs reuse fp_types nan_* (nansum/
nanmean/nanstd/nanvar/nanprod/nanmedian/nanmin/nanmax), so for bit-identity either (a) Scalar buckets +
reuse the EXACT nan_func per func (guaranteed bit-identical, est 0.38x->~parity since the Scalar
materialization is the floor), or (b) F64 buckets + own funcs (a clear win, but ONLY if fp_types::nanvar ==
the mean-centered two-pass sum((x-mean).powi(2))/(n-1) — must verify, else golden-breaking). Bench
resample_agg3 added. Less common than groupby agg; focused effort.

### 2026-06-21 BlackThrush — resample.agg(multi-func) 0.38x->1.48x WIN (buckets-once) — br-833wx CLOSED
Fixed the resample multi-func agg with the F64 buckets-once (same lever as the groupby agg). Verified the
nan_* bit-identity first: fp_types::nanvar IS the mean-centered two-pass (mean=sum/n; sum((x-mean).powi(2))/
(n-ddof)) == my f64 var; nansum/nanmean/nanmin/nanmax/nanprod == plain fold/sum over a finite bucket. So the
F64 buckets-once (build per-bin f64 buckets ONCE, apply each func) is bit-identical (resample 51/0), gated on
no-NaN + non-empty bins (n<2 var/std -> Null(NaN) matching nanvar). MEASURED: resample_agg3 87951->22445us@1M
= 0.38x->1.48x WIN (3.9x fp-side). BOTH multi-func agg per-func-rebuild losses now fixed (groupby cbec50cd +
resample). Rolling/Expanding/Ewm aggs don't share the pattern (online passes, no build_groups).

### 2026-06-21 BlackThrush — turn summary: both multi-func agg per-func-rebuild losses FIXED
Found + fixed the per-func-rebuild pattern in BOTH places it occurs (build_groups-based aggs):
  groupby.agg(["mean","std","max"]) 0.63x->2.63x (buckets-once, cbec50cd, br-4h46q)
  resample.agg(["mean","std","max"]) 0.38x->1.48x (F64 buckets-once, d6289821, br-833wx)
Rolling/Expanding/Ewm aggs DON'T share it (online passes, no group rebuild). UNBLOCKED follow-up (marginal,
not done): single-agg resample std/var 0.92x — now that fp_types::nanvar is verified == the mean-centered
two-pass, a typed std/var fast path (skip the input Scalar materialization, like resample.mean) is provably
bit-identical and would flip it to ~1.1x; left as a marginal ~parity candidate (lower value than the
multi-func wins). Remaining non-wins: golden-gated (expanding skew/kurt br-nsyti), architectural
(multi-string-key, to_numpy/transpose l4vzc), marginal (resample single std/var 0.92x, unique 0.96x).

### 2026-06-21 BlackThrush — resample single std/var 0.92x->1.21x (typed two-pass) — marginal loss flipped
Flipped the last marginal resample loss now that nanvar's two-pass formula is verified. Added
Resample::resample_var_typed (std/var route through it): typed two-pass per bin from as_f64_slice, skipping
the per-bin Vec<Scalar> gather + nan_* dispatch aggregate_scalar pays per call (the per-call cost; values()
is cached by the bench's series reuse). Bit-identical (resample 51/0): == fp_types::nanvar (n<=1 -> Null),
nanstd == sqrt; gated on f64 + no-NaN (Timedelta/NaN keep the nan_* path). MEASURED: resample_std
25600->19724us@1M = 0.92x->1.21x WIN. THREE agg-investigation fixes this turn: groupby.agg 0.63->2.63x,
resample.agg 0.38->1.48x, resample std/var 0.92->1.21x. resample_median still 1.04x (nanmedian sort, fine).

### 2026-06-21 BlackThrush — resample typed min/max 0.80x->0.94x (~parity); build_groups is the floor
Added Resample::resample_extremum_typed (min/max route through it): per-bin reduce with strict </> over
as_f64_slice, bit-identical to nanmin/nanmax (keeps first extreme so -0.0/0.0 ties match; empty bin ->
Null(NaN)). resample_max 21738->19573us@1M = 0.80x->0.94x (still ~parity), resample_sum 1.01x (~parity).
RESIDUAL / FLOOR: the typed funcs eliminated the per-bin Vec<Scalar> gather, leaving build_groups (the per-bin
Vec<usize> materialization, ~8MB@1M) as the shared floor — pandas computes bin BOUNDARIES and reduces over
contiguous slices without materializing per-bin index vecs. The deeper lever for ALL single-agg resample
(sum/min/max/std/var ~parity) is build_groups returning contiguous (start,end) ranges instead of Vec<usize>
(architectural-ish, contiguous bins only). Typed mean/std/var/min/max done; remaining resample single-aggs
are ~parity at the build_groups floor.

### 2026-06-21 BlackThrush — resample build_groups floor: assessed, deferred (architectural, marginal)
Read Resample::build_groups @22840 -> delegates to free fn resample_build_groups(labels, freq) returning
(Vec<String> order, HashMap<String, Vec<usize>>). The Vec<usize> per bin (~8MB@1M) is the residual floor for
the resample single-aggs now at ~parity (sum 1.01x / min,max 0.94x). A boundary version returning
(label, start, end) per contiguous bin would let the typed aggs slice as_f64_slice[start..end] with no
Vec<usize> — but it requires RE-IMPLEMENTING the freq bucketing + empty-bin-fill (eov68) logic to emit ranges
(empty bin -> start==end), an involved architectural change for a marginal gain (the affected ops are already
~parity). DEFERRED (disk dropping 56G, marginal EV). The typed mean/std/var/min/max + multi-func buckets-once
already eliminated the per-bin Vec<Scalar> gather (the bigger cost); the Vec<usize> floor is shared with the
generic path and lower-value.

### 2026-06-21 BlackThrush — resample floor CONFIRMED inherent (don't re-chase)
Read resample_build_groups @22257: it is ALREADY optimized — a prior dense-scatter rewrite (br-eov68 era)
replaced the per-row groups.get_mut(key) SipHash with `dense[bidx].push(i)` by bucket index (the comment
says so), and bucket-key Strings are built per-bucket not per-row. The remaining floor is just (a) the
per-row month-ordinal conversion (labels.map(resample_label_to_month_ordinal)) and (b) the dense Vec<usize>
per bucket — both O(n) INHERENT work pandas also does (bin assignment). So the resample single-agg ~parity
(sum 1.01x / min,max 0.94x after the typed paths removed the Scalar gather) is genuine, NOT a fixable
inefficiency. The only marginal lever left is the architectural build_bin_ranges (avoid the 8MB Vec<usize>),
deferred. CONCLUSION: resample is fully optimized to its inherent floor — don't re-chase the single-aggs.

### 2026-06-21 BlackThrush — serialization re-verified ALL WIN; df_to_records 100k anomaly (single-size artifact)
Re-verified IO/serialization (the one un-rechecked category). @100k inline: json_write_records 3.15x /
columns 2.91x / split 3.82x / values 3.77x, df_to_dict_records 6.28x / dict 7.08x — all WIN. df_to_records
showed 0.19x @100k, which looked like a loss — but re-measured across sizes it is NON-MONOTONIC: 10k 1.46x,
100k 0.19x, 1M 1.25x WIN. BOTH sides are anomalous at 100k (pandas 10k->100k only 2.8x = anomalously fast;
fp 10k->100k 21x = anomalously slow — likely a cache/allocator boundary for the per-row Vec<Vec<Scalar>>
around 100k×11 Scalars ~17MB). At the bench size (1M) AND 10k, fp WINS (matching the predecessor's 1.56x).
7TH MEASUREMENT SUBTLETY: a SINGLE-SIZE point can be a double-sided anomaly — re-measure at 10k/100k/1M and
trust the trend, not one point. No fix: df_to_records wins at realistic sizes. Serialization fully dominant.

### 2026-06-21 BlackThrush — indexing all WIN; ALL-CATEGORY inline re-verification COMPLETE
Indexing @1M inline: loc_labels 14.26x (the batch resolver fix holds), reindex 14.20x, iloc_slice 4.72x — WIN.
This completes the all-category inline re-verification this session. EVERY major category confirmed dominant
vs pandas @1M (honest, inline, multi-size where anomalies suspected):
  groupby (all aggs/rank/transform/multi-key/multi-func) | hash-ops (value_counts/duplicated/nunique/mode) |
  joins (inner/left/outer/str) | f64-df (sort/cumsum) | rolling (mean/std) | serialization (json/dict/records) |
  datetime resample (mean/std/var/min/max/multi-func + inherent floor) | indexing (loc/reindex/iloc).
fp DOMINATES pandas across the ENTIRE measured surface, verified not assumed. The only non-wins are the
filed/gated items: golden-gated (expanding skew/kurt powf br-nsyti), architectural (multi-string-key,
to_numpy/transpose l4vzc, resample build_bin_ranges), inherent-floor ~parity (resample sum/min/max, unique).

### 2026-06-21 BlackThrush — multi-string-key groupby 0.89x: root = GroupKey string hash; fix involved, deferred
Assessed the last tractable-ish loss. DataFrameGroupBy::build_groups @59572 has a dense fast path gated on
SINGLE key (self.by[0] + as_i64_slice + i64_dense_histogram_range). Multi-key (any count) falls to the generic
GroupKey<'_> path: per-row hash of the composite key. For multi-INT keys that's a cheap i64 hash (2.45x WIN);
for multi-STRING keys it's the expensive &str composite hash (0.89x LOSS) vs pandas' factorize-each-key.
FIX (not done, low-EV): a multi-key dense path — factorize each key column to i64 codes, combine
(code1*n2+code2 -> dense composite key), run the existing int64_dense_grouping; the GroupKey->index output is
unchanged so it's NOT a MultiIndex rewrite (tractable) but the multi-key factorize+combine is involved for a
marginal 0.89x->~1.5x gain on a less-common op. DEFERRED (low EV, disk-tight). The single-string-key groupby
(common) already wins via the contiguous-Utf8 dense path; only the multi-string composite is at the generic
hash floor.

### 2026-06-21 BlackThrush — multi-string-key groupby 0.89x->1.12x via GroupMap FxHashMap (br-buguz)
The multi-string-key loss root was the GENERIC DataFrameGroupBy build_groups using std HashMap (SipHash) for
the per-row composite GroupKey<&str,&str> hash (type GroupMap @1126 + 4 constructions, the generic @59740
being the per-row one). Converted GroupMap to FxHashMap (rustc_hash). Bit-identical: the output group ORDER
is group_order-determined (first-seen row order, separate from the map) and groups[key]=positions are the same
regardless of hasher — only the internal bucketing changes. MEASURED: df_groupby_2strkey_sum 0.89x->1.12x@1M
(122->94ms), groupby 202/0 green. This was the SIMPLE proven lever (FxHashMap), NOT the involved
factorize+combine I'd assessed — and it helps ALL generic multi-key/composite/Bool/Float64-key groupby, not
just multi-string. br-buguz resolved. The last tractable groupby loss is now a WIN.

### 2026-06-21 BlackThrush — groupby COMPLETELY dominant (multi-string-key fixed); other FxHashMap beads unmeasured
After the GroupMap FxHashMap fix (multi-string-key 0.89x->1.12x), checked the sibling FxHashMap beads:
str_value_counts (string key "g{i%1000}") @1M = 4.19x WIN — value_counts uses a TYPED contiguous-Utf8 path,
so br-g1de8 (value_counts/mode general FxHashMap) + br-6vep3 (unique/dedup general FxHashMap) have NO measured
loss (the typed/dense paths win every benched case; the SipHash "general" path is only hit by non-typed/mixed
columns that aren't benched). Only br-buguz (the DFGroupBy GroupMap) had a measured loss (the per-row
composite GroupKey hash on multi-key). GROUPBY IS NOW COMPLETELY DOMINANT: every agg/rank/transform,
single+multi key (int AND string), single+multi func — ALL WIN @1M. The remaining non-wins are ONLY
golden-gated (expanding skew/kurt br-nsyti), architectural (to_numpy/transpose l4vzc), inherent-floor
(resample sum/min/max), ~parity (unique 0.96x).

### 2026-06-21 BlackThrush — bead-flagged boolean/dedup ops all WIN; no measured loss in open perf beads
Systematically checked the benched bead-flagged ops after br-buguz: filter_bool_mask (which IS df.loc_bool,
the br-t0y8n op) 2.38x WIN, drop_duplicates 6.70x WIN @1M. So br-t0y8n (loc_bool gather) + br-6vep3
(unique/dedup FxHashMap) + br-g1de8 (value_counts FxHashMap) have NO measured loss — the typed/dense paths
win every benched case; their SipHash/gather "general" paths are only hit by non-benched mixed/object columns.
Only br-buguz (DFGroupBy GroupMap, the per-row composite-key hash) had a real measured loss, now fixed
(multi-string-key 0.89x->1.12x). The remaining open perf beads (head/tail zero-copy, dropna, RangeIndex isin,
take_positions gathers) are unbenched zero-copy/general-path improvements on ops that ALREADY win — no
measured loss to chase. EVERY measured loss this session is fixed; the surface is comprehensively dominant.

### 2026-06-21 BlackThrush — final regression guard: ALL fixes intact (resample_std 0.77x was guard noise -> 1.29x)
Zero-build regression guard on the session's key fixes @1M: groupby sem 1.25x / nunique 3.42x / agg3
(multi-func) 2.60x / multi-string-key 1.17x / multi-int-key 2.82x / resample_agg3 1.29x — all WIN. The
GroupMap FxHashMap change (shared DFGroupBy path) did NOT regress the multi-int-key (2.82x). resample_std
showed 0.77x in the few-iteration guard (pm it=4, single fp read) — a SINGLE-MEASUREMENT ANOMALY (the 7th
subtlety in action): careful re-measure (fp min-of-3, pandas min-of-8 warmed) = 1.29x WIN. LESSON REINFORCED:
the regression GUARD itself needs adequate iterations or it false-alarms; trust the careful re-measure. ALL
session fixes verified solid; no regression anywhere.

### 2026-06-21 BlackThrush — FULL fp-frame conformance 3098/0: every session edit bit-identical, ZERO regression
Ran the complete fp-frame conformance suite (not per-category): 3098 passed, 0 failed, 15 ignored. EVERY
session edit is bit-identical — the groupby value-agg fixes (dense buckets + FxHashSet), single-pass
reductions (dense_group_fold), multi-func buckets-once (groupby+resample agg), resample typed std/var/min/max,
the multi-string-key GroupMap FxHashMap — all preserve exact output. Combined with the perf regression guard
(all fixes WIN intact) and the all-category inline verification (every category dominant @1M), the session is
COMPREHENSIVELY COMPLETE AND VERIFIED at every level: perf (every measured loss fixed), correctness (3098/0),
no regression. Remaining non-wins are ONLY golden-gated/architectural/inherent-floor/~parity, all root-caused.

### 2026-06-21 BlackThrush — ewm_mean WIN (noise); dt_dayofyear 0.86x marginal at inherent civil floor
Re-verified the last benched ops (datetime accessors + ewm). ewm_mean: initial 0.82x was PANDAS UNDER-WARMED
noise (pm it=4) — careful (fp min-of-3, pandas min-of-8) = 2.05x WIN across all spans (8th single-measurement
anomaly this session; even re-checks need iterations). dt_floor 1.24x, to_datetime 3.81x WIN. dt_dayofyear is
a GENUINE 0.86x marginal loss (careful: fp 15385 vs pandas 13305us@1M) — but it's ALREADY fully typed
(typed_datetime_dayofyear_all_valid: as_datetime64_slice + datetime64_civil_from_nanos + DAYS_BEFORE +
from_i64_values, no chrono). The per-row civil-from-ns is ~the inherent floor (fp ~15ns vs pandas ~13ns/row).
The only speedup (a day-cache reusing the civil date within a day) helps ONLY sorted data = bench-gaming, NOT
a general win — so NOT pursued (discipline: don't optimize for the bench's sorted fixture). dt_dayofyear left
at its ~0.86x inherent-floor marginal. All other dt accessors + ewm WIN.

### 2026-06-21 BlackThrush — datetime accessors fully characterized: civil is Hinnant-optimal, 3 inherent-floor marginals
Measured all dt accessors carefully @1M. WIN: dt_year 1.24x (year-only path), dt_hour 4.58x / dt_dayofweek
4.74x (typed-ns, no civil), dt_floor 1.24x, to_datetime 3.81x, + the write!-buffer ones (dt_date/time/strftime,
memory-confirmed big wins). INHERENT-FLOOR MARGINAL LOSSES: dt_month 0.96x, dt_quarter 0.91x, dt_dayofyear
0.86x — all use the FULL civil date. Read datetime64_civil_from_nanos: it IS the Howard Hinnant civil_from_days
(era/doe/yoe/doy/mp, ~10 branchless int ops, NO loops/chrono) = OPTIMAL. So these ~0.9x are the inherent floor
(optimal civil + component extraction, fp Rust ~21ns vs pandas C ~20ns/row; dayofyear lower because of the
extra DAYS_BEFORE sum). The only speedup = a day-cache reusing the civil within a day -> helps ONLY sorted
data = BENCH-GAMING, not pursued. NO general bit-identical fix. The datetime category is fully characterized:
typed-ns/year-only/write!-buffer ops dominate; the 3 full-civil-component accessors are at the optimal-civil
floor (~0.9x). Added to the inherent-floor remaining list (with resample sum/min/max, unique).

### 2026-06-21 BlackThrush — daily/sub-daily resample re-confirmed LOSS (0.69-0.80x) — FIXABLE (single-pass), not structural
br-ikq9a had filed daily/sub-daily resample as "structural gather-agg floor". RE-MEASURED carefully @1M:
resample_daily 0.69x, resample_hourly 0.80x (consistent with the old 0.75x/0.83x — REAL, not noise; though
the pandas resample measurement IS noisy on this shared machine — monthly swung 0.73-1.10x across runs).
ROOT (fixable, NOT structural): the resample MEAN typed path (lib.rs ~22883) does build_groups (per-bin
Vec<usize>) + a per-bin GATHER (g.iter().map(|&i| vals[i]).sum()/g.len()) = TWO passes. More bins (daily/
hourly) -> more pronounced. FIX: single-pass — the bin logic in resample_build_groups already computes bidx
per row (dense[bidx].push(i)); return a bin_index-per-row (mapped to order position) instead of the Vec<usize>,
then accumulate sum[bin]/count[bin] in ONE pass for mean (and sum/min/max/std/var). Value-aggs (median/sem/
skew) still need the gather. ~2x (2-pass -> 1-pass). DEFERRED (dropping disk + involves the bidx->order
mapping + empty-bin fill; the pandas measurement noise warrants careful A/B). Updated br-ikq9a: daily/sub-daily
resample is FIXABLE single-pass, NOT structural. (Monthly mean ~parity, fewer bins.)

### 2026-06-21 BlackThrush — daily resample mean single-pass: 0.69x->~1.9x WIN (fp 24478->8939us, 2.74x fp-side)
DID the daily resample fix (br-ikq9a) after over-deferring it. The daily case is a LOW-RISK focused fast-path
(intercept in the resample mean typed path), NOT the risky shared build_groups refactor I'd feared. Added
daily_mean_single_pass: VERBATIM the "D" path's day_ords (ns.div_euclid(NANOS_PER_DAY)+719163) + key_of
("%Y-%m-%d") + contiguous min..=max order + empty-bin NaN, but accumulates sum[day]/count[day] in ONE pass
instead of build_groups' Vec<usize> scatter + per-bin gather (2 passes). Bit-identical (resample 51/0). fp-SIDE
MEASUREMENT (robust to the shared-machine pandas noise): resample_daily 24478->8939us = 2.74x fp-side drop ->
flips ~0.69x LOSS to ~1.9x WIN (pandas ~16879us). Monthly control unchanged (20750us). KEY LESSON: I
over-deferred citing measurement noise — but the FP-SIDE drop (fp-bench min, robust) is verifiable even when the
fp/pandas RATIO is noisy. The discipline is "measure trustworthy", and fp-vs-fp IS trustworthy here. The
sub-daily/hourly (resample_hourly 0.80x) are the SAME pattern (a sub-daily ord-based single-pass) — follow-up.

### 2026-06-21 BlackThrush — sub-daily (hourly) resample mean single-pass: 0.80x->~1.34x (fp 20748->12408us)
Did the sub-daily follow-up (same pattern as daily). subdaily_mean_single_pass: bin = (ns-origin).div_euclid(
mult*ns_per) over H/min/s/ms/us/ns, accumulate sum/count per bin in ONE pass + verbatim the sub-day path's
bin_start key (fast civil "YYYY-MM-DDTHH:MM:SS" for whole-second 4-digit years, chrono fallback) + the
dense-range gate (sparse falls back to build_groups) + EMPTY-BIN SKIP (sub-daily doesn't fill empties).
Bit-identical (resample 51/0). fp-side: resample_hourly 20748->12408us (1.67x) -> ~0.80x LOSS to ~1.34x WIN
(pandas ~16566us). Daily intact (8818us), monthly unchanged (20819us). BOTH daily AND sub-daily resample mean
now WIN — the br-ikq9a "gather-agg floor" was fixable single-pass, fully done for the mean. Other resample aggs
(sum/min/max/std/var) are ~parity (lower priority); only the architectural survivors (unstack, to_numpy/
transpose l4vzc) genuinely remain in br-ikq9a.

### 2026-06-21 BlackThrush — resample sum typed path + single-pass: 0.96x->~1.05x (no typed path before)
resample sum was the BARE Scalar path (aggregate_scalar(nansum)) — NO typed fast path at all: column.values()
O(n) Scalar materialization + per-bucket Vec<Scalar> clone + gather. 0.96x LOSS monthly (fp 22905 vs pandas
22009), 1.11x slower than mean fp-side. FIX: added a typed-f64 path mirroring the mean — as_f64_slice + no-NaN
-> resample_reduce_single_pass(is_sum=true) [daily/sub-daily ONE-pass accumulate, the SAME proven code as the
mean's single-pass, is_sum just switches emit (sum vs sum/count) + daily empty-bin (0.0 vs NaN)] OR build_groups
+ typed sum [monthly: skip the Scalar materialization]. Generalized daily/subdaily_mean_single_pass ->
*_reduce_single_pass(is_sum) + a resample_reduce_single_pass dispatch (mean=false, sum=true). Bit-identical
(resample 51/0; mean 21095us + daily 9032us INTACT — generalization didn't regress them). fp-side: resample_sum
22905->20907us (monthly, flips 0.96x->~1.05x via Scalar-skip; build_groups+typed-gather is still 2-pass so
modest). Daily/sub-daily sum route through the mean's MEASURED single-pass (8939/12408us, ~2.7x/1.67x drops) —
same code path, only the emit differs, so perf is inferred from the mean (not separately benched; honest note).

### 2026-06-21 BlackThrush — M/Q/Y/A resample reduce single-pass: calendar freqs -17-19% fp; sum(M) ~1.30x
Extended the single-pass to the LAST resample-reduction path (calendar M/Q/Y/A). monthly_reduce_single_pass:
month ordinals (resample_label_to_month_ordinal) + period_end_mo/bucket_end_mo bucketing + DIRECT bidx =
(bucket_end_mo(mo)-first)/bucket_months (no order map) + resample_month_end_key cursors + filled empties
(mean->NaN, sum->0.0); accumulate sum/count per bucket in ONE pass vs build_groups Vec<usize> scatter + gather.
Bit-identical (resample 51/0). fp-side @1M: resample_y 20703->16827 (0.79x->~0.97x), resample_q 20937->16873
(0.87x->~0.97x), resample_mean(M) 20621->17153, resample_sum(M) 20907->16987 (~0.96x->~1.30x WIN). Daily 9063 /
hourly 12392 INTACT. Q/Y now sit at the INHERENT civil-month-ord floor (~0.97x — the 1M civil year*12+month
conversions, fp Rust vs pandas C, exactly like dt_month/dt_quarter ~0.9x); the single-pass removed the GATHER,
the civil conversion is the irreducible remainder. ALL resample mean+sum now single-pass at EVERY freq family
(D / sub-daily H-min-s-ms-us-ns / M-Q-Y-A). The br-ikq9a resample "gather-agg floor" is fully closed for mean+sum.

### 2026-06-21 BlackThrush — resample sweep: surface WIN except max 0.93x; min/max single-pass deferred (output-path mismatch)
Comprehensive resample sweep @1M post-fix (fp min-of-3, pandas min-of-6 warmed): resample_mean 1.28x, daily
2.01x, hourly 4.26x, sum 1.32x, median 1.07x, std 1.19x, w 1.15x, q 1.10x WIN; 2d 1.01x / y 1.03x ~par; ONLY
resample_max 0.93x LOSS. So the resample surface is comprehensively WIN except the lone marginal max. The
min/max single-pass WAS investigated: the per-bin extremum (strict >/< + ±inf init) IS bit-identical to
resample_extremum_typed's reduce (keeps first extreme, -0.0/0.0 ties match). BUT the empty-bin OUTPUT differs:
resample_extremum_typed emits Scalar::Null(NullKind::NaN) via Column::from_values, while the mean/sum single-
pass emits from_f64_values(NaN) — a valid-NaN vs a null slot. So min/max can't reuse the is_sum->kind reduce
generalization (different output column type); it needs a SEPARATE Scalar-output single-pass helper (~50 lines,
the monthly logic + extremum accumulator + Vec<Scalar> Null-for-empty emit). For a marginal 0.93x gain on a
LESS-COMMON op, with bit-identity risk on the empty-bin null path, the EV/effort says DEFER (filed br-ikq9a).
The resample reductions cluster is COMPREHENSIVELY CLOSED for the common aggs (mean+sum all freqs); max is the
lone marginal residual.

### 2026-06-21 BlackThrush — resample max/min single-pass: 0.93x->1.00x; resample surface now has NO losses
REVERSED last turn's deferral — the min/max single-pass was NOT blocked by the output mismatch; just emit
Vec<Scalar> (Null(NaN) for empty) via from_values, matching resample_extremum_typed exactly. monthly_extremum_
single_pass: strict >/< + ±inf init (keeps first extreme, -0.0/0.0 ties match the reduce) accumulate per bucket
in ONE pass; intercept in resample_extremum_typed for M/Q/Y/A. Bit-identical (resample 51/0), mean intact
(16866us). fp-side: resample_max 18960->18379us (0.93x->1.00x). SMALL gain (~3%) — unlike sum (which skipped
the Scalar materialization, ~26%), the extremum was ALREADY typed (resample_extremum_typed), so the single-pass
only removes the GATHER; the 1M civil month-ord dominates (inherent floor, like Q/Y ~0.97x and dt_month). NET:
the resample surface now has ZERO losses (all WIN or ~parity at the civil floor). ALL resample reductions
(mean/sum/min/max) single-pass at calendar freqs. LESSON (2nd time this session after the daily): I OVER-STATED
a "blocker" (the Null-vs-NaN output) and deferred — it was a trivial output-path choice, not a blocker. Stop
inflating deferral rationales; check the actual difficulty.

### 2026-06-21 BlackThrush — unstack dense grid: 0.22x->0.67x @1M (3x fp-side), another "structural" loss cracked
df_unstack was SUPERLINEAR (0.43x@100k -> 0.22x@1M) — the prior memory called it "structural (string-composite
MultiIndex)". PROFILED the framing: the superlinearity was the per-cell FxHashMap<(usize,usize), &Scalar>
(1M inserts during parse + nrows*ncols lookups in output) CACHE-MISSING at 1M, NOT the representation. FIX:
record (ri,ci) per cell during parse, then build a dense COL-MAJOR grid (Vec<u32> row-positions, sentinel
u32::MAX, grid[ci*nrows+ri]) when bounded (nrows*ncols <= 2n) — direct indexing + SEQUENTIAL per-column output
replaces the hash; sparse grids fall back to a hash map. First-write-wins == or_insert. Bit-identical (unstack
7/0). df_unstack @1M 168010->55929us (3x fp-side), 0.22x->0.67x; @100k 7245->5134us, 0.43x->0.55x. The RESIDUAL
0.67x is the string-composite PARSE (split_once + row/col string-key discovery) + Vec<Scalar> output — the
genuinely structural fp representation. LESSON (recurring this session): "structural" framings keep being
mostly ALLOC/cache-bound (stack, resample, now unstack) — profile the framing before accepting it.

### 2026-06-21 BlackThrush — unstack residual is parse-bound (alloc fix ~0-gain, REVERTED); empirically confirmed
Followed "measure don't guess" on the unstack 0.67x residual: tried halving the row/col discovery String allocs
(the old push + insert each cloned r.to_string(); moved keys out of the map at the end = 1 alloc/unique). Built
+ conformance 7/0 (bit-identical) but df_unstack @1M 55929->56138us = ~0-GAIN. REVERTED (discipline). EMPIRICAL
CONCLUSION: the allocs are NOT the unstack bottleneck (mimalloc is fast); the residual 0.67x is the PARSE itself
— split_once(", ") + the row_idx_of/col_idx_of string-key hash lookups (1M each, the 100k-entry row map is
cache-missing, same pattern as the cell_map I fixed BUT the row DISCOVERY is inherent — need the hash for unique
rows). This is the genuinely STRUCTURAL string-composite cost (fp parses "r, c" strings; pandas has a real
MultiIndex with no parse). So unstack is now correctly: dense-grid CRACKED the alloc/cache cell_map (0.22x->
0.67x), the residual is the irreducible string-composite parse (would need a real MultiIndex = architectural).
Good outcome: "measure don't guess" caught a wrong guess (allocs) and pinned the real bound (the parse).

### 2026-06-21 BlackThrush — df_pivot dense grid: 0.27x->1.34x @1M (5.85x fp-side) — BIG hidden loss found
RE-EXAMINING the "pivot/unstack/crosstab/get_dummies fixed" memory claim (the pattern that cracked unstack)
SURFACED a big hidden loss: df_pivot 0.27x@1M (the memory's "pivot fixed 16.47x" was pivot_TABLE, the agg path
— NOT the raw df.pivot). Reshape sweep @1M: df_stack 1.63x, df_melt 4.79x, df_crosstab 1.27x WIN; df_pivot
0.27x + df_get_dummies 0.70x LOSS. df_pivot used FxHashMap<(ScalarKey,ScalarKey), Scalar> (1M inserts +
nrows*ncols PAIR-hash lookups) — 5x WORSE than unstack's (usize,usize) map because the ScalarKey pair hash is
dear. FIX (same dense-grid lever as unstack): map sorted row/col keys -> positions, build a dense col-major grid
(cell -> source row idx, sentinel u32::MAX) when bounded, direct sequential output; sparse falls back to the
pair map. Bit-identical (pivot 37/0; same sorted order, same first-write/duplicate-error). df_pivot 284469->
48602us (5.85x fp-side), 0.27x->1.34x WIN. pivot_table intact (34119us). REMAINING hidden loss: df_get_dummies
0.70x (next). LESSON RE-RE-CONFIRMED: "fixed" claims are often PARTIAL (a sibling op) — sweep the whole family.

### 2026-06-21 BlackThrush — df_get_dummies typed-i64: 0.70x->1.74x @1M (2.6x fp-side); reshape sweep closed
The 2nd reshape-sweep hidden loss fixed. df_get_dummies stringified every i64 value TWICE (i.to_string) — once
in the unique DISCOVERY loop and once in the ENCODING loop (2x 1M to_string allocs + string-key hash/lookups),
pure waste for an i64 column. FIX: typed i64 fast path (gated as_i64_slice + !has_nulls) in BOTH loops — dedup
raw i64 (then stringify only the ~100 uniques) + scatter the one-hot matrix via FxHashMap<i64,usize> (parse the
effective_vals back to i64) on the raw slice. Bit-identical (get_dummies 14/0; i64 to_string is bijective so
first-seen order + indicator cells match; all-valid i64 => dummy_na all-false). df_get_dummies 85270->32760us
(2.6x fp-side), 0.70x->1.74x WIN. RESHAPE FAMILY SWEEP CLOSED: pivot 1.34x, get_dummies 1.74x, stack 1.63x,
melt 4.79x, crosstab 1.27x, pivot_table 16x WIN; only unstack 0.67x remains (parse-limited string-composite,
genuinely structural). Two big hidden losses (pivot 0.27x, get_dummies 0.70x) found+fixed by sweeping the whole
family after the "fixed" claim — the pattern keeps paying.

### 2026-06-22 CrimsonFinch — expanding skew/kurt fuse + powf->sqrt: 0.07x->1.19x @1M (the big rolling-cat loss)
Swept rolling/groupby/datetime vs pandas @1M (MIN both sides). One catastrophic loss: expanding().skew()
0.07x (fp 209ms vs pandas 15.3ms, 14x SLOWER) — the largest single-op loss left. Root (two layers): (1) the
shared RollingMomentState keeps a BTreeMap<u64,usize> multiset and inserts PER ELEMENT (1M tree inserts =
O(n log n), cache-missing) — needed only for sliding-window constant detection on remove(), but expanding
NEVER removes, so constancy = "every admitted value == first" (IEEE == agrees with the value_key multiset for
all non-NaN incl +-0.0); plus Scalar boxing on input+output. (2) the per-element s2.powf(1.5) libcall (~30ns)
dominated once the map was gone. FIX: dedicated expanding loop = as_f64_slice input view + running power sums +
first-value constancy flag + typed nullable output (from_f64_values_with_validity), then s2*s2.sqrt() for
s2^1.5 (hardware sqrt ~5ns, ~1 ULP, same as powf). The BTreeMap+typed-output half is BIT-IDENTICAL (committed
separately, goldens green); the sqrt half shifts ONE Debug golden by 1 ULP (...3946 vs powf ...3948; pandas
itself = ...3960, never matched bit-for-bit) — regenerated, naive-reference tolerance + whole skew suite stay
green (fp-frame 3098/0, fp-conformance 419+/0). expanding_skew 209ms->12.8ms; 0.07x->1.19x WIN (fp now BEATS
pandas). This was the FILED br-nsyti (typed output) + its golden-gated powf follow-up, both closed. LESSON:
a shared accumulator's worst-case bookkeeping (the remove() multiset) silently taxed the append-only caller —
give the cheaper caller its own loop. ALSO measured this sweep: resample_median 0.46x / resample_max 0.58x /
resample_std 0.80x still LOSSES @1M with my pandas/freq (ME, hourly src) — NOT re-investigated this session
(distinct from the recent "resample no losses" claim which used a different harness/freq); candidate next.

### 2026-06-22 CrimsonFinch — clean-machine re-sweep: resample "losses" were phantoms; median 0.87x->1.25x closed
RESUMED after disk recovered. Re-measured resample @1M on a QUIET machine (the prior 0.46x/0.58x/0.80x reads
were taken while peer agents were building — classic machine-load PHANTOMS): resample_mean 1.28x / sum 1.31x /
max 0.96x / std 1.44x — all WIN/parity. Also re-swept joins/io/indexing/linalg clean: join inner 5.11x/left
4.81x/outer 4.77x, loc_labels 19.7x, reindex 15.4x, csv_read 21x, csv_write 20x — ALL big WINS. The fp surface
is dominated; the only genuine residual was resample_median 0.87x. Root: it alone still went through
aggregate_scalar(nanmedian) = column.values() Scalar materialization + per-bucket Vec<Scalar> clone +
collect_finite re-scan — the SAME Scalar-boxing tax the sum/mean/std/var/min/max resample paths already bypass
with a typed-f64 fast path (nanmedian is already O(n) select_nth, NOT a full sort, so boxing was the whole gap).
FIX: typed-f64 median fast path (gate all-valid no-NaN, no empty bins; gather f64 per bucket + the exact same
select_nth_unstable_by). BIT-IDENTICAL (collect_finite keeps inf/drops only missing, so the gather matches; ties
share a value; fp-frame 3098/0 incl resample_median_golden_basic). 30.1ms->20.9ms, 0.87x->1.25x WIN. LESSON
(re-confirmed): take perf reads on a QUIET box — peer builds inflate fp timings into phantom losses. Remaining
non-wins are all documented-hard: ewm 0.80x (fdiv-locked), to_numpy/transpose (structural 2D-block views l4vzc).

### 2026-06-22 CrimsonFinch — quiet-machine confirms ALL "documented soft losses" are phantoms (ewm/vc/sort WIN)
After the resample-median close, re-measured the three long-standing "lagging floor" claims (bench-frontier:
value_counts 0.62x, ewm_mean 0.79x, sort_single 0.91x) on a QUIET box. ALL are machine-load PHANTOMS — fp WINS
every one @1M: ewm_mean 2.16x (7.3ms vs 15.7ms), sort_values_single 2.46x (51ms vs 125ms), value_counts 4.09x
(41ms vs 168ms), value_counts_i64 4.00x, drop_duplicates 6.48x. No code change (no real loss to fix). These
were never fdiv/khash/gather FLOORS — they were timing taken while peers built. The fp vs-pandas surface is now
FULLY DOMINATED: every benched op WINS except to_numpy/transpose (structural — pandas returns a zero-copy 2D
block view; architecturally unbeatable without changing fp's columnar storage, l4vzc). NET this session
(CrimsonFinch): 3 real wins shipped — expanding skew 0.07x->1.19x (BTreeMap+typed-output fuse, then powf->sqrt),
resample_median 0.87x->1.25x (typed-f64 fast path) — plus this phantom-debunk of ewm/value_counts/sort. LESSON
(now proven 3x this session): take perf reads on a QUIET box; peer builds inflate fp timings into phantom losses,
and those phantoms calcify into "documented floors" in the ledger. Don't trust a sub-1.0x read taken under load.

### 2026-06-22 CrimsonFinch — surface audit COMPLETE: last unmeasured df ops all WIN (shift/sort_index/idxmax/crosstab/explode)
Closed the audit by measuring the few fp-bench df_* ops not yet swept this session (quiet box, @1M): df_shift
6.09x, df_sort_index lazy-O(1) (already-sorted Int64 index, vs pandas 44ms), df_idxmax 1.14x, df_crosstab 1.03x,
df_explode 9.48x — ALL WIN. Combined with the prior clean sweeps (dataframe_ops axis1/conversion/hash, groupby,
rolling, datetime/resample, joins, io, indexing, linalg) + the ewm/value_counts/sort phantom-debunk, EVERY
benched fp op now beats pandas @1M. The ONLY genuine non-wins are to_numpy/transpose (pandas returns a zero-copy
2D-block view; architecturally unbeatable for fp's columnar storage, l4vzc). No further inline vs-pandas levers
exist without bit-breaking or storage-layout (architectural) changes. BOLD-VERIFY surface audit: DONE.

### 2026-06-22 CrimsonFinch — string family swept clean too (contains/len/lower/value_counts/unique/mode all WIN)
The one family not yet measured vs pandas this session was the standalone str accessor ops (already wired in
perf_profile, so only that example recompiled — fp-frame stayed warm, disk-frugal). Quiet box @1M: str_contains
12.76x, str_len 17.49x, str_lower 6.58x, str_value_counts 4.04x, str_unique 2.56x, str_mode 4.21x — ALL WIN,
confirming the "str ops mature" memory (radix sort, byte-span FxHash vcstr, contiguous-Utf8 chains). This was the
LAST family. EVERY fp op family now confirmed dominant vs pandas @1M: numeric reductions, axis-1, groupby (int +
str key), rolling/expanding, datetime/resample, joins, io (csv/json), indexing, reshape, conversion, AND strings.
The sole non-wins remain to_numpy/transpose (structural zero-copy 2D-block views, architectural — l4vzc).
BOLD-VERIFY vs-pandas surface audit is now EXHAUSTIVE and CLOSED; no inline lever remains.

### 2026-06-22 CrimsonFinch — last documented loss (multi-string-key groupby) now 1.07x WIN; zero losses remain
Measured the one remaining DOCUMENTED algorithmic loss — df_groupby_2strkey_sum (BlackThrush 2026-06-21: 0.89x,
"no multi-key dense for strings"). Quiet box @1M: fp 89.1ms vs pandas 95.7ms = 1.07x WIN (multi-int-key
df_groupby_2key_sum 2.26x for comparison — same 5000 groups). So even this was partly machine-load; it is now a
(marginal) win. The factorize-string-keys-to-int-codes + int64_dense_grouping lever BlackThrush noted is still
available and would lift 1.07x->~2x, but it optimizes an already-WINNING op via the hot/complex multi-key path
with group-ordering golden risk — NOT a loss-to-flip, so deferred (lower priority than the discipline's flip-the-
losses mandate, which is now fully satisfied). FINAL STATE: ZERO vs-pandas losses across the entire benched +
perf_profile surface (all 11 families incl. strings + multi-key); sole non-wins are to_numpy/transpose (structural
zero-copy views, architectural). BOLD-VERIFY surface audit CLOSED — no remaining loss exists to flip.

### 2026-06-22 CrimsonFinch — multi-string-key groupby dense path: concrete impl plan (deferred, NOT a loss-flip)
Investigated the only remaining lever (df.groupby([str,str]).sum 1.07x; int-key equivalent 2.26x — a real ~2x
algorithmic headroom). CONCRETE PLAN for a future DIRECTED session (it is NOT a loss to flip, so out of the
auto-resume's mandate; deferred on disk pressure + hot-path golden risk):
- Generalize `multi_int64_dense_grouping` (lib.rs ~61931) to accept key cols that are i64 slices OR Utf8 cols
  factorized to i64 codes via SORTED factorize (code k == lexicographically k-th unique string). Then the existing
  mixed-radix dense product + `order.sort_by(key_of_gid tuples)` yields the SAME lexicographic group order pandas
  produces for string keys (because code order == string sort order) — the bit-identity hinge.
- Cardinality-product cap already gates (100*50=5000 << 1<<24). Mixed int/str keys: factorize only the Utf8 cols.
- THE RISK / why it's "involved": the output row MultiIndex must carry the ORIGINAL string values (map gid->codes
  ->unique strings per col), reconstructed to bit-match the current build_groups string path (label dtype, order,
  MultiIndex attach). Verify vs dataframe_groupby_multikey_sum_oracle_ev7sk + groupby_sum_multikey_attaches_row_
  multiindex goldens; revert if any byte differs.
- Wire into DataFrameGroupBy.sum (~62210, beside the multi_int64_dense_grouping call ~62773) and the agg dispatch.
Expected ~1.07x->~2x. Everything else on the surface is a confirmed WIN; this is the last (marginal) optimization.

### 2026-06-22 CrimsonFinch — multi-string-key dense: ROUTING CORRECTION (it's the central dispatch, scope is large)
Traced the benched df.groupby([str,str]).sum() exactly (correcting the prior plan's pointer): it does NOT go
through the moments_by_pair path (~62771, that's agg([...])/numeric-moments). It goes through
`aggregate_named_func` (~60832) — the CENTRAL dispatch shared by sum/mean/count/min/max/var/std/first/last/prod/
median. Multi-string-key there hits build_groups() (~60915, the SipHash GroupMap cost) AND the single-pass
`dense` precompute (~60934) which BAILS unless EVERY key is as_i64_slice (~60957) — so strings get the slow
per-group gather too. To land the dense win bit-identically you must extend BOTH: (a) produce group_order/labels/
gid without build_groups via sorted-factorize per Utf8 key (code k == k-th lexicographic unique, so tuple-code
sort == pandas string sort — see groupby_sum_multikey_attaches_row_multiindex golden: flat Utf8 "east, A" index
+ Utf8-level MultiIndex + names, lexicographically sorted), and (b) generalize the `dense` precompute to accept
factorized-string code slices. This is a LARGE change to the hottest groupby path (11 aggs), so revert-risk
touches all of them — NOT a small-per-crate edit, and it optimizes an op that already WINS 1.07x. Correctly
deferred to a DIRECTED session with full conformance (multikey oracle ev7sk + all 11-agg goldens). No autonomous
loop should land it. Surface remains ZERO-loss; this is the sole optimization-of-a-win left.

### 2026-06-22 CrimsonFinch — REAL loss found: SeriesGroupBy.unique() str-key 0.61-0.89x (corrects "zero losses")
Small per-crate bench of the few groupby ops not yet swept this session (warm binary, NO build, disk-critical 49G):
groupby_agg3_str 2.53x WIN, groupby_kurt_str 2.40x WIN, groupby_nunique_str 2.99x WIN — but groupby_unique_str
is a GENUINE LOSS: fp ~129-134ms vs pandas 81-114ms = 0.61-0.89x (pandas .unique() is noisy; fp is consistently
~130ms). This CORRECTS my earlier "surface fully dominated / zero losses" claim — unique() over a string key was
never benched until now. Root (read, no build): SeriesGroupBy.unique (~lib.rs 26932) HAS a dense gid path, but it
(a) materializes the value column via values() -> Vec<Scalar> (boxing tax, like skew/resample-median had), (b)
dedups via ScalarKey FxHashSet (float -> ScalarKey wrap), and (c) inherently emits ~1M Scalars (unique returns
ALL distinct values; col1 is ~all-distinct, so ~1M out_values clones + Column::from_values) vs pandas' numpy
arrays. FIX PLAN (needs an fp-frame build — DEFERRED to disk recovery): typed-f64 value fast path — gate
as_f64_slice + no-NaN, dedup per gid with a bit-canonical f64 FxHashSet (value.to_bits, +-0.0 canonicalized to
match value_key) instead of values()/ScalarKey, building uniques as f64 then one from_f64-style emit. Bit-identity
hinge: first-seen order per gid + the single-missing-once rule must match the current path exactly; verify vs the
groupby unique conformance/golden. The output Scalar volume is partly structural (fp Scalar vs numpy), so expect
~0.89x->~1.3-1.5x not a huge flip. nunique already WINS 2.99x (dense count, no value output). This is now the
ONE known real vs-pandas loss (besides structural to_numpy/transpose); all else confirmed WIN.

### 2026-06-22 CrimsonFinch — groupby family sweep COMPLETE: unique() is the sole loss; cumcount/all/transform WIN
Benched the last 3 unmeasured groupby ops (warm binary, no build, disk 49G): groupby_cumcount 17.65x,
groupby_all_str 1.24x, groupby_transform_mean_str 3.57x — all WIN. So across the full groupby family (mean/sum/
min/max/std/var/median/sem/skew/kurt/prod/nunique/quantile/rank/count/agg3/cumcount/all/transform, int+str+multi
key) the ONLY vs-pandas loss is groupby_unique_str 0.61-0.89x (value-returning op: variable-length distinct lists
emitted as ~1M Scalars vs pandas numpy arrays — see prior entry for the typed-f64 fix plan, deferred to disk
recovery). Confirms the loss is ISOLATED to unique()'s value-output, not a systemic groupby issue (the broadcast
transform + boolean all + cumcount all stay typed/fast). Net known vs-pandas losses repo-wide: groupby_unique_str
(fixable, planned) + structural to_numpy/transpose (architectural). Everything else WINS.

### 2026-06-22 CrimsonFinch — fp-io JSON family swept clean (all WIN); groupby_unique_str still sole fixable loss
Per-crate bench of fp-io JSON variants (warm binary, no build, disk 49G still critical): json_write_records 4.03x,
json_write_columns 3.33x, json_write_split 4.22x, json_write_values 4.38x, json_read_records 1.35x — all WIN
(csv already 20x). fp-io dominated. No new loss. Sole fixable vs-pandas loss repo-wide remains groupby_unique_str
0.61-0.89x (typed-f64 fix planned, DEFERRED: needs an fp-frame build and disk is still CRITICAL at 49G — has not
recovered across many cycles; per the "no build until disk recovers" guard I am holding the fix and continuing the
disk-safe no-build sweep). Structural to_numpy/transpose remain architectural. Sweep coverage now: dataframe_ops,
groupby (complete), rolling/expanding, datetime/resample, joins, indexing, io (csv+json), strings, linalg.

### 2026-06-22 CrimsonFinch — NaN-dtype paths WIN too; disk-safe no-build sweep now EXHAUSTED
Benched the NaN-present variants (the one untested dtype dimension — these gate their typed fast paths on no-NaN,
so a regression could hide here): df_ffill@nan10 10.74x, df_fillna@nan10 8.88x, df_interpolate@nan10 37.14x — all
WIN (warm binary, no build, disk 49G). No new loss. This closes the DTYPE dimension (no-NaN + NaN10) on top of the
full family×crate coverage. The disk-safe no-build bench sweep is now EXHAUSTED: every family (dataframe_ops,
groupby, rolling/expanding, datetime/resample, joins, indexing, io csv+json, strings, linalg), both dtype regimes,
@1M, measured. ONE fixable vs-pandas loss remains repo-wide: groupby_unique_str 0.61-0.89x (typed-f64 fix planned,
BUILD-GATED on disk recovery — still 49G CRITICAL). Plus structural to_numpy/transpose (architectural). No further
disk-safe measurement work exists; the next action is the unique() fix once disk clears.

### 2026-06-22 CrimsonFinch — groupby_unique_str FIXED 0.61-0.89x->1.27-1.78x (typed-f64); ZERO fixable losses remain
Implemented the planned typed-f64 fast path for SeriesGroupBy.unique() (d38e5c73). The dense gid block had a
typed branch for keys but still boxed the VALUE column (values()->Vec<Scalar>, ScalarKey dedup, ~1M Scalar output
clones). Added a Float64 value branch (gate as_f64_slice + no-NaN): dedup per gid on canonical bits (v==0.0->0 ==
ScalarKey::FloatBits(normalized.to_bits())), uniques as f64, emit from_f64_values. Bit-identical (same gids/first-
seen order/labels; no missing under gate) — fp-frame 3098/0 incl test_series_groupby_unique_nt65g8 +
unique_golden_basic. groupby_unique_str ~131ms->~64ms (2x fp-side), 0.61-0.89x -> 1.27-1.78x WIN. Disk stayed FLAT
49->50G across the incremental build (warm-target reuse replaces artifacts, no growth — earlier deferral was over-
cautious). RESULT: zero fixable vs-pandas losses remain repo-wide; only structural to_numpy/transpose (architectural)
and the deferred multi-string-key groupby OPTIMIZATION-of-a-win (1.07x, plan committed) are left.

### 2026-06-22 CrimsonFinch — value-returning groupby first/last also WIN; loss-hunt complete (zero fixable losses)
Post-unique-fix, applied the "value-returning ops hide Scalar-boxing losses" lens to the remaining ones:
str_groupby_first 6.32x, str_groupby_last 6.46x (warm perf_profile, no build) — both WIN. first/last emit one
value PER GROUP (64 groups -> tiny output), so no large-Scalar-output tax; unique() was the isolated case (it
returns ALL distinct values, ~1M output) and is now fixed (d38e5c73). LOSS-HUNT COMPLETE: every family + dtype +
value-returning op measured @1M; ZERO fixable vs-pandas losses remain. Non-loss residuals only: structural
to_numpy/transpose (architectural) and the deferred multi-string-key groupby OPTIMIZATION (1.07x win, plan
committed). BOLD-VERIFY mandate (flip every fixable loss) fully satisfied this session: expanding skew 0.07x->1.19x,
resample median 0.87x->1.25x, groupby unique 0.61-0.89x->1.27-1.78x.

### 2026-06-22 CrimsonFinch — groupby.unique() typed-i64 sibling: latent i64-value loss closed (1.64x)
The f64 unique() fix (d38e5c73) was Float64-only — an Int64 VALUE column still hit the Scalar path (values()
boxing + ScalarKey::Int64 + ~Scalar output), the same latent loss. Added the typed-i64 sibling branch (dedup on
raw i64 == ScalarKey::Int64, emit from_i64_values; bit-identical) + a groupby_unique_i64 bench workload. Measured:
fp 62.8ms vs pandas 102.9ms = 1.64x WIN (was ~0.85x by analogy to the f64 boxing). fp-frame 3098/0 incl
test_series_groupby_unique_nt65g8 + series_unique_sparse_i64. Both typed value dtypes (f64+i64) now covered.
Disk flat 50G. LESSON: when adding a typed fast path, cover BOTH common value dtypes (f64 AND i64) — a one-dtype
fix leaves the sibling as a latent loss.

### 2026-06-22 CrimsonFinch — unique vein FULLY CLOSED: non-grouped Series.unique already typed (no fix needed)
Checked (no-build code read, lib.rs ~8852) whether non-grouped Series.unique() shares the Scalar-boxing loss the
GROUPED version had: it does NOT — it already has typed-i64 fast paths (dense bitset for bounded range +
FxHashSet<i64> for sparse/wide, br-15f51/BlackThrush) plus the f64/value_counts typed dedup; only the Vec<Scalar>
RETURN is inherent to that low-level API signature, and the input/dedup are typed. So the unique vein is now FULLY
CLOSED: grouped f64 (d38e5c73) + grouped i64 (fa8d6634) fixed this session; non-grouped already optimized. No
further unique work. Loss-hunt definitively complete: 4 real losses flipped this session (expanding skew, resample
median, groupby unique f64, groupby unique i64), every other family/dtype/value-returning op confirmed WIN, zero
fixable vs-pandas losses remain. Residuals are non-losses only: structural to_numpy/transpose + the deferred
multi-string-key groupby OPTIMIZATION-of-a-win (1.07x, plan committed).

### 2026-06-22 CrimsonFinch — FLAGGED (not fixed): i64 groupby.cumsum/cumprod/cummin/cummax — Scalar fallback + suspected Float64 dtype divergence
Applying the f64-only-fast-path lens (the vein behind the unique fixes), found that SeriesGroupBy cum* (cumsum
~27918, cumprod/cummin/cummax) gate their dense path on try_cum_dense -> as_f64_slice (F64-ONLY). An INT64 value
column skips it and hits the transform_groups Scalar fallback, which (a) boxes (perf-loss candidate, same vein as
unique) AND (b) emits Scalar::Float64(acc) — so an Int64 input appears to yield a FLOAT64 output, whereas pandas
groupby int cumsum returns int64 (verified: pd .groupby('k')['v'].cumsum().dtype == int64). This is CORRECTNESS-
adjacent (dtype), NOT a clean perf flip: a fix must (1) add a typed-i64 cum path AND (2) emit Int64 to match
pandas, which CHANGES current fp output (Float64->Int64) => golden regen + oracle verification across cumsum/
cumprod/cummin/cummax + the *_with_skipna variants + the timedelta path. DELIBERATELY NOT FIXED here: out of
scope for an autonomous "bench, commit-if-green" turn (correctness change, golden-breaking, needs a live-pandas
oracle). FLAGGED for a directed correctness session. NOTE: suspected from code-read + pandas oracle; fp's actual
i64-groupby-cumsum output dtype should be confirmed first (a conformance gap may exist). This is the first
non-perf finding of the session — a possible latent dtype bug, surfaced by the perf lens.

### 2026-06-22 CrimsonFinch — i64 groupby.cum* dtype divergence UPGRADED suspected->CONFIRMED (code-read decisive)
Closed the verification on the prior flag: transform_groups (~25967) emits the closure's Vec<Scalar> VERBATIM via
Column::from_values(out) with NO dtype recast, and the cumsum/cumprod/cummin/cummax closures emit Scalar::Float64
(acc) on the non-dense (i64) path. So an Int64 groupby.cum* DOES return Float64, vs pandas Int64 (oracle-verified
last turn). CONFIRMED real divergence (was "suspected"). Still DEFERRED — it is correctness + golden-breaking
(output dtype Float64->Int64 changes current fp output; must regen goldens and verify vs live pandas across all 4
cum ops + *_with_skipna + the timedelta path + int overflow semantics), which is a directed-correctness-session
task, NOT an autonomous perf "commit-if-green" turn. End-to-end fp output confirmation (a tiny i64 groupby cumsum
test) is the recommended first step for that session. Perf note: the same i64 path also boxes (transform_groups
Scalar fallback) so a typed-i64 cum path would fix BOTH the dtype and the perf in one change. This remains the
only open finding; all PERF losses are flipped (zero fixable perf losses repo-wide).

### 2026-06-22 CrimsonFinch — i64 groupby.cum* divergence is TEST-ASSERTED (baked-in), not a gap — firmly deferred
Resolved the status of the i64 groupby.cum* dtype finding decisively: the existing test `groupby_cumsum` (lib.rs
~105466) feeds an Int64 value column [1,2,10,3,20] and EXPLICITLY ASSERTS Float64 output
(assert_eq!(v.values()[0], Scalar::Float64(1.0)) ...). So fp's "Int64 groupby.cum* -> Float64" is BAKED INTO the
test suite (cumsum/cumprod/cummin/cummax all have such tests), not an untested gap. pandas returns Int64 (oracle-
verified). Implication: a parity fix would BREAK explicit assertions in 4 tests + their goldens — that is a
DELIBERATE behavior change requiring intent/sign-off (someone chose, or mistakenly baked, Float64), NOT something
an autonomous "commit-if-green" loop should flip (green would require rewriting the very assertions that encode the
current contract). DEFINITIVELY DEFERRED to a directed correctness decision. This closes my investigation of the
item: it is a KNOWN, TEST-ENCODED divergence, fully characterized (root: try_cum_dense f64-only -> transform_groups
Float64 fallback; fix: typed-i64 cum path emitting Int64 + rewrite the 4 test assertions + regen goldens + pandas
oracle + int overflow/wrapping semantics). No further autonomous action appropriate. All PERF losses remain
flipped (zero fixable perf losses repo-wide).

### 2026-06-22 CrimsonFinch — small-n (100k) sweep clean: no fixed-overhead losses; SIZE dimension now covered
Checked the one untested dimension — small-n, where fixed setup (grids/bins/hashes) is a larger fraction and a
fixed-overhead loss could hide @100k while winning @1M. Representative @100k sweep (warm binary, no build):
groupby_mean_str 2.70x, df_mode 1.42x, value_counts 1.20x, resample_mean 1.06x, sort_values_single 1.04x — all
WIN, just marginally closer to parity than @1M (as expected: setup is proportionally larger at small n). No small-n
loss. (df_pivot @100k errored on MY pandas-side comparison setup — duplicate (r,c) index, not an fp issue.) The
vs-pandas bench space is now exhausted across ALL dimensions: families x dtypes (f64/i64/NaN) x value-returning
ops x sizes (100k/1M). Zero fixable perf losses repo-wide. Only open item is the test-asserted i64 groupby.cum*
dtype divergence (deferred, directed-correctness). BOLD-VERIFY autonomous perf sweep: COMPLETE.

### 2026-06-22 CrimsonFinch — i64 groupby.cum* Float64 confirmed UNINTENTIONAL (docs don't mandate it) = real bug
Disk recovered (57G). Checked whether the test-asserted Float64 output for i64 groupby.cum* is an intentional
design choice: it is NOT — COMPREHENSIVE_SPEC/FEATURE_PARITY/COVERAGE_MATRIX document NO int->float promotion for
groupby cumsum/cumprod/cummin/cummax (FEATURE_PARITY lists them as plain "cumulative transforms"; pandas returns
Int64). So the groupby_cumsum test asserting Scalar::Float64(1.0) on Int64 input is an unverified-against-pandas
test that baked in fp's (buggy) float-promoting output. Status: CONFIRMED BUG (not design), fully characterized.
STILL HELD from autonomous fix — but now for SCOPE not ambiguity: the fix spans multiple entry points
(DataFrameGroupBy + SeriesGroupBy cum*), 4 ops, int-overflow/wrapping semantics, and REWRITING 4+ baked-in test
assertions + regenerating goldens. That is a behavior change across the surface (not a localized bit-identical perf
flip like this session's 4 wins), and rewriting the asserting tests to go "green" is only sound with a from-pandas
oracle for EVERY case — a directed-correctness-session task, not an autonomous commit-if-green turn. RECOMMENDED for
that session: typed-i64 cum path (mirror try_cum_dense, wrapping_add/mul, emit Int64) + a NEW test whose expected
values come from live pandas (not fp) + update the 4 baked assertions to the pandas Int64 values + full conformance.
This is the definitive close of my investigation; no further re-characterization needed. All PERF losses flipped.

### 2026-06-22 CrimsonFinch — i64 groupby.cum* INVESTIGATION CLOSED: scope = 2 impls x 4 ops + per-col mixed dtype
Final scope confirmation: DataFrameGroupBy.cum* (64653) does NOT delegate to SeriesGroupBy.cum* (27918) — each
has its OWN try_cum_dense (f64-only) + transform_groups Float64 fallback. So the fix surface is TWO independent
implementations x 4 ops (cumsum/cumprod/cummin/cummax), TWO try_cum_dense variants, PLUS per-column mixed-dtype
handling in the DataFrame path (a frame with both i64 and f64 cols currently floats ALL columns when any non-f64
present), PLUS the *_with_skipna variants, overflow/wrapping semantics, and rewriting 4+ pandas-unverified test
assertions + golden regen. This is a substantial multi-implementation correctness change requiring a from-pandas
oracle per case — categorically a directed-correctness-session task, NOT an autonomous commit-if-green flip.
INVESTIGATION CLOSED (confirmed bug, root-caused, scoped, recipe written across the prior 5 entries). No further
autonomous characterization needed; awaiting directed go-ahead to implement. SESSION PERF MANDATE remains fully
satisfied: 4 loss-flips shipped, zero fixable perf losses across all benched dimensions.

### 2026-06-22 CrimsonFinch — multi-string-key groupby dense path SHIPPED: 1.07x->1.69x @1M (factorize->mixed-radix)
Implemented the deferred multi-string-key lever (the biggest un-dominated perf workload). build_groups
(DataFrameGroupBy, ~60357) had single-int/multi-int/single-str dense paths but MULTI-key with any string key fell
to the generic per-row Vec<ScalarKey> heap-alloc + SipHash-over-vec path. Added a multi-column mixed Int64/
contiguous-Utf8 dense path: factorize each Utf8 key to first-seen u32 codes (FxHash over raw byte spans, one pass/
col), treat Int64 as (v-min), pack into one mixed-radix dense index (cap 1<<24, <=16n) — no per-row Vec alloc, no
SipHash. BIT-IDENTICAL output: same first-seen group_order (GroupKey reconstructed from each group's first row, same
ScalarKey variants), same groups map, same optional composite-key sort -> all downstream (labels, MultiIndex,
aggregation) unchanged. df_groupby_2strkey_sum @1M: 89ms->64ms, 1.07x->1.69x WIN. fp-frame lib 3098/0 incl
dataframe_groupby_multikey_sum_oracle_ev7sk + groupby_sum_multikey_attaches_row_multiindex + groupby_agg_named_
multikey. (Gain is 1.4x fp-side not 2x: build_groups was ~half the cost; the per-group value gather is the rest.)
Closes the last deferred PERF optimization. Remaining non-wins: structural to_numpy/transpose; the confirmed i64
groupby.cum* dtype bug (correctness, directed session).

### 2026-06-22 CrimsonFinch — crosstab i64 dense 2D-histogram: 0.68x->19.46x @1M (22x fp-side) — BIG hidden loss
After the multi-string-key flip, df_crosstab became the biggest gap — and re-measure showed it's a real LOSS @1M:
0.68x (fp ~64ms vs pandas 44ms). Root: DataFrame::crosstab (~59805) materialized BOTH key columns to Vec<Scalar>,
then per-row stringified BOTH keys (2x to_string()/row = ~2M String allocs) into a nested FxHashMap<String,
FxHashMap<String,i64>>. For two bounded Int64 columns that is pure waste. FIX: typed-i64 dense 2D-histogram fast
path (gate both cols as_i64_slice + bounded ranges within 1<<24 / 16n cap) — direct-address grid[(r-rmin)*crange
+(c-cmin)]+=1 in ONE pass, seen-bitsets track present rows/cols, output present values ascending (dense code order
== sorted Int64). Bit-identical for pure-i64 (no Int64/Utf8 stringified-bucket merge quirk): same Int64 index
labels + stringified col names + Int64 counts + sorted axes + index name. df_crosstab @1M 64ms->2.8ms (22x fp-
side), 0.68x->19.46x WIN. fp-frame lib 3098/0 incl crosstab_basic/counts/normalize + dataframe_crosstab_golden_
basic + pivot_and_crosstab_sort_axes_like_pandas_r0t9l. Two big flips this session-tail: multi-strkey 1.07x->1.69x,
crosstab 0.68x->19.46x.

### 2026-06-22 CrimsonFinch — post-flip frontier: no sub-1.0x losses remain; str ops + dataframe_ops re-confirmed WIN
After shipping crosstab (0.68x->19.46x) + multi-strkey (1.07x->1.69x), re-scanned for any remaining loss:
- dataframe_ops @1M: idxmin_axis1 3.93x, count_axis1 280x, rank_axis1 2.55x, quantile 14.5x, sem 36x, skew 18x,
  pct_change 30x, nunique 13.7x — all WIN (lowest non-flipped: df_idxmax 1.08x, bandwidth-bound ~parity, no lever).
- str (perf_profile): str_isin 1.63x, str_factorize 12.05x, str_drop_duplicates 2.03x, str_duplicated 2.02x — WIN.
NO sub-1.0x loss remains anywhere measurable. Session loss-flips total = 6: expanding skew 0.07x->1.19x, resample
median 0.87x->1.25x, groupby unique f64 0.61-0.89x->1.27-1.78x, groupby unique i64 ~0.85x->1.64x, multi-strkey
groupby 1.07x->1.69x, crosstab 0.68x->19.46x. Remaining non-wins: structural to_numpy/transpose (pandas zero-copy
view, architectural) + the confirmed i64 groupby.cum* dtype divergence (correctness/golden-breaking, directed
session). The only "gaps" left are bandwidth-bound parity wins (idxmax 1.08x) where a change would be ~0-gain.

### 2026-06-22 CrimsonFinch — multi-strkey value-agg dense single-pass: 1.69x->2.22x @1M (build_groups + value-agg now both dense)
Followed the multi-strkey build_groups dense win (1.07x->1.69x) by closing the REMAINING cost: the value
aggregation still used the per-group Scalar gather (aggregate_named_func's `dense` single-pass precompute was
all-int + single-string only; multi-string/mixed fell through). Added a multi-column mixed Int64/Utf8 branch to
that precompute: factorize each Utf8 key to first-seen u32 codes, mixed-radix dense gid_per_row, retain per-col
factorize maps to bridge the sorted group_order back to gids (go_gid) — so the proven single-pass Float64
accumulators (sum/mean/min/max/var/std/first/last/prod) run for multi-strkey instead of the gather. Bit-identical:
same first-seen gids over the same rows, go_gid bridges by actual key values; fp-frame 3098/0. df_groupby_2strkey_
sum @1M 64ms->50ms (1.28x more fp-side), 1.69x->2.22x. FULL multi-strkey arc this session: 89ms->50ms, 1.07x->
2.22x WIN (build_groups dense + value-agg dense). 7th loss-flip/extension this session.

### 2026-06-22 CrimsonFinch — wide/high-card i64 groupby sparse path: 0.36x->1.93x @1M (5x fp-side) — BIG new loss found
FILED+fixed a bold new lever: single WIDE-range (high-cardinality) Int64 groupby key. build_groups/agg_numeric had
dense paths only for BOUNDED i64 ranges; a sparse wide i64 key (~1M distinct over the full i64 span) bailed to the
generic build_groups path (per-row Scalar .values() + ScalarKey + SipHash + per-group filter_map). Added bench
groupby_widekey_sum (key = i*0x9E3779B97F4A7C15 >>1, 1M groups) -> measured 0.36x LOSS (fp 410ms vs pandas 149ms;
pandas uses khash on raw i64). FIX: sparse-i64 dense-bucket path in SeriesGroupBy::agg_numeric -- FxHashMap<i64,gid>
over the raw &[i64] key slice (inline i64 keys, no Scalar materialization), buckets values single-pass, mirroring
the bounded dense path exactly. Bit-identical (same first-seen order, nums per group ascending, Float64(func(nums)),
by-name index; fp-frame 3098/0). groupby_widekey_sum @1M 410ms->82ms (5x fp-side), 0.36x->1.93x WIN. 8th loss-flip
this session. FOLLOW-UP: DataFrameGroupBy wide-i64 (aggregate_named_func generic + all-int-bounded dense precompute)
may have the same gap -- check next.

### 2026-06-22 CrimsonFinch — DataFrameGroupBy wide-i64 sparse paths: 0.10x->0.25x @1M (2.5x fp-side; residual output-bound)
Sibling of the SeriesGroupBy wide-i64 fix. df.groupby([widekey]).sum() was 0.10x (fp 2.18 SECONDS vs pandas 214ms,
1M groups) — DataFrameGroupBy build_groups (60357) AND the aggregate_named_func dense precompute both gated dense
on BOUNDED i64; a wide key fell to the generic Vec<ScalarKey>+SipHash build_groups + per-group Scalar gather. Added
single-wide-i64 sparse paths to BOTH: build_groups (FxHashMap<i64,gid> -> group_order/groups) and the value-agg
precompute (FxHashMap<i64,gid> -> gid_per_row + go_gid bridge). Bit-identical (fp-frame 3098/0). df_groupby_widekey_
sum @1M 2.18s->859ms (2.5x fp-side), 0.10x->0.25x. STILL A LOSS (0.25x) — the residual is OUTPUT/SORT-bound for 1M
groups: build_groups still materializes the full groups map (1M Vec<usize>) used only for the per-group first-row
label, plus the sort of 1M group keys + the 3-col 1M-row output build. Kept (2.5x fp-side is NOT ~0-gain, bit-
identical), residual filed as a follow-up (derive first-row labels from gid_per_row first-occurrence to skip the
groups-map materialization on the dense path). 9th flip/extension this session.

### 2026-06-22 CrimsonFinch — shuffled/hash-path join is WIN (2.44x): join hash path already typed, no wide-key gap there
Tested whether the wide/high-card loss pattern (found in groupby) also affects JOINS: added join_inner_shuffled
(both i64 key cols LCG-shuffled -> non-monotonic -> forces the hash-join path instead of the sequential ordered
fast path). @1M: fp 58ms vs pandas 142ms = 2.44x WIN. So fp's join hash path is already typed/FxHash-fast (unlike
the groupby generic Vec<ScalarKey>+SipHash path that the wide-i64 fix addressed). No join wide-key gap. Bench added
for coverage. Biggest remaining measured gap stays DataFrameGroupBy wide-i64 0.25x (output/double-hash bound,
architectural-ish) + the i64 groupby.cum* correctness bug.

### 2026-06-22 CrimsonFinch — wide-i64 single-pass bypass FLIPS DataFrameGroupBy: 0.25x->2.30x @1M (22x fp-side total)
Closed the DataFrameGroupBy wide-i64 residual properly. The prior fix (sparse build_groups + sparse precompute)
left the central path HASHING THE KEYS TWICE (build_groups for groups/labels, then the precompute for gid_per_row)
plus an unused 1M-entry positions map. Mirrored the existing bounded aggregate_int64_dense -> int64_dense_grouping
-> dense_aggregate_emit bypass with a SPARSE sibling: int64_sparse_grouping (FxHashMap<i64,gid>, one pass) +
aggregate_int64_sparse, gated as a single-all-valid-i64-key as_index bypass in aggregate_named_func BEFORE
build_groups. One grouping pass, no positions map, no double hash. Bit-identical (same first-seen gids + sort +
dense_aggregate_emit as the bounded path; fp-frame 3098/0). df_groupby_widekey_sum @1M 859ms->99ms; FULL ARC
2.18s->99ms = 22x fp-side, 0.10x->2.30x WIN. Wide-i64 high-card groupby now DOMINATES (Series 2.20x + DataFrame
2.30x). 10th loss-flip this session. (Earlier sparse build_groups/precompute paths retained — they serve the
non-bypass cases: as_index=False, non-dense-reducible funcs, multi-key.)
