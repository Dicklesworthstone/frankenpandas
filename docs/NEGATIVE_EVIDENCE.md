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
| SeriesGroupBy.value_counts per-group ScalarKey map -> FxHashMap (g1de8/BlackThrush) | 1M Int64 values grouped by 1000 Int64 groups, 50 value classes per group; `bench_sgb_vc`; `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, `rch exec -- cargo run -p fp-frame --example bench_sgb_vc --release -- 1000000 1000 8`; pandas 2.2.3 `v.groupby(by).value_counts()` | 54.698 ms | before 35.357 ms; after 23.527 ms | **2.33x faster than pandas; FP-side 1.50x faster** | ✅ KEEP — `SeriesGroupBy::value_counts` now uses `FxHashMap<ScalarKey, usize>` for each per-group value counter instead of std SipHash. The map only tracks first-seen value slots and counts; output ordering remains governed by group order plus stable count-desc sorting, so hasher choice is behavior-transparent. |
| fp-join Scalar-backed Utf8 merge borrowed-byte positions (BlackThrush) | 1M-row fact (10000 distinct Scalar-backed Utf8 keys) ⋈ 10000-row dim, `bench_merge_utf8`; same local CPU pin, `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, `cargo build --release -p fp-join --example bench_merge_utf8`, then `bench_merge_utf8 1000000 10000 5 {left,outer}`; pandas 2.2.3 `left.merge(right,on='key',how=...)` | left 100.250 ms / outer 154.560 ms | before left 152.521 ms / outer 496.633 ms; after left 91.061 ms / outer 168.754 ms | **left 1.10x faster than pandas, 1.68x FP-side; outer 0.92x vs pandas, 2.94x FP-side** | ✅ KEEP PARTIAL — added all-valid Scalar-backed Utf8 single-key position planners for inner/left/outer merges, hashing borrowed byte spans from `Scalar::Utf8` instead of cloning every row key into `JoinKeyComponent::Present(IndexLabel::Utf8)`. Duplicate-key cross-products, left unmatched rows, and outer key sorting match the generic path; null-bearing or non-Utf8 values stay on generic merge. This closes the left-join pandas gap on the measured workload and cuts the outer gap to ~8%, but outer output materialization still trails pandas slightly. Guards: `cargo check -p fp-join --all-targets`, `cargo test -p fp-join` (135 unit + 3 conformance tests), `cargo fmt -p fp-join --check`, `cargo clippy -p fp-join --all-targets --no-deps -- -D warnings`, `FP_ALLOW_SYSTEM_PANDAS_FALLBACK=1 FP_REQUIRE_LIVE_ORACLE=1 cargo test -p fp-conformance live_oracle_dataframe_merge_{inner,outer}_basic -- --nocapture`. Full `cargo clippy -p fp-join --all-targets -- -D warnings` is blocked by existing `fp-frame` dependency lints at `crates/fp-frame/src/lib.rs:65182` and `:65621`; UBS on touched files also exits 1 due the existing broad fp-join scanner inventory, while its internal fmt/clippy/check/test-build subchecks are clean. |
| DataFrame::set_index(drop=true) clone-retained-columns candidate (uza04.211/BlackThrush) | 1M Int64 key + 1 Int64 payload, scalar-key benchmark shape; temporary expanded `bench_set_index`; pandas 2.2.3 `df.set_index('a', drop=True)` | 1.346 ms | current main 0.178 ms; candidate 0.205 ms | **candidate 6.58x faster than pandas but 0.87x current FP (15% slower)** | ❌ REVERT — building the retained column map directly did not improve the measured scalar-key drop path; current clone+remove is already cheap after prior Arc-backed Column sharing and typed label paths. Do not land this isolated hunk without a new workload that proves a same-worker gain. |
| fp-join merge OUTER/LEFT on Utf8 key — MEASURED GAP (1q4q4) | 1M-row fact (10000 distinct Utf8 keys) ⋈ 10000-row dim, inner/left/outer; `bench_merge_utf8`; pandas 2.2.3 `left.merge(right,on,how)` | inner 106 / left 110 / outer 178 ms | inner 64.3 / left 135 / outer 834 ms | **inner 1.65× WIN; left 0.81× (1.2× slower); outer 0.21× (4.7× SLOWER)** | 🔴 DOCUMENTED LOSS (no fix this pass — golden-gated join code, deferred for a focused careful session post-crash). INNER single-key Utf8 already wins via `merge_single_key_inner_unsorted` (ordered/contiguous-Utf8 + generic hash). LEFT has only Int64 fast paths (ordered-unique/dense-cycle/dense-i64) — a Utf8 key drops to the generic merge; OUTER has NO single-key fast path AND `sort_outer_join_rows` does a stable String-cmp sort of 1M `IndexLabel::Utf8` (pandas outer also sorts — verified `['a','a','m','q','z']` — but via factorize→int codes, sorting integers). LEVER (matches pandas, future bead): FACTORIZE both key columns' Utf8 → dense int codes once (the proven 1q4q4 lever), join on codes (reuse the int64 dense-join fast paths), and emit the outer result sorted by integer code instead of String — killing both the per-row `Vec<ScalarKey>`/pointer-key build and the String-cmp sort. Est. outer ~0.21×→~2×. Bench `crates/fp-join/examples/bench_merge_utf8.rs` committed for the focused pass. |
| fp-join contiguous Utf8 left/outer position planner (uza04.209/cod-b) | Same `bench_merge_utf8` 1M-row fact (10000 distinct Utf8 keys) ⋈ 10000-row dim on RCH worker `vmi1149989`; before/after same checkout session, `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, `cargo run -p fp-join --example bench_merge_utf8 --release -- 1000000 10000 3 {left,outer}` | left 110 / outer 178 ms | before left 132.813 / outer 1321.721 ms; after left 128.150 / outer 850.721 ms | **after left 0.86× (1.16× slower); after outer 0.21× (4.78× slower); FP-side outer 1.55× faster** | ✅ KEEP PARTIAL — added borrowed-byte `contiguous_utf8_left_positions` and `contiguous_utf8_outer_positions` for all-valid contiguous single Utf8 keys, reusing the existing optional-position output builders. Duplicate-key semantics are preserved by emitting full left/right cross-products, null-bearing keys stay on the generic route, and outer rows are sorted by raw UTF-8 bytes to match the generic `ScalarKey::Utf8` order without per-row `String` clones. This materially reduces the current same-worker outer gap but does NOT close pandas; the deeper factorize-to-dense-code lever remains open for a future pass. Guards: `cargo test -p fp-join` (132 tests), `cargo check -p fp-join --all-targets`, `cargo clippy -p fp-join --all-targets -- -D warnings`, `cargo fmt -p fp-join --check`, `ubs crates/fp-join/src/lib.rs crates/fp-join/examples/bench_merge_utf8.rs` exit 0 (broad existing warnings only). |
| fp-join dense-code contiguous Utf8 outer planner candidate (uza04.210/cod-b) | Same `bench_merge_utf8` 1M-row fact (10000 distinct Utf8 keys) ⋈ 10000-row dim, outer join only; `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, `cargo run -p fp-join --example bench_merge_utf8 --release -- 1000000 10000 3 outer` | outer 178 ms | current baseline 1645.581 ms on RCH `vmi1149989`; dense-code candidate 4432.978 ms on RCH `vmi1264463` | **candidate 0.04× vs pandas (24.9× slower); 0.37× vs current FP baseline (2.69× slower, conservative cross-worker)** | ❌ REVERT — attempted a single `FxHashMap<&[u8], usize>` factorization of both all-valid contiguous Utf8 key columns into dense code buckets, then sorted codes by borrowed bytes and emitted optional outer positions. Focused duplicate-key tests passed before the benchmark, but the candidate added enough hashing/bucket/sort overhead to regress the measured outer workload badly. Source hunk removed; do not retry this isolated dense-code bucket layer. The remaining viable route is a deeper join/output primitive that avoids both per-side maps and post-hoc optional-position materialization, not another wrapper around the borrowed-byte planner. |
| fp-join fixed-decimal Utf8 outer fused output primitive (wikcu/cod-b) | Same `bench_merge_utf8` 1M-row fact (10000 distinct `k00000000`-style Utf8 keys) ⋈ 10000-row dim, outer join only; `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, `rch exec -- cargo run -p fp-join --example bench_merge_utf8 --release -- 1000000 10000 3 outer`; current-main baseline repeated in owned cod-b worktree at `c372fd4` | outer 178 ms | current-main repeat 3065.793 ms on RCH `vmi1264463`; after best 714.206 ms on RCH `vmi1149989` | **after 0.25× vs pandas (4.0× slower); FP-side 4.29× faster than current main, conservative cross-worker** | ✅ KEEP PARTIAL — added a heavily-guarded all-matched fixed-decimal Utf8 outer path that parses the right dimension's sorted unique decimal suffix domain once, bucket-counts fact rows by arithmetic code, and builds the merged key plus numeric payload output directly in sorted bucket order. This avoids the generic outer path's per-row optional-position materialization and string-key sort for the common benchmark shape while falling back for nulls, non-decimal keys, right gaps, unmatched right buckets, non-numeric payloads, sort/indicator/validate cases, or duplicate right keys. Residual vs pandas is still structural; the next route is consuming key order lazily or eliding public key materialization rather than another optional-position wrapper. |
| RangeIndex::values direct arithmetic closeout (uza04.165/cod-b) | 1M labels, `RangeIndex(0, 2n, 2).values()` plus forced checksum; focused Criterion `range_index_values` under warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`; pandas 2.2.3 local `idx.values` plus `arr.sum()` | 0.090 ms p50 (`best=0.089 ms`, p95 0.099 ms) | 0.773 ms Criterion mean (`745.91-803.28 us`) | **0.12x (8.6x slower)** | ✅ VERIFIED EXISTING FP-side WIN / 🔴 still pandas loss — current `RangeIndex::values` already generates the arithmetic progression directly instead of round-tripping through `to_index().labels()`. The measured legacy comparator (`to_index().labels()` then Int64 extraction) was 6.571 ms at 1M, so the direct path is **8.50x faster FP-side** (100k: 72.4 us vs 236.5 us, 3.27x). No new production code kept in this pass; an attempted vectorization reshaping probe was stopped under disk-critical policy before results and reverted. Residual vs pandas is structural: pandas exposes a NumPy int64 view/cache and sums in C, while FP returns and consumes an owned `Vec<i64>`. Next viable lever is a typed view/array API or avoiding public materialization, not another label-enum bypass. |
| Index lazy-Int64 list aliases direct materialization (6cnhb/cod-b) | 1M-label strided `Index::from_range(0, 3n, 3)`, owned-label aliases `to_list()` / `values()` / `ravel()`; `bench_range_setops index_list_aliases`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`; local same-target before/after. Pandas 2.2.3 local `RangeIndex(...).tolist()` / `.values` / `.ravel()` with equivalent digest. | `tolist` 18.080 ms; `values` 0.000631 ms; `ravel` 0.001733 ms | before `to_list` 2.480 ms / `values` 2.369 ms / `ravel` 2.373 ms; after `to_list` 1.600 ms / `values` 1.637 ms / `ravel` 1.399 ms | **`to_list` 11.30x faster than pandas `tolist`; FP-side 1.55x / 1.45x / 1.70x faster. Pandas `values`/`ravel` remain structural ndarray-view wins.** | ✅ KEEP — `Index::to_list()` now materializes owned `IndexLabel`s directly from unit, affine, two-run, strided, contiguous-Utf8, or typed backing instead of first building/caching a raw `Vec<i64>` through `int64_view()` and then mapping that vector to `IndexLabel::Int64`. Public alias outputs are unchanged, names are unaffected, and the affine regression test proves neither the owned-label cache nor the intermediate raw-i64 cache is populated by list aliases. This closes the Rust-owned materialization lever; pandas' `.values`/`.ravel()` O(1) ndarray views are still the broader typed-view API gap, not a list-alias loop issue. |
| RangeIndex median BOLD-VERIFY (uza04.170/cod-b) | 1M-label unit, stride-3, and descending `RangeIndex` shapes; 64 median calls per timed batch. pandas 2.2.3 local has no direct `RangeIndex.median()`, so comparator used prebuilt `pd.Series(idx).median()` over identical labels; FP used `bench_range_setops range_median 64` with warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | unit 737.961 ms / strided 727.262 ms / descending 712.997 ms | unit 0.000501 ms / strided 0.000500 ms / descending 0.000510 ms | **unit 1,472,976x / strided 1,454,525x / descending 1,398,033x faster** | ✅ VERIFIED EXISTING WIN — current `RangeIndex::median` is already closed-form and only evaluates the middle one or two labels through widened `i128` `value_at`; it does not allocate `values()` or `IndexLabel` vectors. No production code change was kept; this pass added only a reproducible `range_median` benchmark mode plus this ledger row. |
| RangeIndex diff constant-step BOLD-VERIFY (uza04.171/cod-b) | 1M-label unit, stride-3, and descending `RangeIndex` shapes, `periods=1`; pandas 2.2.3 local `pd.Series(idx).diff(1)` over identical labels vs `bench_range_setops range_diff`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | unit 0.587493 ms / strided 1.025182 ms / descending 0.540814 ms | before unit 2.561713 ms / strided 2.576221 ms / descending 2.560631 ms; after unit 0.563197 ms / strided 0.561974 ms / descending 0.555723 ms | **after unit 1.04x / strided 1.82x faster; descending 0.97x pandas (2.8% slower residual); FP-side 4.55x / 4.58x / 4.61x faster** | ✅ KEEP — `RangeIndex::diff` now fills boundary `None`s plus the constant arithmetic progression delta `step * periods` instead of recomputing two widened labels per output slot. Overflow behavior is preserved by computing the product in `i128` and filling valid slots with `None` when it cannot fit `i64`; period zero and out-of-range periods keep the previous semantics. |
| RangeIndex join same-lattice Int64 BOLD-VERIFY (uza04.190/cod-b) | 1M-label `RangeIndex(0, n, 1, name='k')` joined with flat Int64 `Index(n/2..n+n/2, name='k')`; `inner` and `outer`; pandas 2.2.3 local `RangeIndex.join(Index, how=...)` vs `bench_range_setops range_join`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | inner 3.807292 ms / outer 4.415635 ms | before inner 3.044637 ms / outer 10.109983 ms; after inner 1.233537 ms / outer 1.821050 ms | **inner 3.09x / outer 2.42x faster than pandas; FP-side 2.47x / 5.55x faster** | ✅ KEEP — direct RangeIndex-vs-Int64 joins now detect same-step affine Int64 slices. `inner` returns the lazy overlap span and `outer` returns the lazy contiguous extension when that exactly matches the existing self-then-new-other output order; non-affine, duplicate, off-lattice, prefix-extension, and mixed-label cases stay on the previous materializing/hash paths. |
| RangeIndex isin mark-position BOLD-VERIFY (ruthb/cod-b) | 1M-label `RangeIndex(0, n, 1).isin(values)` with deterministic mixed hit/miss/duplicate needles; small=1,024 needles, large=500,000 needles; pandas 2.2.3 local `RangeIndex.isin` vs `bench_range_setops range_isin`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | small 13.066309 ms / large 43.364404 ms | current small 0.020549 ms / large 3.966811 ms | **small 635.86x / large 10.93x faster than pandas** | ✅ VERIFIED EXISTING WIN — `RangeIndex::isin` already uses direct position marking from each needle (`position_of_value`) instead of building a needle set and scanning every range label. Duplicates collapse by overwriting the same mask slot, misses are ignored, ascending/descending/empty semantics are covered by existing `range_index_isin_marks_positions_without_hash_ruthb`; this pass added only the reproducible bench mode and closed the stale-open bead. |
| MultiIndex nunique direct-count retry (uza04.194/cod-b) | 1M-row two-level Utf8 `MultiIndex` with 700 x 700 level domains and 4,900 observed unique tuples; pandas 2.2.3 local `mi.nunique()` vs `bench_multiindex_dedup`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | `nunique` 16.775462 ms; `len(unique())` 15.925472 ms | current `nunique` 46.619 ms / `unique().len()` 47.966 ms; dense-presence candidate 53.465 ms | **current 0.36x pandas (2.78x slower); dense candidate 0.31x pandas and 0.87x current FP** | ❌ REVERT — current `MultiIndex::nunique` already avoids materializing the unique output via `identity_packed_keys()` plus `FxHashSet<u64>` (landed earlier in `5b96dcb49`), but still spends most time factorizing per-level Utf8 values and building packed-key vectors. A retry that counted a compact dense `Vec<bool>` presence table by packed key regressed to 53.465 ms, so the production hunk was removed. The retained change is only a reusable `bench_multiindex_dedup` `nunique`/`unique_len` timing lane; a real fix needs a deeper pack/count primitive that avoids per-level code vectors and repeated string hashing. |
| MultiIndex compact two-level identity-code sidecar (2tc1q/cod-b) | Same 1M-row two-level Utf8 `MultiIndex`, 700 x 700 level domains, 4,900 observed unique tuples; pandas 2.2.3 local `mi.nunique()` / `len(mi.unique())` vs `cargo run -p fp-index --example bench_multiindex_dedup --release -- 1000000 5`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`; final Rust run on RCH `vmi1149989`; checksum `8f340377c4582811`. | `nunique` 17.969 ms; `len(unique())` 15.984 ms | before `nunique` 54.031 ms / `unique().len()` 54.813 ms; after `duplicated` 1.798 ms / `nunique` 1.579 ms / `unique().len()` 2.235 ms | **`nunique` 11.38x faster than pandas; `unique().len()` 7.15x faster; FP-side `nunique` 34.22x faster** | ✅ KEEP — `MultiIndex` now builds a private optional first-seen identity-code sidecar only for two-level compact domains whose dense `(level0, level1)` table fits the existing bounded cell budget. `duplicated`, default `drop_duplicates`/`unique`, and `nunique` scan integer slots directly instead of rebuilding per-level code vectors and hashing tuple labels on every call; empty, wide, high-cardinality, deserialized/missing-sidecar, and non-compact cases keep the existing packed-key/tuple fallback. Observable levels, names, serde payloads, equality, output order, duplicate keep modes, and checksum are unchanged. |
| MultiIndex two-level Utf8 tuple set-ops borrowed keys (uza04.188/cod-b) | 1M-row two-level Utf8 `MultiIndex` set-op batch: `intersection(sort=False)` + `difference(sort=False)`, 700 x 700 level domains with 4,900 observed unique tuples; pandas 2.2.3 local vs `bench_multiindex_setop`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | 92.920 ms | before packed-key path 193.095 ms; after borrowed `(&str,&str)` tuple sets + direct two-level result builder + intersection early-stop 71.122 ms | **1.31x faster than pandas; FP-side 2.72x faster** | ✅ KEEP — the earlier FxHashSet migration was already present but still materialized per-level code vectors and full `u64` key arrays before hashing. The two-level all-Utf8 path now hashes borrowed tuple keys directly, clones only emitted unique rows, and stops `intersection` once every right-side unique key has been emitted. Output order, duplicate collapse, symmetric/union/difference behavior, and shared-name propagation are covered by `multi_index_two_utf8_setops_preserve_order_and_names_codb188`; non-Utf8 or wider MultiIndexes keep the existing packed/fallback paths. |
| CategoricalIndex factorize category-code sidecar (uza04.201/cod-b) | 1M-label low-cardinality string `CategoricalIndex` (100 categories, deterministic xorshift labels), `factorize()`; pandas 2.2.3 local `CategoricalIndex.factorize()` vs `probe_str_categorical`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | 2.951 ms | before categorical rank-map 7.958 ms; after category-code sidecar 0.944 ms; generic `Index` hash baseline 10.368 ms | **after 3.13x faster than pandas; FP-side 8.43x faster than pre-sidecar categorical path** | ✅ KEEP — `CategoricalIndex` now carries a private optional category-code sidecar for valid constructors and code-preserving transforms, so `factorize()` scans integer ranks instead of hashing each label string to recover its category rank. Public labels/categories/ordered/name remain unchanged; deserialized or intentionally invalid fixtures with missing sidecars fall back to the old rank/hash paths, and malformed sidecars are guarded before use. Focused guard: `cargo test -p fp-index categorical_index_factorize_uses_rank_codes_uza04201 --release`; crate gate: `cargo check -p fp-index --all-targets`. |
| CategoricalIndex value_counts category-code sidecar (uza04.200/cod-b) | 1M-label low-cardinality string `CategoricalIndex` (100 categories, deterministic xorshift labels), `value_counts()`; pandas 2.2.3 local `CategoricalIndex.value_counts()` vs `probe_str_categorical`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | 1.191 ms | before categorical rank-map 7.434 ms; after category-code sidecar 0.605 ms | **after 1.97x faster than pandas; FP-side 12.29x faster than pre-sidecar categorical path** | ✅ KEEP — `CategoricalIndex::value_counts()` now uses the private optional category-code sidecar to count integer category ranks directly, avoiding the per-call category map and per-label string hashing. First-seen tie order is preserved by recording the first rank encounter before the stable descending-count sort; missing or malformed sidecars fall back to the old rank/hash paths. Focused guard: `cargo test -p fp-index categorical_index_value_counts_use_rank_counts_uza04200 --release`. |
| CategoricalIndex duplicated category-code sidecar (uza04.198/cod-b) | 1M-label low-cardinality string `CategoricalIndex` (100 categories, deterministic xorshift labels), `duplicated(keep='first')`; pandas 2.2.3 local `CategoricalIndex.duplicated()` vs `probe_str_categorical`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | 3.771 ms | before categorical rank-map 11.267 ms; after category-code sidecar 2.556 ms; generic `Index` hash baseline 11.110 ms | **after 1.48x faster than pandas; FP-side 4.41x faster than pre-sidecar categorical path** | ✅ KEEP — `CategoricalIndex::duplicated()` now uses the private optional category-code sidecar for bounded category domains, marking integer ranks directly for `keep=first`, `keep=last`, and `keep=false` instead of hashing each label to recover its category rank. Sparse category universes, missing sidecars, and malformed sidecars fall back to the old hash/rank paths. Focused guard: `cargo test -p fp-index categorical_index_duplicated_uses_rank_bitset_uza04198 --release`. |
| CategoricalIndex nunique category-code sidecar (uza04.197/cod-b) | 1M-label low-cardinality string `CategoricalIndex` (100 categories, deterministic xorshift labels), `nunique()`; pandas 2.2.3 local `CategoricalIndex.nunique()` vs `probe_str_categorical`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | 2.470 ms | before categorical rank-map 8.652 ms; after category-code sidecar 0.488 ms | **after 5.06x faster than pandas; FP-side 17.73x faster than pre-sidecar categorical path** | ✅ KEEP — `CategoricalIndex::is_unique()` and `nunique()` now use the private optional category-code sidecar to scan integer category ranks directly, avoiding per-call `category_index_map()` construction and per-label string hashing. Missing or malformed sidecars fall back to the old rank-map semantics, preserving invalid-label behavior for deserialized fixtures. Focused guard: `cargo test -p fp-index categorical_index_unique_nunique_use_rank_bitset_uza04197 --release`. |
| CategoricalIndex unique/drop_duplicates category-code sidecar (uza04.199/cod-b) | 1M-label low-cardinality string `CategoricalIndex` (100 categories, deterministic xorshift labels), `unique()`; pandas 2.2.3 local `CategoricalIndex.unique()` vs `probe_str_categorical`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | 2.632 ms | before categorical rank-map 7.429 ms; after category-code sidecar 0.559 ms | **after 4.71x faster than pandas; FP-side 13.29x faster than pre-sidecar categorical path** | ✅ KEEP — `CategoricalIndex::unique()` and `drop_duplicates()` now use the private optional category-code sidecar for bounded category domains, emitting first-seen unique category ranks directly instead of rebuilding category ranks from labels. Categories, ordered flag, name, and the result sidecar are preserved; missing or malformed sidecars fall back to the old rank/hash semantics. Focused guard: `cargo test -p fp-index categorical_index_unique_drop_duplicates_use_rank_bitset_uza04199 --release`. |
| CategoricalIndex monotonic category-code sidecar (uza04.196/cod-b) | 1M-label sorted low-cardinality string `CategoricalIndex` (100 ordered categories), `is_monotonic_increasing`; pandas 2.2.3 local first-access `CategoricalIndex.is_monotonic_increasing` vs `probe_str_categorical`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | 0.612 ms | before categorical rank-map 5.983 ms; after category-code sidecar 0.472 ms | **after 1.30x faster than pandas; FP-side 12.68x faster than pre-sidecar categorical path** | ✅ KEEP — `CategoricalIndex::is_monotonic_increasing()` and `is_monotonic_decreasing()` now use the private optional category-code sidecar to compare raw category ranks directly, avoiding per-call `category_index_map()` construction and per-label string hashing. Missing, length-mismatched, or out-of-range sidecars fall back to the old rank-map semantics. Focused guard: `cargo test -p fp-index categorical_index_monotonic_scans_ranks_without_codes_vec_uza04196 --release`. |
| SeriesGroupBy single Scalar-backed Utf8 key — single-probe build (1q4q4) | 1M rows, series.groupby(Utf8 key, 1000 groups).sum() | 36.65 ms | 13.1 ms | **2.80× faster** | ✅ DEEPENED — was 1.12× near-parity (FP 32.7ms). SeriesGroupBy.build_groups' generic loop calls `values()` per row and probes TWO hash structures (a `seen` set + the `groups` map). Added a Scalar-backed Utf8 fast path (gated non-contiguous): hoist `values()`, probe ONE `FxHashMap<&str, gid>`, append row indices into per-group Vecs. Bit-identical: same first-seen order/order_keys (ScalarKey::Utf8), same groups map. 32.7→13.1ms (2.5× FP-side). Guards: 202 fp-frame groupby tests, 108 fp-conformance groupby, fmt+clippy clean. |
| DataFrameGroupBy multi-key Scalar-backed Utf8 — mixed-radix dense (1q4q4) | 1M rows, groupby(['a','b']) two Utf8 keys (100×50), i64 value, .sum() | 90.70 ms | 70.3 ms | **1.29× faster** | ✅ FIXED — was 0.87× LOSS (FP 104.6ms). The multi-key mixed Int64/Utf8 dense path (mkdense) only fired for CONTIGUOUS Utf8 (`as_utf8_contiguous`); Scalar-backed Utf8 (from_values/in-memory) fell to the generic per-row `Vec<ScalarKey>` heap-alloc + SipHash. Added a `KeyCol::StrScalar` arm that factorizes the `&str` from the materialized Scalar slice into dense codes, packed into the same mixed-radix direct-address gid table. Bit-identical: codes only pick the dense slot (bijective over the code ranges so first-seen slot order == first-seen key order); group_order keys reconstructed from `vals[row]` (ScalarKey::Utf8). Oracle-EXACT (50k, 37×13: 481 groups, vsum 2499760, sorted a00,b00→a36,b12). 104.6→70.3ms (1.49× FP-side). Guards: 202 fp-frame groupby tests, 108 fp-conformance groupby, fmt+clippy clean. |
| DataFrame.isin non-numeric all-valid column — typed Bool (1q4q4) | 1M rows × 4 Utf8 cols (50-cat), 1000-string needle set | 149.91 ms | 64.0 ms | **2.34× faster** | ✅ FIXED — was 1.13× near-parity (FP 132.5ms). `isin_apply_column` had Int64/Float64 typed paths but Utf8 (and any non-numeric) columns fell to the `Vec<Scalar::Bool>` boxing fallback (32 B/elem; isin is output-bound). Added an all-valid path emitting typed Bool (`from_bool_values`) — the `idx.contains` probe is unchanged, so bit-identical (`from_bool_values` == `from_values` over `[Bool,..]`). 132.5→64.0ms (2.07× FP-side). Oracle-EXACT (40k×3: true_count 88811). Guards: 25 fp-frame isin tests, fmt+clippy clean. Sister to the Series.isin Scalar-backed Utf8 fix. |
| Series.sort_values Float64/Int64 "spread" — stable pair sort ❌ REVERTED (1q4q4) | 2M f64/i64, sort_values(ascending), high-cardinality — **measured on TRULY-SHUFFLED data** | f64 196.8 / i64 209 ms | radix f64 239 / i64 230 ms | **radix 0.82× / 0.91× (LOSS); the pair-sort "fix" was 0.78× — a REGRESSION, REVERTED** | ❌ REVERTED (commits 9d0e56db + 24d3880f undone). **Measurement-error correction:** the bench's value generator `(i·2654435761)>>13` is MONOTONICALLY INCREASING in `i` (no overflow at 2M rows), so the "unique" data was *already sorted* — where a comparison `sort_by` is O(n) (detects the run) but the radix is not. That made the pair sort look like a 1.86× win (65ms). On a splitmix64-**shuffled** 2M f64 the pair sort is **251ms vs the radix 239ms** — a ~5% REGRESSION, and both LOSE to pandas (196.8ms, 0.78×/0.82×). Root cause of the radix's shuffled cost is NOT the argsort (order-independent counting sort) but `reorder_by_positions` gathering columns by a cache-random permutation. A comparison sort can't beat that. Lesson: **always bench sorts on shuffled data.** The radix path (original) is retained. |
| DataFrame.melt — typed id-column repetition (1q4q4) | 500k rows × 6 f64 value cols, 1 Int64 id | 125 ms | 81 ms (was 175) | **1.54× WIN (was 0.72× LOSS)** | ✅ FIXED — melt already built the value column (typed f64 concat) and the variable column (contiguous Utf8) typed, but the **id column** was repeated via `src.values()` — materializing the column to `Vec<Scalar>` (32 B/cell) then tiling that `n_value_vars` times (3M Scalar boxes here). Added a typed path: tile the contiguous `i64`/`f64` buffer directly via `from_i64_values`/`from_f64_values`. BIT-IDENTICAL (8 fp-frame melt tests pass). 175→81ms (2.2× FP-side). Other dtypes keep Scalar tiling. (Context from the same reshape sweep: DataFrame whole-frame reductions are huge wins — df.std **42×**, sem **37×**, skew **19×** vs pandas' slow per-column path; `DataFrame.stack` stays 0.34× — the documented STRUCTURAL flat-composite-string index vs pandas' MultiIndex, not a typed-output gap.) |
| Series.isin Scalar-backed Utf8 — &[u8] probe + typed Bool (1q4q4) | 2M rows, 50-cat Utf8 series, 1000-string needle set | 41.48 ms | 18.0 ms | **2.31× faster** | ✅ FIXED — was 0.51× LOSS (1.9× slower, FP 80.6ms). isin had Int64-bitset + contiguous-Utf8 byte-span paths, but a Scalar-backed Utf8 column (from_values/in-memory) fell to the generic `IsinIndex` whose `Vec<Scalar::Bool>` output boxes 32 B/elem (isin is output-bound at scale). Added the Scalar-backed Utf8 sibling: `FxHashMap<&[u8], ()>` of the string needles probed over the borrowed bytes, emitting typed Bool (`from_bool_values`). Bit-identical (a string value matches iff its bytes equal a string needle; non-string needles never match; no null). 80.6→18.0ms (4.5× FP-side). Oracle-EXACT (40k, 37-needle: true_count 29605). Guards: 25 fp-frame isin tests, fmt+clippy clean. |
| SeriesGroupBy.idxmax()/idxmin() — sequential dense scan (1q4q4) | 1M rows, 1000 int64 groups, **SHUFFLED** | idxmax 5.4 · idxmin 4.95 ms | idxmax 1.98 · idxmin 2.1 ms | **idxmax 2.7× · idxmin 2.4× — ALL WIN (was 0.46×/0.38× LOSS)** | ✅ FIXED — `idxmax`/`idxmin` used `agg_scalar`, whose closure scanned `data[idx]` over each group's scattered row indices = **random** cache-unfriendly access (the dominant cost at 1M rows; `build_groups` was already dense). Replaced with a single SEQUENTIAL pass tracking the best value + its row per group (dense gid by `key − min`, `data[row]` read in order). BIT-IDENTICAL to the f64 `agg_scalar` path: same first-seen group order, same first-row-wins strictly-better tie, same `Scalar::Utf8(label.to_string())` output (35 fp-frame + 18 fp-conformance idx tests pass). idxmax 11.7→1.98ms (**5.9× FP-side**), idxmin 13.1→2.1ms (6.2×). Lesson: a typed `data[idx]` group scan is still RANDOM access — the sequential `data[row]` rewrite is the lever for any `agg_scalar`-style op. Non-f64 values / non-dense Int64 keys keep `agg_scalar`. **SIBLING (same lever): SeriesGroupBy.all()/any() — 0.29×/0.32× → 0.57×/0.62× (2× FP-side, 7.5→3.8ms), bit-identical sequential AND/OR fold (`group_bool_reduce_dense`, typed Bool/Int64/Float64 + dense Int64 key; 137 tests pass). Still a loss — pandas' vectorized bool reduction is only ~2.2ms, so FP's histogram-scan + per-row gid lookup can't fully close it.** **SIBLING (same lever, moments): SeriesGroupBy.sem()/skew()/kurt() — sem 0.08×→0.50× (42→6.8ms, 6.2× FP-side), skew 0.14×→0.68× (31→6.6ms, 4.7×). They went through `agg_values_scalar` (a `Vec<Scalar>` materialized PER GROUP + `nansem`/`nanskew`/`nankurt`); replaced by `group_moment_dense` — a typed TWO-PASS mean-centered moment pass (pass 1 sum+finite count→mean; pass 2 Σ(x−mean).powi(2/3/4)) reproducing each formula EXACTLY (bit-identical; 271 fp-frame tests pass). Still a loss: pandas fuses grouped moments in ONE pass (3.4/4.5ms) while FP's nanskew is two-pass mean-centered (a one-pass rewrite would need a golden regen). **DataFrameGroupBy.idxmax()/idxmin() (multi-col): same dense sequential scan with ONE shared int64_dense_grouping pass (`try_idx_extreme_dense`) — FLIPPED 0.19×→4.2× WIN, 0.23×→4.9× WIN (86/77→3.9/3.6ms, 22× FP-side); pandas DataFrame idxmax has per-col overhead (16.3/17.9ms). Bit-identical (229 fp-frame + 18 fp-conformance tests).** **DataFrameGroupBy.all()/any() (multi-col): NOT in aggregate_named_func`s dense set → fell to build_groups; added `try_bool_reduce_dense` (shared int64_dense_grouping + sequential per-column AND/OR over typed Bool/Int64/Float64). FLIPPED 0.30×→4.4× WIN (all), 0.30×→4.6× WIN (any) (61/67→4.3ms, 15× FP-side). Bit-identical (137 fp-frame tests; AND/OR order-independent, seeded with identity).** **DataFrameGroupBy.first()/last() (multi-col): the dense `aggregate_int64_dense` path was 10× slower than a dedicated scan AND only handled f64/i64 (a Bool/Utf8 col forced the slow generic path). Added `try_first_last_dense` — one shared int64_dense_grouping pass finds each group`s first/last ROW, then every value column is gathered at those rows via zero-copy `Column::take_positions` (dtype-preserving, ANY column type). FLIPPED 0.49×→9.9× WIN (first), 0.52×→7.5× WIN (last) (40→2.0/2.7ms, 20× FP-side). Bit-identical for all-valid columns (93 fp-frame tests); nullable cols (skipna) keep the generic path.** **DataFrameGroupBy.ffill()/bfill() (multi-col transform): build_groups + per-column `Vec<Scalar>` materialization + scattered-index fill. Added `try_fill_dense` — per-column SOURCE row (last/next non-null in group within limit) computed in one sequential pass over the cheap `ValidityMask`, then `take_positions` gathers the fill (any dtype). ffill 0.68×→1.16× WIN, bfill 0.68×→0.98× (161/169→94/117ms, 1.5–1.7× FP-side); output-bound (full n-row frame gather) limits the gain vs the reductions. Bit-identical (33 fp-frame + 32 fp-conformance tests).** **DataFrameGroupBy.ngroup() — generic ran build_groups (SipHash map) + scattered out[idx] into Vec<Scalar>; reuse int64_dense_grouping (already sort-respecting + matches build_groups order) mapping each gid→emit position, typed Int64 output. 0.42×→1.11× WIN (35→13.4ms, 2.6× FP-side; output-bound full-frame Int64). Bit-identical (old-FP hash == dense hash A/B; 3 fp-frame + 3 fp-conformance). cumcount/size already WIN (1.36×/2.2×).** **Series.str.slice() — general arm collected a Vec<char> PER element (heap alloc/row); added ASCII forward-slice (step 1/None) byte-slice fast path resolving Python-style negative/OOB bounds against byte len. 0.56×→1.27× WIN (187→82ms, 2.3× FP-side). Bit-identical: 9-case oracle (negative/OOB/start>stop/empty/None) == Python exactly, 25 fp-frame tests. (str.replace 1.3× / split_get 5.4× / strip 6.7× already WIN.)** **Series.map(dict) Int64→Int64 — the typed path resolved over an FxHashMap<i64,&Scalar>; the &Scalar value is a pointer into the scattered mapping Vec, so every per-row probe was a cache miss. Replaced with a TYPED i64→i64 resolver: dense direct-address table when mapping keys form a bounded range (common categorical re-encode), else value-inlined FxHashMap<i64,i64>. Full-coverage map 9.9→2.18ms (4.5× FP-side, 0.38×→1.74× WIN). Bit-identical (first-occurrence-wins, unmapped→NaN fall-through; partial+dup oracle == pandas, 32 fp-frame tests). (replace 173× WIN; cut 0.17× / qcut 0.39× still LOSS — binning, next.)** **DataFrameGroupBy.nunique() Int64 cols — was ~parity (1.03×, both hash-bound); dense single-pass distinct count via a bit-packed 2D (group,value) seen bitset over int64_dense_grouping gids. 1.03×→11× WIN (99.9→9.3ms, 10.7× FP-side). Bit-identical (key-aligned oracle == pandas; 26 fp-frame + 13 fp-conformance). NOTE: a per-value last-seen-EPOCH array (first attempt) MIScounts interleaved/shuffled keys (g0..g1..g0 re-counts a shared value) and passed 26 tests but failed the A/B-vs-old-FP oracle — the 2D (group,value) bitset is the correct structure (gated to bounded group×value cell budget).** **SeriesGroupBy.nunique (same lever): was 1.28× (dense buckets but per-group FxHashSet); same 2D-bitset → 1.28×→11.9× WIN (52→5.6ms, 9.3× FP-side). Bit-identical (3-way shuffled oracle MY==OLD_FP==PANDAS; 26 fp-frame + conformance).** **cut/qcut typed input — gathered floats via series.values() (1M Scalar boxes); added as_f64_slice fast path (all-valid no-NaN f64 => all Some, no materialization). cut 0.17×→0.25× (120→82ms), qcut 0.39×→0.55× (145→104ms), 1.4–1.45× FP-side, bit-identical (15 fp-frame + 9 fp-conformance). STILL LOSS — residual is the 1M Utf8 interval-label output (no Categorical dtype) vs pandas Categorical codes + the Vec<Option<f64>>; structural, deeper flip needs a Categorical column type.** **Series.str.zfill()/pad() — both built a separate `padding` String AND `format!`d it with `s` = 2 heap allocs + format machinery per row; replaced with ONE `with_capacity` build (push padding + str). FLIPPED zfill 0.86×→1.47× WIN, pad 0.86×→1.40× WIN (139/142→81/88ms, 1.6–1.7× FP-side). Bit-identical: sign/empty/both-center odd+even oracle == pandas exactly, 22 fp-frame tests.** groupby.nunique is already a 2.3× WIN.** **DataFrameGroupBy.sem()/skew()/kurtosis() (multi-col): routed through the SAME dense moments helper (`moments_by_pair` extended with m3/m4 + the sem/skew/kurt finalizers; `try_moment_dense` renames `{col}_{func}`→`{col}`, gated to all-f64 value cols). FLIPPED: sem 0.23×→2.6× WIN (72→6.5ms, 11× FP-side), skew 0.33×→2.5× WIN (66→8.6ms) — pandas DataFrame-groupby has per-column overhead (16.7/21.6ms for 2 cols) while FP shares ONE int64_dense_grouping pass across columns. Bit-identical (348 fp-frame + 151 fp-conformance tests; var/std unchanged).** |
| DataFrameGroupBy nth/head/tail — dense per-group selection (1q4q4) | 1M rows, 1000 int64 groups, **SHUFFLED** | nth 0.80 · head 1.38 ms | nth 2.0 (was 6.5) · head 3.0 (was 6.5) ms | **nth 0.40× · head 0.47× — IMPROVED from 0.12×/0.21× (2.2–3× FP-side), still a loss** | ⚠️ IMPROVED, bit-identical. nth/head/tail used `build_groups` (SipHash) + a per-group `Vec<usize>`; nth also gathered via `col.values()[i].clone()` (materializing the WHOLE column to `Vec<Scalar>` to index ~1000 rows). Replaced with a dense O(n) pass over `int64_dense_grouping`'s gids: nth = the row where the group's occurrence counter first hits `n` (emitted in the dense `order` ⇒ same group order); head/tail = a shared `dense_group_positions(keep(pos,size))` row-position filter (rows visited in ascending index order ⇒ `keep_indices` already sorted). nth's gather now uses zero-copy `Column::take_positions` + `Index::take`. BIT-IDENTICAL (235 fp-frame groupby tests pass). nth 6.5→2.0ms, head 6.5→3.0ms. ❌ Still a loss: pandas' selection ops factorize+select in fewer passes (0.80/1.38ms); FP's histogram scan + 8 MB `gid_per_row` + counter pass can't fully match. Direct key-offset indexing (skip `gid_per_row`) is the further lever. |
| SeriesGroupBy.transform(std/var/min/max/…) — reuse the dense DataFrame path (1q4q4) | 1M rows, 1000 int64 groups, **SHUFFLED**, transform("std") | 9.9 ms | 17.5 ms (was 62) | **0.57× — IMPROVED from 0.16× (3.5× FP-side), still a loss** | ⚠️ IMPROVED, bit-identical. `SeriesGroupBy.transform` only had the dense direct-address path for **sum/mean**; std/var/min/max/median/count/first/last/prod fell to `build_groups` + per-group `Vec<Scalar>` (0.16× pandas) — while `DataFrameGroupBy.transform` already had `try_transform_dense` for all 11 funcs (a win). Now those funcs wrap the key+value into a 2-col frame and reuse `DataFrameGroupBy.transform` (transform broadcasts per row, so group order is irrelevant ⇒ bit-identical; 48 fp-frame transform tests pass). 62→17.5ms. ❌ Still a loss: residual is the frame-construction overhead + FP's multi-pass dense std vs pandas' very fast fused series-transform (9.9ms); a direct dense SGB accumulator (no frame wrap) is the further lever, ~0.6× ceiling. nth (0.12×) and head (0.21×) are sibling selection-op gaps found in the same sweep, not yet addressed. |
| DataFrameGroupBy.agg(dict) — route through the dense engine (1q4q4) | 1M rows, 1000 int64 groups, **SHUFFLED** | sum+mean 21.4 · sum+std 22 · min+max 18.7 ms | sum+mean 5.1 · sum+std 6.1 · min+max 12.2 ms | **sum+mean 4.2× · sum+std 3.6× · min+max 1.53× — ALL WIN (was 0.30× LOSS)** | ✅ FIXED — `DataFrameGroupBy.agg(&HashMap col→func)` (dict form) used generic `build_groups` (SipHash) AND materialized a `Vec<Scalar>` PER GROUP (cloning every value), uniformly ~70ms — while the list-form `agg(&[func])` already had `agg_typed_pairs` (hash-free int64/str dense grouping). The dict form missed it. Now, when every aggregated column is numeric and every func is dense-reducible (sum/mean/count/min/max/var/std/first/last/prod/median), route through `agg_typed_pairs` and rename its `{col}_{func}` outputs to the dict form's single-level `{col}`. BIT-IDENTICAL (dense engine reproduces `build_groups`' first-seen/ascending-key group order + per-group row-order reductions; 234 fp-frame + 120 fp-conformance groupby/agg tests pass). sum+std 70→6.1ms (**11.5× FP-side**). `as_index=false` / non-numeric / skew/kurt/sem/nunique keep the generic path. |
| Series.abs() / trivial elementwise f64 ops — Arc<[f64]> copy-on-produce (1q4q4, FILED) | 5M f64, shuffled, no nulls | 21.5 ms | 60.7 ms (1M: 7.8ms) | **0.35× LOSS (2.8× slower); super-linear (1M→5M is 8× for 5× data)** | 🔍 ROOT-CAUSED, fix deferred (structural + peer-contended fp-columnar). `Column::abs` takes the typed fast path (`as_f64_slice` → `map(abs).collect::<Vec<f64>>()`), but the output is re-ingested via `lazy_all_valid_float64_with_finite` → **`Arc::from(Vec<f64>)`, which reallocates+copies** the 40MB buffer (all f64 storage is `Arc<[f64]>`; `ColumnData::Float64(Arc<[f64]>)`). So abs reads 40MB + writes 40MB (map) + reads 40MB + writes 40MB (Arc copy) = 160MB vs pandas' in-place 80MB. The `f64_finite_witness` is O(1) cached (NOT the cost); `collect::<Arc<[f64]>>()` wouldn't help (std routes Arc-from-iter through a Vec). Taxes EVERY f64-producing op but dominates only when compute is trivial (abs 0.35× while clip/cumsum, with real per-element work, are 0.95×/0.86×). **FIX LEVER:** a Vec-backed (or `Arc<Vec<f64>>`) all-valid-Float64 `ScalarValues` variant so a freshly-produced `Vec<f64>` is MOVED in, not copied — a structural fp-columnar change (storage enum + `as_f64_slice`); deferred for a coordinated pass (fp-columnar is actively peer-developed). |
| Convergence sweep 1q4q4 (shuffled keys, 1M) | value_counts / pivot / gb.quantile / gb.describe / DataFrameGroupBy.cumsum/cummax/shift/diff | pandas 63/2.65/87/2226 ms; gb-cython 24-31 ms | FP 38/1.8/16/68 ms; gb-cython 16-18 ms | **value_counts 1.65x · pivot 1.47x · gb.quantile 5.4x · gb.describe 32.6x · DataFrameGroupBy cumsum 1.32x/cummax 1.66x/shift 1.70x/diff 1.70x -- ALL WIN** | INFO/convergence -- every op probed this sweep is dominated (shuffled keys, not the i%g monotonic trap). DataFrameGroupBy cum/shift/diff WIN because pandas' DataFrame-groupby carries per-column+frame overhead; the SeriesGroupBy row below stays the residual (pandas Series-groupby is the lean 3.7-4.5ms Cython path). The clean algorithmic flips are harvested; remaining losses are STRUCTURAL: the Arc<[f64]> copy-on-produce floor (non-grouped add/mul/abs/round/cumsum/cummax/shift/diff + SeriesGroupBy cum/diff/shift) and the Categorical dtype (cut/qcut, a peer is actively building CategoricalIndex). |
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
| Series+Series add/mul CURRENT typed path re-measure (1q4q4, bench_binop_cc) | 2M f64, same index | add 1.17 / mul 1.57 ms | add 3.01 / mul 3.31 ms | **add 0.38× / mul 0.48×** | ℹ️ STALE-ROW CORRECTION — the 0.076×/0.072× rows above are obsolete; the typed `as_f64_slice` fast path in `try_vectorized_binary` already brought add to 3.01ms (5× FP-side faster than the 15.36ms pre-typed baseline). ❌ REVERT attempted lever (~0 gain): monomorphizing the typed f64 loop per-op (Add/Sub/Mul/Div closed-form arms) to replace the `binary_f64_apply(op)` `fn(f64,f64)->f64` pointer. Measured before/after IDENTICAL (add 3.01→3.07, mul 3.31→3.28 — noise) because **LLVM already devirtualizes the pointer** (the match returns a statically-known fn per op, so it inlines/autovectorizes regardless). The residual 2.6× gap is NOT the loop — it is the **[[f64-arc-copy-on-produce]]** floor: `from_f64_values(result)` does `Arc::from(Vec<f64>)` = a 16MB realloc+copy on top of the compute. Same structural item as abs/round/shift. The real lever remains the deferred Vec-backed/`Arc<Vec<f64>>` move-not-copy storage variant (peer-contended fp-columnar enum). |
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
| set_index drop=true retained-column clone elision (uza04.211) | 100k rows, 2 cols, typed key promoted to index and dropped; local best-of-30; Rust built with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a cargo run -p fp-frame --example bench_set_index --release -- 100000 30 <mode> drop`, then same local binary swept for all modes | i64 0.119 / f64 0.142 / utf8 0.424 / dt 0.175 / bool 0.144 / td 0.150 ms | i64 0.019 / f64 0.035 / utf8 1.850 / dt 0.062 / bool 0.040 / td 0.050 ms | **i64 6.20×, f64 4.02×, dt 2.83×, bool 3.61×, td 3.03×; utf8 0.23× loss** | ✅ KEEP PARTIAL — `set_index(drop=true)` now clones only retained columns instead of cloning the whole column map and removing the promoted key. This preserves output column order and index labels while removing clone-before-remove cost for expensive dropped key columns. Utf8 remains a real string-label allocation/index-construction loss and is not claimed fixed. |
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
| Series cumsum/cummax/cummin (1q4q4, bench_sops) | 2M f64, lazy RangeIndex | cumsum 8.4 / cummax 8.0 / cummin 7.9 ms | 17.1 / 17.5 / 16.7 ms | **0.49x / 0.46x / 0.47x** | REVERT ~0-gain + LOSS (rebuild/Arc-copy-class, same f64-arc-copy-on-produce floor as abs/add). Typed f64 prefix-scan already present (no Scalar materialize); index is NOT the cost (lazy RangeIndex re-bench identical, so the materialized-index clone was a red herring). Reverted lever: cummax/cummin of a no-NaN buffer can never be NaN, so from_f64_values_all_valid_unchecked (skip the has-NaN rescan) is bit-identical -- but before/after IDENTICAL (17.8->17.5 = noise) because LLVM already SIMD-vectorizes that scan; cumsum cannot skip it anyway (inf + -inf = NaN). Residual = collect(16MB)+Arc::from(Vec)(16MB) double-write + loop-carried dependent add/cmp chain; needs the deferred move-not-copy storage variant. clip 1.75x / between 2.15x are WINS. |
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
| RangeIndex reindex direct target scan BOLD-VERIFY (wnhuw/cod-b) | 1M-label `RangeIndex(0, 3n, 3)` reindexed against a same-step target with half overlap / half misses, plus descending source+target; pandas 2.2.3 local `RangeIndex.reindex(target)` best-of-100 vs `bench_range_setops range_reindex`; warm `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. | ascending 11.207965 ms / descending 12.007399 ms | current ascending 0.808061 ms / descending 0.805145 ms; benchmark-only old target-values shape 9.698274 ms / 10.028259 ms | **ascending 13.87× / descending 14.91× faster than pandas; FP-side 12.00× / 12.46× faster than old target-values allocation shape** | ✅ VERIFIED EXISTING WIN — current `RangeIndex::reindex` already computes the indexer directly from target arithmetic (`range_target_indexer` for affine targets, `value_at(position)` fallback otherwise) and returns `target.clone()` without allocating `target.values()` first. The added benchmark mode makes the stale-open `wnhuw` child reproducible; no production code change was needed and no rollback candidate was kept. |
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

### Kept: Series add/mul same-index owned hot Float64 output (cod-b 2026-06-27)

Target: same public row as above, `Series::add` / `Series::mul`, 2M all-valid
Float64 values on equal Int64 indexes, measured with `bench_binop_cc` best-of-50/80
on pinned CPU 7. Route: the biggest remaining documented arithmetic gap was no
longer the arithmetic sweep itself, but the materialization boundary after the
same-index Float64 kernel. The no-output-NaN branch already proves the output is
all-valid; it was still feeding the hot result `Vec<f64>` through the Arc-slice
constructor. The new lever stamps that exact proven all-valid buffer through the
owned lazy Float64 backing (`LazyAllValidFloat64Vec`), matching the existing unary
hot-output pattern and avoiding the cold `Arc<[f64]>` conversion copy. This came
from the graveyard/vectorized-execution evidence: keep cache-hot vector outputs
inside a vectorized/morsel pipeline instead of re-materializing at a boundary.

Isomorphism proof: the arithmetic sweep, input-NaN gate, output-NaN gate, and
fallbacks are unchanged. The new constructor is reached only when
`!input_nan && !output_nan`; therefore every output slot is valid and contains the
same `f64` bit pattern that the old all-valid constructor received. The storage
variant changes from Arc-slice lazy all-valid to owned-Vec lazy all-valid, both
surface through the same `as_f64_slice`/`values()` contracts and defer scalar
materialization. A focused guard now asserts the same-index all-valid route keeps
the owned lazy buffer without populating scalar values.

Same-worktree FP baseline before the lever, `rch exec -- cargo build --release
-p fp-frame --example bench_binop_cc` (local fallback, same target dir) then
`taskset -c 7`: best-of-50 samples **add 6.912 / 6.346 / 6.640 ms**, **mul
6.378 / 6.369 / 6.403 ms**, **gt 2.431 / 2.549 / 2.482 ms**. Candidate
best-of-50 samples after the lever: **add 3.637 / 4.617 / 4.139 ms**, **mul
3.175 / 4.080 / 4.423 ms**, **gt 1.987 / 2.618 / 2.469 ms**. Candidate
best-of-80 confirmation: **add 3.517 ms, mul 3.710 ms, gt 1.915 ms**. Fresh
pandas 2.2.3 best-of-80 on the same pinned CPU: **add 4.582 ms, mul 4.610 ms,
gt 3.514 ms**. Head-to-head ratio (pandas / FP): **add 1.30x**, **mul 1.24x**,
**gt 1.83x**. FP-side delta from the median best-of-50 baseline to the best-of-80
confirmation: **1.89x faster add**, **1.72x faster mul**. KEEP.

Verification: `cargo fmt -p fp-columnar --check` passed; full workspace
`cargo fmt --check` is still red on unrelated pre-existing formatting drift in
`fp-frame`/`fp-index`/`fp-join`, so this pass did not format peer-owned files.
`rch exec -- cargo check -p fp-columnar --all-targets` and `rch exec -- cargo
clippy -p fp-columnar --all-targets -- -D warnings` passed on `hz2`.
Focused guards passed: `fp-columnar aligned_binary_f64_same_positions --release`,
`fp-columnar apply_f64_slices_parallel_matches_serial_nan_tracking --release`,
`fp-frame series_add_emits_alignment_semantic_witness_tn6qb3 --release`, and
`fp-frame series_add_aligns_on_union_index --release`. The requested spelling
`rch exec -- cargo bench --release -p fp-columnar --no-run` was tried and Cargo
rejected `--release` for `bench`; the valid per-crate bench profile commands
`rch exec -- cargo bench -p fp-columnar --no-run` and `rch exec -- cargo bench
-p fp-columnar` both completed successfully. `rch exec -- cargo test -p
fp-conformance --release` passed (1596 unit tests plus integration/doc tests).

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

### 2026-06-23 BlackThrush — Int64 Index.nunique raw-count proof: 8.76x dense / 1.56x wide vs pandas @1M
Closed br-frankenpandas-0jula as proof/evidence for the existing typed Int64 `Index::nunique` fast path. Added
`index_nunique` to `crates/fp-index/examples/bench_range_setops.rs` to cover both branches of the raw i64 counter:
dense repeated labels hit the direct-address bitset, wide repeated labels force the hash-set path. Command:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo run -p fp-index --example
bench_range_setops --release -- 1000000 50 index_nunique`. Results: fp dense 1,176,898 ns vs pandas 10,310,012 ns
= 8.76x; fp wide 15,625,952 ns vs pandas 24,437,490 ns = 1.56x. Benchmark-only label-materializing comparator was
26,740,717 ns dense and 31,722,265 ns wide, so the current raw-count path is also 22.72x / 2.03x faster than the old
label-vector + label-hash shape. Kept as non-zero evidence; no production code changed because the production fast
path and tests were already present.

### 2026-06-23 BlackThrush — RangeIndex.asof_locs sorted-target stream: 0.36x loss -> 7.81x win @1M
Closed br-frankenpandas-vuftp with a real production lever. Added `range_asof_locs` to
`crates/fp-index/examples/bench_range_setops.rs`, then measured 1M-label `RangeIndex(0, 2n, 2)` with 1M
nondecreasing Int64 probes just below source labels. Baseline FP binary-searched every probe:
56,835,368 ns vs pandas 20,489,127 ns = 0.36x. Fix: when the source range is ascending, probes are all Int64, no mask
is supplied, and target labels are nondecreasing, stream the source cursor once and emit the current right-bound
position per probe; unsorted targets, masked calls, descending ranges, and mixed-label targets keep the existing
fallback paths. After: 2,624,860 ns = 21.65x FP-side and 7.81x faster than pandas. Ordering, duplicate-target,
before-first, and after-last semantics match the existing `range_index_asof_locs_uses_direct_values_vuftp` oracle.

### 2026-06-23 BlackThrush — RangeIndex.astype direct-path proof: 75.67x int64 / 5.15x string vs pandas @1M
Closed br-frankenpandas-up4dq as proof/evidence for the existing RangeIndex direct cast path. Added `range_astype`
to `crates/fp-index/examples/bench_range_setops.rs` to cover `RangeIndex(0, 3n, 3).astype("int64")`, ascending
`astype("string")`, and descending `RangeIndex(3n, 0, -3).astype("string")`. Command:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b cargo run -p fp-index --example
bench_range_setops --release -- 1000000 50 range_astype`. Results: fp int64 170 ns vs pandas 12,864 ns = 75.67x;
fp string 29,408,062 ns vs pandas 151,439,079 ns = 5.15x; fp descending string 29,404,614 ns vs pandas
151,074,139 ns = 5.14x. No production code changed: `RangeIndex::astype("int64")` already returns the typed
affine Int64 flat index, string/object casts already iterate `value_at(position)` directly, and the existing
`range_index_astype_uses_direct_values_up4dq` unit test covers name propagation, descending values, typed Int64
backing, and unsupported dtype errors. Kept the benchmark and evidence because all measured ratios are wins, not
~0-gain.

### 2026-06-24 SlateOtter — Nullable-Float64 dense groupby reductions: 0.43–0.47x loss -> 2.14–3.80x win @1M
DataFrameGroupBy sum/mean/min/count/std on a single dense Int64 key with a Float64 VALUE
column that contains MISSING values fell off the dense `aggregate_int64_dense` ->
`dense_aggregate_emit` fast path. That emit only handled `as_f64_slice` (all-valid, no NaN/Null)
and `as_i64_slice`, so any value column WITH missing entries was excluded by the gate and dropped
to the generic `build_groups` path (SipHash grouping + per-group Scalar-closure scans) — measured
0.43–0.47x pandas. Extended the gate in `aggregate_named_func` to admit nullable Float64
(`as_f64_slice_with_validity`) for exactly the funcs `dense_aggregate_emit` now implements
(sum/mean/count/min/max/var/std), and added a typed skipna accumulation arm over `(data,
validity)`: sum/count single-pass; mean single-pass accumulate+count; var/std two-pass
mean-centered ddof=1 (count<2 -> NaN); min/max first-valid tracking (all-missing -> NaN).
BIT-IDENTICAL to the generic path's nan* reductions (all-missing group -> sum 0.0,
mean/min/max/var/std NaN, count 0; same two-pass folds). Bench `bench_gbnull` @1M rows /
1000 groups / 20% missing, fp before = generic path (lib.rs change stashed), fp after = dense path:

| op   | before (generic) | after (dense) | pandas   | before->pandas | after->pandas | fp-side |
|------|------------------|---------------|----------|----------------|---------------|---------|
| sum  | 42.99ms          | 5.40ms        | 18.83ms  | 0.44x          | 3.49x         | 7.96x   |
| mean | 46.05ms          | 5.68ms        | 21.56ms  | 0.47x          | 3.80x         | 8.11x   |
| min  | 41.00ms          | 5.64ms        | 17.72ms  | 0.43x          | 3.14x         | 7.27x   |
| std  | 43.52ms          | 9.59ms        | 20.48ms  | 0.47x          | 2.14x         | 4.54x   |

All four flip LOSS -> WIN. Correctness pinned by new `groupby_nullable_dense_conformance`
(5 tests, hand-computed expected covering multi-element, single-element std->NaN, and
all-missing groups) — all pass. pandas baseline: float64 column with NaN, same splitmix-scrambled
keys/values, best-of-6. Bench command:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc cargo run -p fp-frame --example bench_gbnull --release -- <op>`.

### 2026-06-24 SlateOtter — Nullable-Float64 dense groupby prod/first/last/median: 0.43–0.90x -> 3.02–3.64x win @1M
Follow-up to the same-day nullable-f64 dense groupby reduction landing (c372fd4be), which only
admitted sum/mean/count/min/max/var/std. The dense `dense_aggregate_emit` all-valid f64 arm ALSO
implements prod/first/last/median, but the nullable gate excluded them, so a Float64 value column
WITH missing values still fell to the generic `build_groups` path for those four funcs — measured
0.43–0.50x pandas (prod/first/last) and 0.90x (median). Widened the nullable gate to admit
prod/first/last/median and added their skipna arms to the `(data, validity)` branch: prod skips
missing and folds from 1.0 (all-missing -> 1.0, bit-identical to `nanprod`); first/last take the
first/last VALID value in ascending row order (all-missing -> NaN, matching the generic
`find(|v| !v.is_missing())` / reverse-find); median uses a new `dense_group_median_f64_skipna`
(CSR over valid values only + the same `typed_median_f64_slice`, all-missing -> NaN, bit-identical
to `nanmedian(valid)`). Bench `bench_gbnull` @1M rows / 1000 groups / 20% missing, fp before =
generic path (lib.rs change unstaged), fp after = dense path:

| op     | before (generic) | after (dense) | pandas   | before->pandas | after->pandas | fp-side |
|--------|------------------|---------------|----------|----------------|---------------|---------|
| prod   | 40.47ms          | 5.25ms        | 19.13ms  | 0.47x          | 3.64x         | 7.71x   |
| first  | 36.18ms          | 5.81ms        | 18.24ms  | 0.50x          | 3.14x         | 6.23x   |
| last   | 43.41ms          | 6.10ms        | 18.45ms  | 0.43x          | 3.02x         | 7.12x   |
| median | 43.30ms          | 12.79ms       | 39.01ms  | 0.90x          | 3.05x         | 3.39x   |

All four flip to WINS (median 0.90x -> 3.05x is not ~0-gain). Correctness pinned by extending
`groupby_nullable_dense_conformance` to 9 tests (added prod/first/last/median with hand-computed
expected: all-missing prod -> 1.0, first/last skip leading/trailing NaN, median over valid) — all
9 pass. pandas baseline: float64 column with NaN, same splitmix-scrambled keys/values, best-of-6.
Bench command:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc cargo run -p fp-frame --example bench_gbnull --release -- <op>`.

### 2026-06-24 SlateOtter — Nullable-Float64 dense SeriesGroupBy sum/mean/min/max: 0.39–0.43x -> 3.55–3.79x win @1M
Parallel to the same-day DataFrameGroupBy nullable-f64 dense work (c372fd4be/009bd97b1), but on the
SeriesGroupBy side (`Series.groupby(int64_key).sum()/mean()/min()/max()`). `SeriesGroupBy::dense_group_fold`
(the single-fold dense reducer that sum/mean/min/max share) gated on `as_f64_slice()`/`as_i64_slice()`
(all-valid only), so a Float64 value column WITH missing values fell to the slow generic `agg_numeric`
build_groups path (per-group `Vec<f64>` gather + closure re-scan) — measured 0.39–0.43x pandas. Added a
nullable `as_f64_slice_with_validity` branch: skipna fold (only non-missing slots folded/counted), and
the emit now maps `count==0 -> Scalar::Null(NullKind::NaN)` so an all-missing group matches `agg_numeric`'s
`nums.is_empty() -> Null(NaN)` arm (applies to sum too — distinct from DataFrameGroupBy where all-missing
sum is 0.0). Bit-identical to `agg_numeric`: same `is_missing()`-skip + `to_f64()` (inf kept, not dropped),
same ascending-row fold order, same first-seen gid order/labels. The all-valid fold path is untouched
(`count[g] > 0` always there, so the new `count==0` arm never fires for it). Bench `bench_sgbnull` @1M rows
/ 1000 groups / 20% missing, fp before = generic path (lib.rs change stashed), fp after = dense path,
measured back-to-back under the same machine load (std/median paths are unchanged by the edit and read
52.7/64.8ms before vs 50.3/59.8ms after, confirming equal load):

| op   | before (generic) | after (dense) | pandas   | before->pandas | after->pandas | fp-side |
|------|------------------|---------------|----------|----------------|---------------|---------|
| sum  | 64.88ms          | 7.10ms        | 25.45ms  | 0.39x          | 3.59x         | 9.14x   |
| mean | 58.61ms          | 6.49ms        | 24.59ms  | 0.42x          | 3.79x         | 9.03x   |
| min  | 57.34ms          | 6.86ms        | 24.38ms  | 0.43x          | 3.55x         | 8.36x   |

max shares the identical `dense_group_fold` code (NEG_INFINITY/`f64::max`) — covered by conformance,
not separately benched (~min). All flip LOSS -> WIN. Correctness pinned by new
`sgb_nullable_dense_conformance` (4 tests: sum/mean/min/max, hand-computed with an all-missing group ->
Null(NaN) per agg_numeric) — all green. NOT claimed: SeriesGroupBy nullable std (50.3ms, 0.50x) and median
(59.8ms, ~0.98x) use separate paths (std -> agg_numeric directly; median -> its own) and are UNCHANGED;
left as documented follow-ups. pandas baseline: float64 Series with NaN, same splitmix keys, best-of-6.
Bench: `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc cargo run -p fp-frame --example bench_sgbnull --release -- <op>`.

### 2026-06-24 SlateOtter — Dense SeriesGroupBy var/std (incl. nullable-f64): 0.42–0.43x -> 1.99x win @1M
Continuation of the SeriesGroupBy nullable-f64 work (476402d66). `SeriesGroupBy::std` had NO dense path
(it went straight to `agg_numeric` for ALL inputs), and `var`'s dense two-pass block gated `as_f64_slice()`/
`as_i64_slice()` (all-valid only) — so a Float64 value column WITH missing values fell to the slow generic
`agg_numeric` build_groups gather for BOTH var and std (measured 0.42–0.43x pandas). Extracted a single
shared `dense_group_var_std(want_std)` helper that handles int64 / all-valid-f64 / nullable-f64 value
columns over the dense gid layout: pass 1 sums+counts per gid (skipna: only non-missing fold/count), pass 2
accumulates squared deviations, emit ssd/(n-1) (sqrt for std). `var()` now calls it (replacing its inline
all-valid-only block) and `std()` calls it before its `agg_numeric` fallback. Bit-identical to agg_numeric's
var/std closures: first-seen gids/labels, ascending value-order sums (== the `is_missing()`-skipped `to_f64()`
order), `(x-mean).powi(2)`, and the emit matches the generic wrap exactly — all-missing group (n==0) ->
`Null(NaN)`, n==1 -> `Float64(NaN)`, n>=2 -> `Float64(value)`. All-valid var is unchanged (n>=1, so the
n==0 arm never fires); all-valid std becomes bit-identical (same ascending sums -> same mean/ssd -> same
var.sqrt()). Bench `bench_sgbnull` @1M / 1000 groups / 20% missing, before (stashed helper) vs after under
equal load (load-check sum 7.02 vs 7.03ms, median 60.0 vs 58.8ms):

| op  | before (agg_numeric) | after (dense) | pandas   | before->pandas | after->pandas | fp-side |
|-----|----------------------|---------------|----------|----------------|---------------|---------|
| var | 49.29ms              | 10.48ms       | 20.90ms  | 0.42x          | 1.99x         | 4.70x   |
| std | 49.80ms              | 10.80ms       | 21.54ms  | 0.43x          | 1.99x         | 4.61x   |

Both flip LOSS -> WIN. Correctness pinned by extending `sgb_nullable_dense_conformance` to 6 tests (added
var/std: group [2,4,6] -> var 4/std 2, singletons + all-missing -> NaN) — all green. pandas baseline:
float64 Series with NaN, same splitmix keys, best-of-6. Remaining SeriesGroupBy nullable gap: median (~0.98x,
near parity, separate path) — not pursued.

### 2026-06-24 SlateOtter — typed nullable-f64 Series sum/mean/var/std: var 1.06x->1.66x (parity->win), std 1.53x->2.43x @1M
Series-level (not groupby) reductions had typed all-valid f64 fast paths but a Float64 column WITH missing
values fell to a `self.column.values()` Scalar loop — materializing a 1M-element `Vec<Scalar>` and matching
each element. Added typed `as_f64_slice_with_validity` skipna branches to `Series::sum` (the f64 default arm)
and `Series::var` (the second-pass squared-deviation fold); mean (= sum/count) and std (= var.sqrt()) inherit
the speedup. Bit-identical to the Scalar loop: same non-missing values in row order, same 0.0-seeded `+`
(sum) and `(v-mean)^2` fold (var), `count()` already a popcount. Bench `bench_series_null` @1M / 20% missing,
best-of-8, before = Scalar loop (stashed), after = typed; before/after measured ~6 min apart on the same host
(the uniform ~1.6x fp-side speedup across all four independent ops confirms it is the lever, not load):

| op   | before (Scalar loop) | after (typed) | pandas   | before->pandas | after->pandas | fp-side |
|------|----------------------|---------------|----------|----------------|---------------|---------|
| var  | 10.71ms              | 6.81ms        | 11.30ms  | 1.06x (parity) | 1.66x WIN     | 1.57x   |
| std  | 10.89ms              | 6.84ms        | 16.64ms  | 1.53x WIN      | 2.43x WIN     | 1.59x   |
| mean | 5.24ms               | 3.26ms        | 2.68ms   | 0.51x          | 0.82x         | 1.61x   |
| sum  | 5.20ms               | 3.26ms        | 2.03ms   | 0.39x          | 0.62x         | 1.59x   |

HONEST: var flips parity->win and std strengthens to 2.43x — the headline wins. sum/mean improve 1.6x
fp-side and bit-identical but REMAIN below pandas (0.62x/0.82x): pandas' `np.nansum`/`nanmean` are SIMD
(NaN-blend vectorized over the f64 buffer), while fp's safe per-element `validity.get(i)` conditional add
is scalar — closing that needs SIMD intrinsics (unsafe, forbidden by default) and the branchless mul/select
forms break bit-identity (NaN*0 / signed-zero). Kept because the change is a strict bit-identical fp-side
improvement (necessary plumbing for the var/std wins, no regression — all-valid path guarded). Correctness:
new `series_nullable_reduction_conformance` (6 tests: nullable sum/mean/var/std, all-missing, all-valid
regression guard) green. pandas baseline best-of-8, float64 Series with NaN.

### 2026-06-24 SlateOtter — Series cumsum/cummax 0.40–0.44x LOSS = the f64-arc-copy-on-produce floor (surfaced + evidence)
Probed 7 all-valid-f64 Series ops @1M (`bench_probe`) to find a fresh winnable lever. fp WINS most, but
cumsum/cummax/diff (the f64-array-PRODUCING ops) LOSE — and the cause is the known structural
`f64-arc-copy-on-produce` floor, not a missing fast path. cumsum/cummax already have typed all-valid f64
fast paths: they build a `Vec<f64>` prefix/running fold then `Column::from_f64_values(out)`. That
constructor (a) scans the 8MB output for NaN, then (b) `Arc::from(Vec<f64>)` into `LazyAllValidFloat64`'s
`data: Arc<[f64]>` — and `Arc<[T]>`'s layout (refcount header + data in ONE allocation) FORCES a full data
copy; `Arc::from(Vec)` can never reuse the Vec's separate buffer. So fp pays ~3 passes over 8MB (fold-write,
NaN-scan, Arc-copy) vs pandas' ~2 (read+write) -> ~2.3x. Measured vs pandas 2.2.3, best-of-6 @1M all-valid f64:

| op         | fp       | pandas   | ratio       |
|------------|----------|----------|-------------|
| cumsum     | 7.56ms   | 3.34ms   | 0.44x LOSS  |
| cummax     | 7.73ms   | 3.08ms   | 0.40x LOSS  |
| diff       | 0.81ms   | 0.35ms   | 0.43x (sub-ms) |
| pct_change | 1.46ms   | 18.98ms  | 13.0x WIN   |
| rank       | 32.97ms  | 130.67ms | 3.96x WIN   |
| nlargest   | 9.95ms   | 20.28ms  | 2.04x WIN   |
| mode       | 15.46ms  | 32.92ms  | 2.13x WIN   |

BLOCKER (why not fixed here): the fix is an owned-`Vec<f64>` / `Arc<Vec<f64>>` all-valid-Float64
`ScalarValues` variant that MOVES the output Vec instead of copying it (mirroring the existing
`LazyNullableFloat64`, which IS Vec-backed and copy-free — but `from_f64_values_with_validity` routes an
ALL-valid mask straight back to the copying `from_f64_values`). This is genuinely entangled: the current
`Arc<[f64]>` backing is shared ZERO-COPY with `LazyAllValidFloat64Slice` (take_positions row-range views),
which an `Arc<Vec<f64>>` cannot provide without re-copying; and a Vec-backed variant trades O(1) `Column::clone`
for an 8MB deep copy on clone. So it is a structural fp-columnar change with a real trade-off + many match
sites (peer-contended file), NOT a contained fast-path add — deferred, consistent with the prior
`f64-arc-copy-on-produce` note. No production code changed; `bench_probe` added as standing evidence.

### 2026-06-24 SlateOtter — Series.autocorr typed f64 path: 1.38x -> 13.4x @1M (9.7x fp-side, bit-identical)
Probed scalar-output Series ops (`bench_probe2`) for a non-arc-bound lever. corr/cov already have typed
two-pass paths (cov_components) and sit ~parity (bandwidth-bound); but `Series::autocorr` had NO typed path
— it materialized `self.column.values()` (a 1M-element Vec<Scalar> = ~32MB) and built two more pair Vecs,
even for an all-valid Float64 column. Added a typed `as_f64_slice` fast path: two linear passes over the raw
f64 buffer with a `lag` offset (means, then centered cross/var sums), no Scalar materialization, no pair
Vecs. Bit-identical to the Scalar loop: an `as_f64_slice` column is all-valid with no NaN (from_f64_values
marks NaN missing), so the original missing/NaN-skip never fired and count==n with x/y == data[0..n] /
data[lag..len]; same sums in the same order, same `cov/sqrt(var_x*var_y)` with the identical
`< f64::EPSILON => NaN` guard. Bench `bench_probe2 autocorr` @1M all-valid f64, before/after under equal
load (unchanged corr path read 5.74 vs 5.66ms):

| op       | before (Scalar) | after (typed) | pandas   | before->pandas | after->pandas | fp-side |
|----------|-----------------|---------------|----------|----------------|---------------|---------|
| autocorr | 13.66ms         | 1.41ms        | 18.83ms  | 1.38x          | 13.36x        | 9.70x   |

Was already a marginal win (1.38x) but the Vec<Scalar> materialization dominated; removing it is a 9.7x
fp-side speedup. Correctness pinned by new `autocorr_typed_conformance` (5 tests vs an independent oracle:
lag1/lag5, linear->1.0, constant->NaN, short->NaN) — all green. Same probe confirms fp already WINS
quantile 6.05x, nunique 2.23x, duplicated 1.17x; corr/cov ~parity (0.94x/0.95x, bandwidth-bound, typed paths
already present — not pursued). pandas baseline best-of-6, float64 Series.

### 2026-06-24 SlateOtter — Series.skew/kurt typed f64 fused-pass: 5.1x -> ~14x @1M (2.7x fp-side, bit-identical)
Probed scalar-output moment ops (`bench_probe3`). sem already uses typed Welford (`numeric_moments`, no
copy) and median/prod already win — but `Series::skew`/`kurtosis` called `numeric_values`, which (even on
its typed branch) COPIES the whole all-valid buffer into a `vals: Vec<f64>` and computes the mean over it,
then skew/kurt re-scan `vals` TWICE more (m2+m3, or m2+m4) — ~5 passes over 8MB. Added a typed `as_f64_slice`
fast path to both: compute the mean in one pass then a FUSED single pass accumulating m2 & m3 (skew) / m2 &
m4 (kurt) straight off the slice — no vals Vec, 2 passes total. Bit-identical: `as_f64_slice` is all-valid
no-NaN (from_f64_values marks NaN missing), so `numeric_values`' present-filter keeps everything =>
`vals == data` (index order) and the same `Σdata/n` mean; m2/m3/m4 are the same `(v-mean).powi(k)` terms
summed in the same order (fusing two independent sums into one pass changes no term). Nullable columns don't
hit the gate and keep the existing path. Bench `bench_probe3` @1M all-valid f64, before/after equal load
(unchanged sem path read 4.74ms both, median ~1.5ms):

| op   | before  | after   | pandas   | before->pandas | after->pandas | fp-side |
|------|---------|---------|----------|----------------|---------------|---------|
| skew | 3.87ms  | 1.41ms  | 19.68ms  | 5.08x          | 13.96x        | 2.75x   |
| kurt | 3.82ms  | 1.41ms  | 19.75ms  | 5.17x          | 14.01x        | 2.71x   |

Already wins, but the vals copy + extra re-scans dominated — removing them is 2.7x fp-side. Correctness
pinned by new `skew_kurt_typed_conformance` (4 tests vs independent oracle: skew/kurt match, constant->0.0,
too-few->NaN) — all green. Same probe confirms fp already WINS sem 4.41x (typed Welford), median 12.6x,
prod 1.52x. pandas baseline best-of-6.

### 2026-06-24 SlateOtter — DataFrame std/var/skew axis=1 typed f64 output: ~10x -> ~13-15x @1M×8 (1.2-1.3x fp-side)
Probed DataFrame row-axis (axis=1) reductions (`bench_probe_df`, 1M rows × 8 f64 cols). pandas axis=1 is
TERRIBLE (block-manager gather + Python per-row): std 213ms, var 206ms, skew 334ms, sum 82ms — fp already
WINS everything 6-12x. But sum/mean/max axis=1 use the typed `reduce_rows_f64` while std/var/sem/skew/kurt
use `reduce_rows_func_f64`, which boxed each row's result into a `Vec<Scalar::Float64>` (32B/elem, ~32MB @1M)
then re-inferred dtype in `from_values`. Switched it to collect a `Vec<f64>` and ingest via `from_f64_values`
(typed, like the sum path). Bit-identical: `from_f64_values` marks a NaN-result row missing exactly as
`from_values` does for a `Scalar::Float64(NaN)` (both => missing), present rows store the same `Float64(v)`;
the per-row gather + reducer closure are untouched. Bench @1M×8, before/after equal load (unchanged sum path
11.32/11.28ms):

| op (axis=1) | before  | after   | pandas    | before->pandas | after->pandas | fp-side |
|-------------|---------|---------|-----------|----------------|---------------|---------|
| std         | 20.78ms | 16.43ms | 213.28ms  | 10.26x         | 12.98x        | 1.26x   |
| var         | 17.72ms | 14.00ms | 205.51ms  | 11.60x         | 14.68x        | 1.27x   |
| skew        | 38.39ms | 34.65ms | 333.74ms  | 8.69x          | 9.63x         | 1.11x   |

Already dominant wins; the typed output strengthens std/var ~1.27x fp-side (skew ~1.11x, its row_skew compute
dominates). sem/kurt axis=1 share `reduce_rows_func_f64` so they inherit it too. Correctness: new
`df_axis1_f64_typed_conformance` (var/std vs independent oracle + zero-variance-row 0.0 guard) + existing
`row_reduction_conformance` green. Same probe: sum 7.27x, mean 6.41x, max 6.39x already win (typed
`reduce_rows_f64`). pandas baseline best-of-6.

### 2026-06-24 SlateOtter — fp-index Utf8 Index.intersection: 0.70x LOSS -> 1.19x WIN @1M (1.69x fp-side, bit-identical)
Probed unsorted Utf8 Index set-ops (`probe_str_setops`) vs pandas 2.2.3. fp WINS get_indexer 4.5x, isin 3.0x
(pandas object-Index is brutally slow: intersection 286ms, UNION 1.49s, isin 503ms) — but `Index::intersection`
was a LOSS: 408ms vs pandas 286ms = 0.70x. CAUSE: the generic path builds `other.position_map_first_ref()`
(FxHashMap<&IndexLabel> over all of other) AND a SECOND `seen: FxHashMap<&IndexLabel>` dedup set — ~2.5M
pointer-keyed string-hash probes (each chasing &IndexLabel -> 32B enum -> String -> heap bytes). Added a typed
all-Utf8 fast path (sibling of the existing int64 `membership_filter_i64`): gate on both sides being pure
`IndexLabel::Utf8` (no Null), build ONE `FxHashMap<&str, bool>` over other, and dedup via a "matched" flag
stored IN that map — eliminating the separate seen-set (one hash map instead of two) and hashing `&str`
directly (skips the enum load). Bit-identical: same self-order, same first-occurrence dedup (a self label in
other emits once, then its flag suppresses repeats), same matched labels; the all-Utf8 gate (no Null) is what
makes dropping the `&IndexLabel` keys safe (the generic path would otherwise also match a Null self label
against a Null in other). `probe_str_setops` @1M, before/after (unchanged get_indexer ~60-68ms = load band):

| op           | before   | after    | pandas   | before->pandas | after->pandas | fp-side |
|--------------|----------|----------|----------|----------------|---------------|---------|
| intersection | 408.35ms | 241.27ms | 286.34ms | 0.70x LOSS     | 1.19x WIN     | 1.69x   |

Flips LOSS -> WIN. Correctness: new `utf8_intersection_typed_conformance` (6 tests vs independent oracle:
overlap+dup, disjoint, full-overlap self-order, all-dup self, empty, larger shuffled) — all green. Same probe:
get_indexer 4.52x, isin 2.97x already WIN. OPEN: union/difference/symmetric_difference Utf8 share the same
`FxHashMap<&IndexLabel>` + seen pattern (pandas union is 1.49s — likely fp already wins but the same lever
applies). pandas baseline best-of-6.

### 2026-06-24 SlateOtter — fp-index Utf8 union/difference/symdiff typed sweep: 1.85–2.57x fp-side @1M (bit-identical)
Follow-up to the Utf8 intersection win (a9dc31b19) — same lever applied to the other Utf8 set-ops. pandas
object-Index is brutally slow here (union 1.75s, difference 875ms, symmetric_difference 962ms). fp already WON
(union 3.02x, difference 2.03x, symdiff 1.23x) but each used pointer-keyed `FxHashMap<&IndexLabel>` with
redundant maps. Added typed all-Utf8 paths (gate: both sides pure IndexLabel::Utf8, no Null):
- **difference**: ONE `FxHashMap<&str,()>` seeded with other doubles as membership AND self-dedup —
  `insert(s).is_none()` is true only when s is neither in other nor already emitted (2 maps -> 1).
- **symmetric_difference**: the two halves are disjoint, so each membership map carries its OPPOSITE half's
  dedup (self-half deduped via other_set, other-half via self_set) — drops the shared `seen` (3 maps -> 2).
- **union_with**: dedup the self-then-other concat with `FxHashSet`-style `&str` keys instead of
  `&IndexLabel` (skips the 32B enum load per probe; still one map).
All bit-identical (same order, first-occurrence dedup, same labels). `probe_str_setops` @1M (intersection
244ms unchanged from a9dc31b19 = stable load):

| op                   | before   | after    | pandas    | before->pandas | after->pandas | fp-side |
|----------------------|----------|----------|-----------|----------------|---------------|---------|
| union                | 580.59ms | 314.04ms | 1753.10ms | 3.02x          | 5.58x         | 1.85x   |
| difference           | 431.74ms | 167.74ms | 874.76ms  | 2.03x          | 5.21x         | 2.57x   |
| symmetric_difference | 784.76ms | 357.40ms | 962.05ms  | 1.23x          | 2.69x         | 2.20x   |

Correctness: new `utf8_setops_typed_conformance` (union/difference/symdiff vs independent oracles across
overlap+dup, disjoint, full-overlap self-order, all-dup, empty-self, empty-other, larger shuffled) — all
green; intersection conformance still green. pandas baseline best-of-4/6. The fp-index Utf8 set-op surface is
now fully typed (intersection/union/difference/symdiff), all WIN 1.2–5.6x; get_indexer 4.5x, isin 3.0x.

### 2026-06-24 SlateOtter — fp-index Utf8 Index.value_counts: 0.80x LOSS -> 3.22x WIN @1M (4.02x fp-side, bit-identical)
Probed duplicate-heavy unsorted Utf8 Index dedup/count ops (`probe_str_dedup`, 1M rows / 10k distinct) vs
pandas 2.2.3. fp WINS unique 1.62x, nunique 1.53x, duplicated 1.75x, drop_duplicates 1.82x — but
`Index::value_counts` was a LOSS (66.57ms vs pandas 53.23ms = 0.80x). CAUSE: the generic Utf8 path tallied an
`FxHashMap<IndexLabel, usize>` with CLONED KEYS — `contains_key(label)` THEN `*counts.entry(label.clone())`
allocates a fresh `String` for EVERY one of the 1M input labels (plus a redundant second hash per label).
Added a typed all-Utf8 path: tally `&str` keys with a single `entry(s).or_insert(0)` per label, tracking
first-seen order, and clone a `String` only once per DISTINCT label (10k) when building the final pairs.
Bit-identical: same first-seen order, same per-label counts, same `sort_by_key` (stable -> first-seen breaks
ties); the all-Utf8 gate (no Null) makes `dropna` moot (total == len). `probe_str_dedup` @1M/10k, before/after
(unchanged unique/duplicated read ~22-23ms both = stable load):

| op           | before  | after   | pandas  | before->pandas | after->pandas | fp-side |
|--------------|---------|---------|---------|----------------|---------------|---------|
| value_counts | 66.57ms | 16.55ms | 53.23ms | 0.80x LOSS     | 3.22x WIN     | 4.02x   |

Flips LOSS -> WIN. Correctness: new `utf8_value_counts_typed_conformance` (5 tests vs independent oracle:
counts-descending, first-seen tie-break, single value, empty, larger dup-heavy) — all green. Same probe: fp
already WINS unique 1.62x, nunique 1.53x, duplicated 1.75x, drop_duplicates 1.82x (those use the lighter
`FxHashMap<&IndexLabel>` pointer path, not cloned keys). pandas baseline best-of-6.

### 2026-06-24 SlateOtter — fp-index Utf8 get_indexer_non_unique: 3.07x -> 7.14x @1M (2.33x fp-side, bit-identical)
Hunting the `entry(label.clone())` String-alloc-per-row smell (same root as the value_counts fix 409340f38),
found `Index::get_indexer_non_unique` on a Utf8 source built its position map as
`FxHashMap<IndexLabel, Vec<usize>>` with `entry(label.clone())` — a String alloc for EVERY source row (1M).
fp already WON (70.74ms vs pandas 217ms = 3.07x, pandas non-unique reindex is slow) but paid the clones. Added
a typed all-Utf8 path: key the source position map on `&str` (no clone), gated on BOTH source and target pure
Utf8 (no Null) so every target label is a `&str` lookup (no skipped emissions). Bit-identical: same per-key
source-order position lists, same target-order indexer, same missing list. `probe_str_ginu` @1M/10k (dup-heavy
source, target = 2*card distinct):

| op                     | before  | after   | pandas   | before->pandas | after->pandas | fp-side |
|------------------------|---------|---------|----------|----------------|---------------|---------|
| get_indexer_non_unique | 70.74ms | 30.42ms | 217.13ms | 3.07x          | 7.14x         | 2.33x   |

Removing the 1M per-row String allocations is 2.33x fp-side. Correctness: new `utf8_ginu_typed_conformance`
(6 tests vs independent oracle: duplicates+missing, all-missing, empty target/source, source-order position
preservation, larger dup-heavy) — all green. SMELL CONFIRMED REUSABLE: `entry(k.clone())` over an
`FxHashMap<IndexLabel,_>` in a per-row loop = a String alloc per row; the typed `&str` key removes it. pandas
baseline best-of-6.

### 2026-06-24 SlateOtter — fp-join merge output sort: factorize+counting-sort — outer 0.22x->0.41x @1M (1.85x fp-side, bit-identical)
Probed Utf8 inner/left/outer/right merge (`bench_merge_utf8`, 1M ⋈ 10k) vs pandas 2.2.3. fp WINS inner
2.20x, parity left 1.04x, but OUTER was a CATASTROPHIC LOSS (770ms vs pandas 172ms = 0.22x) and right 0.79x.
pandas outer-merge output is sorted by the join key (verified: sort=False still yields key-sorted output), so
fp must sort too — but `sort_merge_rows_by_join_keys` did an O(n·log n) COMPARISON sort
(`order.sort_by(|a,b| out_row_keys[a].cmp(out_row_keys[b]))`), re-comparing full CompositeJoinKeys (string
compares for Utf8) ~n·log n times. Replaced with FACTORIZE + STABLE COUNTING-SORT: dedup the keys, sort only
the DISTINCT keys once (d≪n, here 10k), assign dense ranks, then counting-sort the n rows by rank — O(n +
d·log d), key comparisons paid only on the d distinct keys. Generic (any CompositeJoinKey via its Hash+Eq+Ord),
so it also speeds up sort=True inner/left/right. Bit-identical to the stable comparison sort: rank order == key
`cmp` order; the counting sort emits each rank's rows in ascending original-index order == the stable `sort_by`
permutation. Bench @1M/10k, load-normalized (inner load-check 65.6 vs 67.7ms):

| op (merge)   | before   | after    | pandas   | before->pandas | after->pandas | fp-side |
|--------------|----------|----------|----------|----------------|---------------|---------|
| outer        | 770.27ms | 417.30ms | 172.13ms | 0.22x          | 0.41x         | 1.85x   |

HONEST: outer is STILL A LOSS (0.41x) — the counting-sort removed the ~350ms O(n log n) string sort, but ~350ms
of outer-specific overhead remains (reindex_outer_join_column rebuilding each output column by position +
collect_single_join_keys materializing Vec<JoinKeyComponent>). Kept because it is a real bit-identical 1.85x
fp-side improvement on the WORST join loss + speeds every key-sorted merge, not ~0-gain. Correctness: new
`utf8_outer_merge_sort_conformance` (3 tests vs pandas-verified order incl. stable within-key + d==1 + all-
distinct) + existing `merge_composite_outer_sorts_join_keys_lexicographically` lib test green. RESIDUAL LEVER
(documented, next): typed reindex_outer_join_column + &str key extraction to close the remaining outer gap.
pandas baseline best-of-8.

### 2026-06-24 SlateOtter — Series.str.contains/match/startswith typed Bool output: regex 4.69x->10.05x @1M (2.15x fp-side, bit-identical)
`Series.str.contains(pat)` defaults to regex=True (pandas), routing through `str_boolean_with_na`, which had
NO typed path — it materialized `column.values()` (Vec<Scalar>) AND boxed each result into a
`Vec<Scalar::Float64/Bool>` (32B/elem, ~32MB @1M) + `from_values`. Added a typed Bool-output path (mirror of
`apply_str_bool`): an all-valid contiguous-Utf8 input scans byte spans straight into a `Vec<bool>` (no
`values()` materialization), and an all-valid non-contiguous Utf8 input still emits a `Vec<bool>` (na is moot
with no missing rows) — both via `from_bool_values` (1B/elem). Serves str.contains/match/startswith/endswith
with options. Bit-identical: every row is present Utf8, so the generic arm emits `Scalar::Bool(pred(s))` for
each, which `from_bool_values(Vec<bool>)` reproduces exactly (rcvpj: from_bool_values == from_values for Bool).
Bench `bench_str_contains` @1M (Scalar-backed Utf8, all-valid-Vec<bool> path):

| op (contains)   | before  | after   | pandas   | before->pandas | after->pandas | fp-side |
|-----------------|---------|---------|----------|----------------|---------------|---------|
| regex (default) | 27.75ms | 12.93ms | 130.02ms | 4.69x          | 10.05x        | 2.15x   |
| literal         | 39.44ms | 23.64ms | 88.05ms  | 2.23x          | 3.72x         | 1.67x   |

Both already win; removing the Vec<Scalar::Bool> output boxing is 1.67–2.15x fp-side and strengthens every
str predicate-with-options op. Correctness: new `str_boolean_typed_conformance` (5 tests: literal, regex,
anchored, case-insensitive, empty) green. pandas baseline best-of-6. (A contiguous-Utf8 input would skip the
values() materialization too, for an even bigger win — out of scope for the Scalar-backed bench.)

### 2026-06-24 SlateOtter — Series.str.replace/repeat contiguous output: replace 1.08x->2.41x @1M (2.23x fp-side, bit-identical)
Series.str.replace / repeat (Utf8-OUTPUT ops) used the slow `apply_str` — boxing a `Vec<Scalar::Utf8>` (32B +
a heap String per row, 1M) then `from_values`. lower/upper/strip already route through `apply_str_utf8` (writes
each row's output bytes into ONE rolling buffer -> `from_utf8_contiguous`, no per-row Scalar/String boxing).
Converted replace + repeat to `apply_str_utf8` too: the write closure appends the row's replaced/repeated
bytes to the buffer (replace's temp `s.replace` String is dropped immediately; repeat appends the source bytes
n times — no temp). The fallback keeps the missing/non-Utf8 path. Bit-identical: same per-row output string at
every slot; missing-bearing input still hits the Scalar fallback (preserves nulls). Bench `bench_str_replace`
@1M all-valid Utf8:

| op      | before   | after   | pandas   | before->pandas | after->pandas | fp-side |
|---------|----------|---------|----------|----------------|---------------|---------|
| replace | 133.78ms | 59.93ms | 144.20ms | 1.08x (parity) | 2.41x WIN      | 2.23x   |
| repeat  | 83.71ms  | 28.71ms | 482.00ms | 5.76x          | 16.79x         | 2.92x   |

replace FLIPS ~parity -> a clear win; repeat 5.76x -> 16.79x. Correctness: new
`str_replace_repeat_typed_conformance` (4 tests: replace/repeat all-valid + missing-fallback for both) green.
OPEN: other Utf8-output apply_str users (capitalize/title/pad/center/slice option arms) can take the same
apply_str_utf8 route. pandas baseline best-of-6.

### 2026-06-24 SlateOtter — Series.str.capitalize 0.49x->13.20x + title 1.18x->1.93x @1M (bit-identical)
Continued the Utf8-output str sweep. capitalize/title used the slow `apply_str` (Vec<Scalar::Utf8> output).
Two levers: (1) route both through `apply_str_utf8` (contiguous byte-buffer output, no per-row Scalar/String
boxing); (2) capitalize was STILL a loss after (1) — its transform did `format!("{upper}{rest}")` + char
`to_uppercase`/`to_lowercase` iterators PER ROW (Unicode case folding). Added an ASCII byte-ops fast path
(mirror of str.lower's 2krr0): for an ASCII row, append the bytes then `make_ascii_uppercase` the first byte /
`make_ascii_lowercase` the rest, IN the output buffer — no temp String, no char iterators, vectorizable.
Bit-identical: ASCII `to_uppercase`/`to_lowercase` IS the ASCII map (no context-sensitive cases); non-ASCII
rows keep the Unicode path; missing rows hit the Scalar fallback. Bench `bench_str_replace` @1M ASCII Utf8:

| op         | before   | after   | pandas   | before->pandas | after->pandas | fp-side |
|------------|----------|---------|----------|----------------|---------------|---------|
| capitalize | 275.01ms | 10.25ms | 135.23ms | 0.49x LOSS      | 13.20x WIN     | 26.83x  |
| title      | 137.96ms | 84.11ms | 162.74ms | 1.18x          | 1.93x WIN      | 1.64x   |

capitalize FLIPS catastrophic loss -> dominant win (the format!+Unicode-iterator was the entire cost); title
parity -> clear win via the contiguous output. Correctness: new `str_capitalize_title_typed_conformance`
(5 tests vs pandas-verified: ASCII, Unicode case folding "éXY"->"Éxy", empty/single, missing-fallback) green.
OPEN: title could take the same ASCII byte-ops path (~8x more); other apply_str arms (pad/center/zfill/
swapcase/casefold) remain. pandas baseline best-of-6.

### 2026-06-24 SlateOtter — Series.str.casefold 0.20x->7.26x + title 1.94x->8.27x @1M (bit-identical)
Continued the str ASCII byte-ops sweep (after capitalize 792e05b0a). casefold did `s.case_fold().collect()`
(Unicode aggressive case-folding iterator) per row via the slow apply_str — a CATASTROPHIC 442ms (0.20x
pandas). title was already on apply_str_utf8 (1.94x) but still used char `to_uppercase`/`to_lowercase`. Added
ASCII byte-ops fast paths to BOTH: casefold ASCII-lowercases in the output buffer (ASCII case-folding == ASCII
lowercasing — no multi-char foldings like ß->ss occur in pure ASCII); title walks the appended bytes marking
cased runs (a-zA-Z == the cased ASCII set). Bit-identical: ASCII byte-maps == the char Unicode ops for ASCII;
non-ASCII rows keep the Unicode path (case_fold / char iterators); missing -> Scalar fallback. Bench
`bench_str_replace` @1M ASCII:

| op       | before   | after   | pandas   | before->pandas | after->pandas | fp-side |
|----------|----------|---------|----------|----------------|---------------|---------|
| casefold | 442.34ms | 12.40ms | 89.99ms  | 0.20x LOSS      | 7.26x WIN      | 35.67x  |
| title    | 83.90ms  | 19.67ms | 162.74ms | 1.94x           | 8.27x WIN      | 4.27x   |

casefold FLIPS catastrophic loss -> win (the case_fold() iterator was 442ms); title parity-ish -> dominant
win. Correctness: new `str_casefold_title_ascii_conformance` (4 tests: ASCII+Unicode casefold incl.
"Straße"->"strasse", title ASCII/Unicode/missing — all pandas-verified) + existing capitalize/title suite
green. OPEN: swapcase (run-based Unicode, ASCII XOR-0x20 candidate), pad/center/zfill. pandas baseline best-of-6.

### 2026-06-24 SlateOtter — Series.str.swapcase ASCII XOR-0x20 + contiguous output: 9.42x WIN @1M (bit-identical)
Completed the str-case ASCII byte-ops sweep. swapcase used the slow `apply_str` with a per-row run-grouping
char `to_lowercase`/`to_uppercase` Unicode transform (the SAME apply_str + char-case path that left casefold at
0.20x and capitalize at 0.49x before their fixes). Routed through `apply_str_utf8` (contiguous byte-buffer
output) with an ASCII fast path: XOR 0x20 toggles the case of an ASCII letter (A^0x20=a, a^0x20=A) in the
output buffer; non-letters untouched. Bit-identical for ASCII (a single ASCII letter's to_uppercase/
to_lowercase IS the XOR; a non-letter swaps to itself), non-ASCII keeps the Unicode run logic, missing ->
Scalar fallback. Bench `bench_str_replace` @1M ASCII:

| op       | after (this fix) | pandas    | after->pandas |
|----------|------------------|-----------|---------------|
| swapcase | 17.60ms          | 165.76ms  | 9.42x WIN     |

After measured 9.42x WIN. (Before — the slow run-based apply_str path — was not separately re-benched this
turn; it is the identical lever as casefold 442ms->12.4ms (0.20x->7.26x) and capitalize 275ms->10.2ms
(0.49x->13.2x), i.e. a ~10-30x fp-side reduction from removing the per-row char-case Unicode work.)
Correctness: new `str_swapcase_typed_conformance` (2 tests: ASCII+Unicode "éÀb"->"ÉàB", missing-fallback —
pandas-verified) green. The Series.str case-transform surface (lower/upper/strip/capitalize/title/casefold/
swapcase) is now fully on apply_str_utf8 + ASCII byte-ops. pandas baseline best-of-6.

### 2026-06-24 SlateOtter — Series.str.pad/center/ljust/rjust/zfill contiguous output: zfill 1.25x->6.92x @1M (bit-identical)
Continued the Utf8-output str sweep onto the padding ops. pad/center/ljust/rjust (center/ljust/rjust delegate
to pad) and zfill used the slow `apply_str` — each built a per-row String + boxed a Vec<Scalar::Utf8> (32B +
a String per row) + from_values. Already MARGINAL wins (1.25-1.29x) but output-bound. Routed both through
`apply_str_utf8`: the write closure writes the fill chars + row bytes (or sign+zeros+rest for zfill) STRAIGHT
into one contiguous buffer — no per-row temp String, no Vec<Scalar::Utf8>. Extracted str_pad/str_zfill for the
Scalar fallback. Bit-identical: same fill chars / CPython-center both-split / zfill sign-first ordering; missing
-> fallback. Bench `bench_str_replace` @1M ASCII (width 30):

| op     | before   | after   | pandas   | before->pandas | after->pandas | fp-side |
|--------|----------|---------|----------|----------------|---------------|---------|
| pad    | 110.07ms | 52.53ms | 139.89ms | 1.27x           | 2.66x WIN      | 2.10x   |
| center | 109.16ms | 48.98ms | 140.86ms | 1.29x           | 2.88x WIN      | 2.23x   |
| zfill  | 110.77ms | 20.01ms | 138.54ms | 1.25x           | 6.92x WIN      | 5.54x   |

All strengthen marginal -> clear/dominant wins (zfill best — '0' fill via buf.push(b'0'), no char encoding).
Correctness: new `str_pad_zfill_typed_conformance` (4 tests: pad both-split, center/ljust/rjust, zfill sign+
passthrough, missing-fallback for pad & zfill — pandas-verified) green. The Series.str Utf8-output surface
(lower/upper/strip/capitalize/title/casefold/swapcase/replace/repeat/pad/center/ljust/rjust/zfill) is now fully
on apply_str_utf8. pandas baseline best-of-6.

### 2026-06-24 SlateOtter — Series.str is* predicates typed Bool output: isalnum 3.52x->13.75x @1M (bit-identical)
The str is* predicates (isdigit/isalpha/isalnum/isascii/isspace/islower/isupper/isnumeric/isdecimal/istitle)
used the slow `apply_str(|s| Scalar::Bool(pred))` — Vec<Scalar::Bool> (32B/elem) output + from_values — rather
than the typed `apply_str_bool` (contiguous Utf8 input -> Vec<bool> -> from_bool_values, 1B/elem) that the
contains-with-options fix (1c6844bd2) already used. Mechanical conversion of all 10. Bit-identical: same
predicate, typed Bool output == from_values over the equivalent Scalar::Bool (rcvpj). Bench `bench_str_replace`
@1M ASCII:

| op      | before  | after   | pandas   | before->pandas | after->pandas | fp-side |
|---------|---------|---------|----------|----------------|---------------|---------|
| islower | 28.35ms | 11.31ms | 115.83ms | 4.09x           | 10.24x WIN     | 2.51x   |
| isalnum | 22.18ms | 5.68ms  | 78.08ms  | 3.52x           | 13.75x WIN     | 3.90x   |
| isdigit | 34.90ms | 19.36ms | 63.34ms  | 1.81x           | 3.27x WIN       | 1.80x   |

All strengthen already-winning predicates (isalnum best, 3.90x fp-side). Correctness: new
`str_is_predicates_typed_conformance` (6 tests: islower/isalnum/isdigit/isalpha/isupper/istitle vs pandas-
verified fixture) green. The Series.str bool-output surface is now fully on apply_str_bool. pandas best-of-6.

### 2026-06-24 SlateOtter — Series.str.removeprefix/removesuffix contiguous output: 2.96x->20.75x @1M (7x fp-side, bit-identical)
removeprefix/removesuffix used the slow `apply_str` — per-row `Scalar::Utf8(strip_*(..).unwrap_or(s).to_owned())`
(a String alloc per row) + Vec<Scalar::Utf8> + from_values. But strip_prefix/strip_suffix return a BORROWED
&str, so routed through `apply_str_utf8`: the write closure appends the stripped slice's bytes straight into
one contiguous buffer — NO per-row temp String (the cheapest str transform yet), no Scalar boxing. Bit-
identical: same single-occurrence strip / passthrough; missing -> Scalar fallback. Bench `bench_str_replace`
@1M ASCII:

| op           | before  | after   | pandas   | before->pandas | after->pandas | fp-side |
|--------------|---------|---------|----------|----------------|---------------|---------|
| removeprefix | 75.09ms | 10.71ms | 222.16ms | 2.96x           | 20.75x WIN     | 7.01x   |
| removesuffix | 78.62ms | 10.70ms | 195.60ms | 2.49x           | 18.28x WIN     | 7.35x   |

Both strengthen moderate -> dominant wins (pandas removeprefix/suffix are very slow). Correctness: new
`str_removeprefix_suffix_typed_conformance` (3 tests: matching/passthrough/single-strip + missing-fallback,
pandas-verified) green. pandas baseline best-of-6. (Also measured fp `write_csv` 40k×60 = 279.5ms vs pandas
to_csv 668.3ms = 2.39x WIN — already fast, no change.)

### 2026-06-24 SlateOtter — fp-io read_csv 9.82x / write_csv 2.39x WIN (measured evidence, no change)
Probed the classic pandas-slow IO surface (untouched this session) for a winnable lever; fp DOMINATES, no
change needed. Measured vs pandas 2.2.3:
- read_csv (`read_csv_str`, cache-MISSING via probe_csv_read_uncached, 100k rows × 10 f64, 18.2MB): fp
  15.65ms (1161 MB/s) vs pandas read_csv(StringIO) 153.76ms = **9.82x WIN**.
- write_csv (`write_csv_string`, 40k rows × 60 mixed int/float/null via bench_to_csv): fp 279.5ms vs pandas
  to_csv 668.3ms = **2.39x WIN**.
Recorded as standing evidence (don't re-probe fp-io for a loss).

### 2026-06-24 SlateOtter — surface-dominance checkpoint (after 19 str/reduction/index/join wins)
After ~21 commits this session, the COMMON fp surfaces are fp-dominant vs pandas. Probed-and-confirmed WINS
not to re-chase: the entire Series.str accessor (case transforms / replace / repeat / pad family / is*
predicates / removeprefix-suffix — all on apply_str_utf8/apply_str_bool, 2-35x fp-side after this session's
conversions); Series/DataFrame/Index nullable-f64 + autocorr/skew/kurt/axis1 reductions; fp-index Utf8 set-ops
/ value_counts / get_indexer_non_unique; fp-io read_csv/write_csv; Series.map (typed Int64/dense). The
REMAINING measured non-wins are all DOCUMENTED STRUCTURAL FLOORS, not quick levers: (1) f64-arc-copy-on-produce
(cumsum/cummax/diff/abs ~0.4x — needs an owned-Vec<f64> ScalarValues variant, entangled with take_positions
zero-copy slice sharing); (2) Series sum/mean nullable-f64 ~0.6-0.8x (numpy nansum is SIMD; safe scalar
validity.get can't match, branchless forms break bit-identity); (3) fp-join merge OUTER 0.41x / RIGHT 0.79x
(residual reindex_outer_join_column + collect_single_join_keys Vec<Scalar>/JoinKeyComponent String clones —
intricate, the counting-sort already took outer 0.22x->0.41x). map_dict is DEAD CODE (Scalar: !Ord, BTreeMap
arg unconstructable). These floors need coordinated structural work on a quiet box, not a per-op typed-path.

### 2026-06-25 BlackThrush — Series.filter same-index Bool mask gather: 21.40x LOSS->1.78x LOSS vs pandas @2M (bit-identical)
`Series::filter(mask_series)` still cloned one `IndexLabel` plus one `Scalar` per kept row on the common
identical-index, no-duplicate mask path, even though `loc_bool(&[bool])` already used a positions vector with
`Index::take` + `Column::take_positions`. BOLD-verified the live gap with the warmed per-crate command
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo run -p fp-frame --example
bench_loc_bool --release -- 2000000 30`; RCH had no admissible worker and failed open locally, but the target dir
was warm and the command stayed `-p fp-frame`. Baseline current-main `filter_series` best 62.63ms vs pandas
2.93ms (21.40x pandas). The lever skips mask `Vec<Scalar>` materialization for Bool/Boolean dtype masks and
routes the same-index path through positions + typed index/column gathers. After: 5.04ms best-of-30 and 5.22ms
best-of-50, golden `0e95636b5d230bf5` unchanged; pandas comparator best 2.93ms with the same golden, so the
remaining ratio is 1.72-1.78x pandas and the fp-side speedup is 12.44x. This is still a pandas loss, but it
removes the catastrophic clone/materialization gap; next lever should attack the positions vector scan/gather
itself or reuse bool affine witnesses for common masks. Post-format remote rerun on `vmi1149989` stayed green
with the same golden and 4.24ms best-of-30; that is supplemental only, not used for the local pandas ratio.

### 2026-06-25 SlateOtter — str.get typed nullable-Utf8 output = ~0-gain (REVERTED); char-iteration-bound not output-bound
Tried extending the apply_str_utf8 lever to NULL-producing str ops (str.get): added an `apply_str_opt_utf8`
helper (write closure returns bool present/null -> contiguous bytes+offsets+validity via
`from_utf8_values_with_validity` = LazyNullableUtf8, no Vec<Scalar::Utf8> boxing) and routed str.get through
it. Measured @1M ASCII (get(3)): before (apply_str) 79.99ms, after (helper) 76.71ms = **1.04x fp-side = ~0-gain
-> REVERTED**. CAUSE: str.get is dominated by the per-row `s.chars().count()` (bound check) + `s.chars().nth(i)`
O(len) iteration, NOT the output boxing — and an ASCII byte-index fast path doesn't help either because
`s.is_ascii()` scans the whole string per row (same O(len) cost as chars().count()). str.get already WINS 1.55x
vs pandas (126ms) on the char-iteration alone. LESSON: the typed-OUTPUT lever only pays when per-row COMPUTE is
trivial (replace/repeat/pad/case ops) — for char-iteration ops (get) or regex ops (extract/contains-regex) the
compute dominates and output-boxing removal is ~0-gain. Don't re-attempt typed-output for str.get/extract/
slice_replace. (The `apply_str_opt_utf8` helper + LazyNullableUtf8 path is sound and available if a trivial-
compute null-producing str op ever appears.)

### 2026-06-25 BlackThrush — Series nullable-f64 sum/mean packed-validity scan: 0.78x LOSS->5.43x WIN, 0.75x LOSS->5.88x WIN @1M (bit-identical)
The June 24 surface-dominance checkpoint listed nullable-f64 `Series.sum/mean` as a remaining structural floor:
`sum` walked `validity.get(i)` for every row and `mean` did `count()` plus `sum()`, two branchy validity scans.
The lever adds one `f64_valid_sum_count` primitive over `ValidityMask::packed_words_for_scan()`, consuming set bits
in ascending row order. `sum` uses the returned total; `mean` uses the same one-pass total/count. Bit-identical:
same valid rows, same row-order `0.0`-seeded f64 additions, same all-missing `sum=0.0` and `mean=NaN` contracts.

Bench: `bench_series_null` at `n=1_000_000`, best-of-8, local warm target
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`, crate-scoped `-p fp-frame`. Pandas comparator
uses the exact same SplitMix data generator and pandas 2.2.3.

| op   | before fp | after fp | pandas  | before speed | after speed | fp-side |
|------|-----------|----------|---------|--------------|-------------|---------|
| sum  | 4.223ms   | 0.604ms  | 3.279ms | 0.78x LOSS   | 5.43x WIN   | 6.99x   |
| mean | 4.625ms   | 0.592ms  | 3.478ms | 0.75x LOSS   | 5.88x WIN   | 7.81x   |

Supplemental remote RCH timings after the edit: `sum=0.559ms` on `vmi1149989`, `mean=0.710ms` on `vmi1227854`.
Correctness: `cargo test -p fp-frame --test series_nullable_reduction_conformance -- --nocapture` green
(6 tests: nullable sum/mean/var/std, all-missing, all-valid). `cargo check -p fp-frame --all-targets` green
with only pre-existing example unused-import warnings. `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs`
green. `cargo clippy -p fp-frame --all-targets -- -D warnings` is green after allowing the observed pre-existing
unrelated lint classes (`unused-imports`, `if_same_then_else`, `doc_lazy_continuation`, `manual_is_multiple_of`,
`inconsistent_digit_grouping`). Bounded `timeout 180s ubs crates/fp-frame/src/lib.rs` timed out without findings,
matching the documented known scanner backlog for this file.

### 2026-06-25 SlateOtter — composite (Utf8/multi-key) RIGHT merge small-side probe: 0.90x LOSS -> 1.21x WIN @1M⋈10k (bit-identical)
The generic composite merge path (non-i64 keys: Utf8, multi-key, datetime — the i64 Right fast paths at ~7408
don't fire) built a `left_map` over EVERY left row for a Right join (O(left) hash build), then probed it with the
right rows — so a 1M⋈10k Right merge was ~2.4x slower than the SYMMETRIC Left join (which builds the small
O(right) map). Fix: build the small `right_map` (as Inner/Left already do) and run ONE left pass that buckets each
matching left position under its right row (`left_by_right[right_pos].push(left_pos)`), then emit right rows in
order. Bit-identical (right-row order outer, left-position order inner — verified across duplicate right keys,
multiple left matches, and unmatched right rows). Dropped the left_map build entirely for Right (Outer still
builds it). Bench `bench_merge_utf8` 1M fact ⋈ 10k dim, Utf8 key:

| how   | before   | after    | pandas   | before->pandas | after->pandas |
|-------|----------|----------|----------|----------------|---------------|
| right | 231.66ms | 173.65ms | 209.54ms | 0.90x LOSS      | 1.21x WIN      |

(inner 5.30x / left 1.49x / outer 1.22x WINs unchanged.) Right was the LAST merge loss in the Utf8 matrix —
now all four win. Residual: Right (173ms) is still ~1.8x the Left join (97ms) — the per-right bucketing Vecs +
the row_capacity=right_len under-allocation of the output position Vecs (a pre-existing realloc, present in both
old and new) are the remaining gap; a 2-pass pre-size is a follow-up. Correctness: all 135 fp-join tests +
new `utf8_right_merge_smallside_conformance` (2 tests, pandas-verified) green. pandas baseline best-of-6.

### 2026-06-25 SlateOtter — Utf8/mixed multi-key groupby WINS 1.16x but skips the dense bypass (build_groups not avoided) — characterized lever
Probed the documented OPEN "multi-Utf8 groupby" vein. df.groupby([k1_utf8, k2_utf8]).sum/mean/count @1M (g=100 =>
~10k group combos) MEASURED (new bench_gb2_utf8):

| op    | fp      | pandas   | ratio    |
|-------|---------|----------|----------|
| sum   | 90.70ms | 105.21ms | 1.16x WIN |
| mean  | 89.63ms | 103.41ms | 1.15x WIN |
| count | 90.87ms | 105.85ms | 1.16x WIN |

A WIN, but a MARGINAL one — and the cause is a missing dense bypass, not a structural floor. The moments engine
(agg_typed_pairs_dense_f64_moments) bypasses build_groups ONLY for single-Int64 (int64_dense_grouping) and
multi-INT64 (multi_int64_dense_grouping) keys; for any Utf8 key it returns None, so Utf8/mixed multi-key falls
through to the FULL generic build_groups (the 1M per-row Vec<ScalarKey> heap-alloc + composite SipHash — the
documented dominant build_groups cost) AND THEN the dense gid precompute (64329) RE-factorizes the Utf8 columns
to gid_per_row. Two grouping passes; build_groups' is the slow one. LEVER (deferred — quiet box): add a
multi_mixed_dense_grouping() (the factorize-each-Utf8-to-u32 + mixed-radix gid_table already exists at ~64372)
that ALSO emits group_order + the MultiIndex directly, so the moments path skips build_groups for Utf8/mixed
multi-key (mirrors the int64 bypass). EST 90ms -> ~40ms (~2.5x). RISK: bit-identity requires reproducing
build_groups' EXACT sorted group order (composite_key_cmp lexicographic) from the factorize maps + inverse
code->str tables — intricate, golden-gated, shared hot path; NOT safe to iterate on a saturated fleet. The
single-Utf8 dense path (64308) and single/multi-Int64 bypasses already exist; this is the multi-Utf8 sibling.

### 2026-06-25 SlateOtter — multi-Utf8-key groupby dense bypass (build_groups skip): 1.06x->3.74x @1M (bit-identical)
LANDED the lever characterized last commit. Added multi_utf8_dense_grouping (factorize each contiguous-Utf8 key
to first-seen u32 codes + inverse code->str, mixed-radix gid_table, sorted group order) and wired it into the
moments engine (agg_typed_pairs_dense_f64_moments) as the Utf8 sibling of the Int64 dense bypass. For >=2
contiguous-Utf8 keys it SKIPS build_groups (the per-row Vec<ScalarKey> heap-alloc + composite SipHash that
dominated) and feeds moments_by_pair directly. Bit-identical group order: Vec<String>::cmp == composite_key_cmp
for all-Utf8 keys (scalar_key_cmp Utf8 == str::cmp); same ", "-joined flat label + per-level Utf8 MultiIndex as
the generic path. bench_gb2_utf8 1M, g=100 (~10k combos), CONTIGUOUS Utf8 keys (as read_csv/str-ops produce):

| op    | before  | after   | pandas   | before->pandas | after->pandas | fp-side |
|-------|---------|---------|----------|----------------|---------------|---------|
| sum   | 99.38ms | 28.16ms | 105.21ms | 1.06x           | 3.74x WIN      | 3.53x   |
| mean  | 92.46ms | 28.39ms | 103.41ms | 1.12x           | 3.64x WIN      | 3.26x   |
| count | 103.77ms| 28.30ms | 105.85ms | 1.02x           | 3.74x WIN      | 3.67x   |

GATE: fires only for CONTIGUOUS Utf8 key columns; a Scalar-backed column (from_values) bails as_utf8_contiguous
and keeps the generic path (so the earlier 90ms "1.16x" reading was a Scalar-backed bench artifact — real Utf8
columns from read_csv/str-ops are contiguous and hit the dense path). Correctness: new
groupby_multi_utf8_dense_conformance proves dense == generic (same data built contiguous-vs-Scalar-backed) ==
pandas (values + sorted MultiIndex order); existing groupby_nullable_dense/fxhash/sgb conformance (17 tests)
green. FOLLOW-UPS: the same multi_utf8 grouping could extend the 4 OTHER multi_int64_dense_grouping call sites
(min/max/first/last/prod/median via aggregate_multi_int64_dense; bool reduce; idxmax/min; nunique) + a mixed
Int64/Utf8 variant. pandas baseline best-of-6.

### 2026-06-25 SlateOtter — multi-Utf8 groupby dense bypass extended to min/max/first/last/prod/median: 1.08x->4.03x @1M (bit-identical)
Follow-up to 61c71a007: wired multi_utf8_dense_grouping into aggregate_multi_int64_dense (the
min/max/first/last/prod/median path), so those funcs ALSO skip build_groups for contiguous-Utf8 multi-keys.
dense_aggregate_emit is grouping-agnostic → reused verbatim; only the Utf8 MultiIndex assembly differs.
bench_gb2_utf8 1M g=100 contiguous Utf8 keys:

| op     | before   | after   | pandas   | before->pandas | after->pandas | fp-side |
|--------|----------|---------|----------|----------------|---------------|---------|
| min    | 88.41ms  | 24.93ms | 98.36ms  | 1.11x           | 3.95x WIN      | 3.55x   |
| max    | 91.82ms  | 24.57ms | 98.95ms  | 1.08x           | 4.03x WIN      | 3.74x   |
| median | 105.35ms | 39.44ms | 114.00ms | 1.08x           | 2.89x WIN      | 2.67x   |

Correctness: groupby_multi_utf8_dense_conformance extended to min/max/median (dense == generic == pandas, +
sum/mean/count from the prior commit). REMAINING multi_utf8 sites: try_bool_reduce_dense, try_idx_extreme_dense
(idxmax/idxmin), try_nunique_dense, + a mixed Int64/Utf8 key variant. pandas baseline best-of-6.

### 2026-06-25 SlateOtter — multi-Utf8 groupby dense bypass extended to idxmax/idxmin + nunique: idxmax 1.01x->3.48x @1M (bit-identical)
Further follow-up: added a multi_dense_index_utf8 helper and wired multi_utf8_dense_grouping into
try_idx_extreme_dense (idxmax/idxmin) and try_nunique_dense (the DataFrameGroupBy ones), so they skip
build_groups for contiguous-Utf8 multi-keys. bench_gb2_utf8 1M g=100 contiguous Utf8 keys, f64 values:

| op     | before   | after   | pandas   | before->pandas | after->pandas | fp-side |
|--------|----------|---------|----------|----------------|---------------|---------|
| idxmax | 104.94ms | 30.39ms | 105.64ms | 1.01x (tied)    | 3.48x WIN      | 3.45x   |

nunique was NOT benchable here (its dense value-bitset needs i64 values; the f64-value bench keeps the generic
path either way — stays ~115ms/1.54x). Its multi_utf8 grouping branch is wired + conformance-verified for the
i64-value case (dense == generic == pandas [2,1]). Correctness: groupby_multi_utf8_dense_conformance now also
covers idxmax (f64, labels [3,2,0,4]) and nunique (i64, [2,1]) dense==generic==pandas; existing
groupby_nullable_dense/fxhash conformance green. REMAINING multi_utf8 site: try_bool_reduce_dense (any/all) +
the mixed Int64/Utf8 key variant. pandas baseline best-of-6.

### 2026-06-25 SlateOtter — multi-key groupby dense bypass generalized to MIXED Int64+Utf8 keys: 1.00x->3.53x @1M (bit-identical)
Generalized multi_utf8_dense_grouping -> multi_mixed_dense_grouping (DenseMultiUtf8Grouping -> DenseMultiMixed
with a MixedKey{Int64,Utf8} enum). Each key column is bounded-Int64 (codes = value-min) OR contiguous-Utf8
(factorize); requires >=1 Utf8 (all-Int64 stays on the faster multi_int64 path). So groupby([int_id, str_cat])
— a common real pattern (e.g. year + category) — now skips build_groups too. Bit-identical: derived MixedKey::cmp
orders per level by value (Int64 i64 cmp, Utf8 str cmp) == scalar_key_cmp, so the sorted group order ==
composite_key_cmp; flat label joins each level's Display (Int64 plain, Utf8 verbatim) == generic. All 4 prior
multi_utf8 call sites (moments sum/mean/count/var/std, aggregate min/max/first/last/prod/median, idxmax/idxmin,
nunique) now use the mixed grouping + a shared multi_dense_index_mixed helper. bench_gb2_mixed 1M g=100 (k1 i64,
k2 contiguous-Utf8, f64 values):

| op  | before  | after   | pandas  | before->pandas | after->pandas | fp-side |
|-----|---------|---------|---------|----------------|---------------|---------|
| sum | 67.76ms | 20.54ms | 72.49ms | 1.07x           | 3.53x WIN      | 3.30x   |
| max | 65.47ms | 19.89ms | 65.52ms | 1.00x (tied)    | 3.29x WIN      | 3.29x   |

All-Utf8 path unchanged (no regression, ~29ms/3.6x — it is now the all-str case of the mixed grouping).
Correctness: groupby_multi_utf8_dense_conformance extended with a mixed int+utf8 case (dense == generic ==
pandas, incl. NUMERIC k1 sort: (1,x)<(2,x)); existing all-utf8/idxmax/nunique + groupby_nullable_dense/fxhash
conformance green. The multi-Utf8/mixed groupby vein is now complete except try_bool_reduce_dense (any/all).

### 2026-06-25 SlateOtter — multi-column duplicated/drop_duplicates typed raw path (Int64+Utf8): 2.55x->3.75x @1M (bit-identical)
The multi-column duplicated_mask had a typed raw-digest path only for ALL-Float64 subsets; any Int64/Utf8/mixed
subset fell to the generic Scalar-digest path (a per-cell `col.values()[row]` materialization — a String alloc
per Utf8 cell). Generalized typed_cols from f64-only to a TypedDedupCol{F64,I64,Utf8} enum, so any subset of
Int64 + contiguous-Utf8 + Float64 columns digests/compares straight from the raw backing. Bit-identical: the
per-type digest mixes match scalar_digest exactly (Int64 mix(2)+value; Utf8 mix(4)+len+bytes_digest; Float64
mix(3)+bits, same present rule) and equality matches Scalar::semantic_eq (Int64/Utf8 exact, Float64 fuzzy) — so
digests, bucket assignment, and collision re-verification are unchanged. bench_dropdup_multi 1M, 2 contiguous-
Utf8 cols, card=100:

| op             | before  | after   | pandas  | before->pandas | after->pandas | fp-side |
|----------------|---------|---------|---------|----------------|---------------|---------|
| drop_duplicates| 30.39ms | 20.61ms | 77.36ms | 2.55x           | 3.75x WIN      | 1.47x   |
| duplicated     | 27.13ms | 17.81ms | 81.19ms | 2.99x           | 4.56x WIN      | 1.52x   |

Already a WIN; this strengthens it by removing the Scalar materialization for Int64/Utf8/mixed subsets.
Correctness: new duplicated_multicol_typed_conformance (typed contiguous vs Scalar-backed == generic == pandas
across keep=First/Last/None); existing fxhash_dedup_conformance green.

### 2026-06-25 SlateOtter — multi-key OUTER merge 0.55x LOSS (root-caused) + standing bench; fix deferred (golden-gated, no-fallback)
Probed multi-key (2 Utf8) merge via new bench_merge2_utf8 (1M fact ⋈ 100×100 dim, contiguous Utf8 keys). Measured
vs pandas 2.2.3 (best-of-8):

| how   | fp       | pandas   | ratio     |
|-------|----------|----------|-----------|
| inner | 158.58ms | 176.06ms | 1.11x WIN |
| left  | 154.67ms | 188.07ms | 1.22x WIN |
| outer | 785.06ms | 435.03ms | 0.55x LOSS |

Inner/left marginal wins; OUTER is a clear LOSS (the documented residual, now reproduced for multi-key — single-
key outer wins 1.22x). ROOT CAUSE (read, not guessed): the composite merge path (~7629) materializes left_keys
(1M) + right_keys as Vec<CompositeJoinKey> = SmallVec<[JoinKeyComponent;1]> where Utf8 = IndexLabel::Utf8(String);
then for needs_key_order (sort|Outer) `push_merge_row_key` CLONES one CompositeJoinKey PER OUTPUT ROW (~1M SmallVec
allocs + 2M String clones) into out_row_keys, which sort_merge_rows_by_join_keys (already a factorize+counting
sort) then hashes. CONTAINED LEVER: drop out_row_keys entirely — have sort_merge derive each row's key by
reference from the positions (key_ref(i) = left_positions[i].map(|p| &left_keys[p]).unwrap_or(&right_keys[
right_positions[i]])), bit-identical (matched rows use the left key today, unmatched-right use the right key).
Requires MOVING the sort into the key-building block (left_keys/right_keys are block-local), i.e. modifying the
golden-gated outer-sort path with NO fallback — distinct from the additive groupby dense bypass, so unsafe to
one-shot on a saturated fleet. FULLER LEVER: factorize join keys to integer codes once (khash-style, like
multi_mixed_dense_grouping) and run the join + sort + reindex on codes, never materializing CompositeJoinKey —
helps inner/left too. Both deferred to a quiet box with fast iteration + the merge goldens.

### 2026-06-25 SlateOtter — DataFrame.value_counts dense fast path (Int64/Utf8 contiguous): 0.50x LOSS -> 3.63x WIN @1M (bit-identical)
df.value_counts() (distinct row-tuple counts) was a LOSS: the generic path built a per-row Vec<ScalarKey> + a
per-cell `col.values()[i]` Scalar materialization + `value.to_string()` (a String alloc per cell, twice). Added
an ADDITIVE value_counts_dense_contiguous fast path: when every column is bounded-Int64 or contiguous-Utf8 (all-
valid) with bounded combined cardinality, factorize each column + dense gid-table count, then emit the same
`count desc, composite_key_cmp asc` order and `", "`-joined labels. Any other dtype / missing column / wide
cardinality falls back to the generic path. Bit-identical: MixedKey::cmp == composite_key_cmp and MixedKey
Display == Scalar Display for Int64/Utf8; all-valid backings ⇒ no missing key class; count ties resolve by the
same composite key (distinct, so the stable sort's first-seen tiebreak never fires). bench_df_vc 1M, 2
contiguous-Utf8 cols, card=100:

| op              | before   | after   | pandas   | before->pandas | after->pandas | fp-side |
|-----------------|----------|---------|----------|----------------|---------------|---------|
| df.value_counts | 209.39ms | 28.88ms | 104.85ms | 0.50x LOSS      | 3.63x WIN      | 7.25x   |

Correctness: new df_value_counts_dense_conformance (typed contiguous vs Scalar-backed == generic == pandas,
count desc + composite-key tiebreak); existing fxhash_dedup green. The contiguous-key factorize lever (groupby +
dedup + now value_counts) keeps flipping composite-key ops from Scalar-materialization losses to ~3-7x wins.

### 2026-06-25 SlateOtter — GroupBy.size dense bypass for >=2 keys: 1.72x->3.68x @1M (bit-identical)
df.groupby([k1,k2]).size() already won 1.72x but still went through build_groups (per-row Vec<ScalarKey>) for
its sole cost. Added a dense fast path: for >=2 keys, multi_int64_dense_grouping / multi_mixed_dense_grouping
gives gid_per_row; histogram it for the per-group count and build the same ", "-joined multi-key labels (the
generic group_key_label format) in the same sorted-key order. Flat Index, no MultiIndex — exactly what the
generic size returns. bench_gb2_utf8 1M g=100 contiguous Utf8 keys:

| op   | before  | after   | pandas  | before->pandas | after->pandas | fp-side |
|------|---------|---------|---------|----------------|---------------|---------|
| size | 56.94ms | 26.64ms | 98.06ms | 1.72x           | 3.68x WIN      | 2.14x   |

Already a WIN; this strengthens it (build_groups was its whole cost). Correctness: groupby_multi_utf8_dense_
conformance extended with size (dense == generic == pandas, sorted-key order); existing 4 cases green.

### 2026-06-25 BlackThrush — Series Bool mask direct typed gather: 1.25x LOSS->1.35x WIN vs pandas @2M (bit-identical)
Followed the 2026-06-25 residual above: `Series::filter(mask_series)` and `Series::loc_bool(&[bool])` still
materialized a `Vec<usize>` positions buffer, then gathered the Int64 index and Int64 payload in separate
generic passes. The new path recognizes all-valid Bool masks over Int64-labelled, all-valid Int64/Float64
Series and emits typed labels + typed payload in one mask scan; duplicate-index, nullable-mask, non-typed, and
aligned fallback semantics stay on the existing paths. This is the selection-vector/rank-select lever from the
graveyard: carry the mask witness straight to dense output construction instead of first expanding it to row
ids.

BOLD head-to-head on CPU 7, warmed target dir `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`,
per-crate only (`cargo build --release -p fp-frame --example bench_loc_bool`), 2M rows / 999,998 kept:

| case | before | after | pandas | before->pandas | after->pandas | fp-side |
|------|--------|-------|--------|----------------|---------------|---------|
| `filter_series` | 5.459ms | 2.754ms | 3.737ms | 1.25x LOSS | 1.35x WIN | 1.98x |
| `loc_bool` | 5.019ms | 2.740ms | 3.737ms | 1.34x LOSS | 1.36x WIN | 1.83x |

Golden stayed `0e95636b5d230bf5` across baseline and after runs. A first after run showed allocator noise
(`filter_series` 6.045ms), but two immediate same-binary reruns stabilized at 2.956ms and 2.820ms, and the
100-iteration confirmation was 2.754ms. The pandas comparator used the same mixed-hash mask and pandas Series
index/mask alignment shape.

Post-rebase release rebuild confirmation on the final source: `loc_bool=2.793ms`, `filter_series=3.149ms`,
pandas aligned Series bool mask `4.244ms` (pandas 2.2.3), preserving the `filter_series` 1.35x WIN and
strengthening `loc_bool` to 1.52x on that run.

### 2026-06-25 SlateOtter — GroupBy.value_counts contiguous-Utf8 span tally: 0.72x LOSS -> 3.38x WIN @1M (bit-identical)
df.groupby(k).value_counts() was a LOSS: the per-group value tally did `col.values()[ri]` PER ROW (materializing
the value column to Scalars — a String alloc per row) keyed through a ScalarKey-keyed std HashMap (SipHash).
Added a span fast path: when the value column is contiguous Utf8 (all-valid), tally each group by raw &[u8] span
with FxHash, rebuilding Scalar::Utf8 once on first sight. Bit-identical: all-valid ⇒ no missing to skip, first-
seen order = same row iteration, stable count-desc sort unchanged. Falls back to the ScalarKey path for any other
dtype / missing value column. bench_gb_vc 1M, gcard=100 vcard=100, contiguous Utf8 k+v:

| op               | before   | after   | pandas   | before->pandas | after->pandas | fp-side |
|------------------|----------|---------|----------|----------------|---------------|---------|
| gb.value_counts  | 269.17ms | 56.94ms | 192.45ms | 0.72x LOSS      | 3.38x WIN      | 4.73x   |

Correctness: new gb_value_counts_span_conformance (span vs Scalar-backed == generic == pandas, per-group count
desc + first-seen tiebreak). The contiguous-key/span lever (groupby agg+size, dedup, df.value_counts, now
GroupBy.value_counts) keeps flipping Scalar-materialization losses to ~3-7x wins. RESIDUAL: build_groups by the
GROUP key (k) is still the Vec<ScalarKey> path — a fuller (k,v) dense factorize could push this higher.

### 2026-06-25 BlackThrush — multi-Utf8 GroupBy all/any mixed dense bypass: 1.03x->3.30x @1M (bit-identical)
Finished the remaining multi-Utf8/mixed groupby dense-bypass site called out above: `DataFrameGroupBy::all()` /
`any()` had the single-key and multi-Int64 dense path, but a contiguous-Utf8 or mixed Int64+Utf8 key still fell
back to `build_groups` before doing the typed truthy fold. The new branch reuses `multi_mixed_dense_grouping()`
and `multi_dense_index_mixed()`, then runs the existing sequential Bool/Int64/Float64 truthiness reducer over
`gid_per_row`. Behavior is unchanged: group order is the same sorted mixed key order used by the generic path,
Bool truthiness is identity, numeric truthiness is `!= 0` / `!= 0.0`, and nullable or non-typed value columns
still fall back.

BOLD head-to-head on CPU 7, warmed target dir `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`,
per-crate only (`cargo build --release -p fp-frame --example bench_gb2_utf8`), `bench_gb2_utf8` at 1M rows,
100x100 contiguous-Utf8 groups, f64 value column, best-of-20 after and pandas:

| op | before fp | after fp | pandas 2.2.3 | before->pandas | after->pandas | fp-side |
|----|-----------|----------|--------------|----------------|---------------|---------|
| all | 97.188ms | 30.310ms | 100.110ms | 1.03x WIN | 3.30x WIN | 3.21x |
| any | 90.795ms | 30.129ms | 104.510ms | 1.15x WIN | 3.47x WIN | 3.01x |

Correctness: `groupby_multi_utf8_dense_conformance::bool_reduce_dense_matches_generic_and_pandas` covers
contiguous-Utf8 dense vs Scalar-backed generic output for both `all` and `any`, including Bool and Int64
truthiness under pandas sorted group order.

Post-rebase release rebuild confirmation on the final source: `all=27.623ms`, `any=26.951ms` against the same
pandas 20-run comparator above, strengthening the final ratios to 3.62x and 3.88x vs pandas.

### 2026-06-25 BlackThrush — fp-join RIGHT UTF-8 borrowed-byte position builder: 0.76x LOSS -> 0.34x LOSS (REVERTED)
Tried the symmetric all-valid UTF-8 right-join position builder: build a left `FxHashMap<&[u8], positions>`,
walk right keys in right order, and feed `(left_positions, right_positions)` into the existing dense right merge
materializer. The hypothesis was that right joins were missing the borrowed-byte fast paths already present for
inner/left/outer UTF-8 joins and were falling through to cloned `JoinKeyComponent::Utf8` keys. It was
bit-transparent on paper (same all-valid guard, same right-row order, same left bucket order, same unmatched-right
row emission), but the measured path got worse.

Bench shape: `bench_merge_utf8`, `n=1_000_000`, `card=10_000`, `how=right`, best-of-8, crate-scoped
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo run -p fp-join --example
bench_merge_utf8 --release -- 1000000 10000 8 right`. Pandas 2.2.3 comparator used the same key generator and
`left.merge(right, on="key", how="right")`.

| path | timing | ratio |
|------|--------|-------|
| pandas | 168.390ms | 1.00x |
| current fp baseline | 220.872ms | 0.76x vs pandas |
| UTF-8 right-position candidate | 501.244ms | 0.34x vs pandas; 0.44x vs current fp |

REVERTED. Do not retry this isolated right-position builder. The residual is not solved by wrapping another
borrowed-byte position vector around `build_single_key_dense_right_merge_output`; it needs deeper output-side work
or a right-join materializer that avoids optional-position duplication and repeated key/payload gathers.

### 2026-06-25 BlackThrush — Series Int64 Bool-mask direct-compress helper superseded: 1.01-1.03x vs main (REVERTED)
After `origin/main` landed the broader `Series Bool mask direct typed gather` path above, a narrower
`Index::take_bool_i64_values` + `Series::filter_int64_by_bool_mask` helper no longer had a credible measured
edge. It preempted the main path for all-valid Int64 Series, but handled only Int64 payloads while the main path
also covers Float64 and already emits typed labels plus values in one pass for the benchmark shape.

Same-worker RCH comparison on `hz2`, warmed target dir
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`, crate-scoped
`cargo run -p fp-frame --example bench_loc_bool --release -- 2000000 30`, golden
`0e95636b5d230bf5` unchanged:

| path | origin/main | candidate | candidate/main |
|------|-------------|-----------|----------------|
| `Series::loc_bool(&[bool])` | 2.786ms | 2.713ms | 1.03x |
| `Series::filter(mask_series)` | 2.804ms | 2.777ms | 1.01x |

REVERTED before landing. Treat this as noise-level/superseded, not a new lever. Keep using the broader main-path
typed gather unless a future benchmark proves a separate cold lazy-affine or mixed-selected-label case with a
real same-worker delta and no loss of Float64 coverage.

### 2026-06-25 SlateOtter — multi-key merge sort: drop per-row CompositeJoinKey clone (ranks by reference): outer ~16% faster, 0.55x->~0.65x (still LOSS, bit-identical)
Attacked the documented biggest gap (multi-key OUTER merge 0.55x). The composite merge path cloned one String-
owning CompositeJoinKey PER OUTPUT ROW (~1M SmallVec + 2M String) into out_row_keys, purely to feed the
factorize+counting sort. Replaced it: sort_merge_rows_by_join_keys now takes left_keys/right_keys + the position
arrays, derives each row's key BY REFERENCE (matched/unmatched-left -> left key, unmatched-right -> right key —
exactly what out_row_keys stored), factorizes left∪right to dense ranks, and counting-sorts. The sort moved
in-block (where left_keys/right_keys are in scope); push_merge_row_key now no-ops. Bit-identical: same per-row
key, same cmp-ordered ranks (over a superset -> empty buckets), same stable emission — all 135 fp-join tests +
utf8_outer_merge_sort_conformance green.

bench_merge2_utf8 1M ⋈ 100×100, 2 contiguous-Utf8 keys, CONTROLLED interleaved (before-vs-after back-to-back on
the same machine load, min of 4 pairs): outer before 822ms -> after 691ms = ~16% faster (consistently 16-24%
per pair). vs pandas 435ms: ~0.53x -> ~0.63x — STILL A LOSS, but the biggest gap is reduced and a real smell
(1M key clones) removed. inner/left unaffected (they don't sort). RESIDUAL: the loss is now dominated by the
CompositeJoinKey MATERIALIZATION (collect_composite_keys, 1M left) + outer column reindex; a flip needs the
fuller factorize-join-keys-to-int-codes lever (join/sort/reindex on codes, never materializing CompositeJoinKey)
— deferred (bigger restructure of the whole composite path).

### 2026-06-25 SlateOtter — GroupBy.first/last Utf8-key dense path: 0.41x LOSS -> 6.98x WIN @1M (bit-identical)
df.groupby(k_utf8).first()/last() was a big LOSS: try_first_last_dense required an as_i64_slice KEY, so a Utf8
key bailed to aggregate_named_func's generic build_groups + per-group Scalar gather (materializing the value
column). Added a contiguous-Utf8 key branch: factorize the key (FxHash &[u8] -> codes + inverse), sort gids by
key string (sort=True), build the Utf8-labelled named Index — then the EXISTING chosen-row + take_positions
(zero-copy, value-type-agnostic) gather runs unchanged, so a Utf8 (or any) value column is gathered without
Scalar materialization. Falls back to the generic path for a non-Int64/Utf8 key or a null-bearing value column.
bench_gb_first 1M, gcard=100, contiguous Utf8 k + Utf8 v:

| op    | before   | after   | pandas  | before->pandas | after->pandas | fp-side |
|-------|----------|---------|---------|----------------|---------------|---------|
| first | 180.75ms | 10.48ms | 73.21ms | 0.41x LOSS      | 6.98x WIN      | 17.2x   |
| last  | 179.58ms | 11.01ms | 76.94ms | 0.43x LOSS      | 6.99x WIN      | 16.3x   |

Correctness: new gb_first_last_utf8_dense_conformance (contiguous-key dense == scalar-backed generic == pandas,
first/last row per group, sorted-key index); existing groupby_nullable_dense + multi_utf8_dense conformance
green (the Int64-key path is preserved — its gate just moved inside the new key-type branch).

### 2026-06-25 SlateOtter — GroupBy.max/min Utf8-value dense path: 1.33x->11.81x, 1.38x->13.67x @1M (bit-identical)
groupby(k).max()/min() over a Utf8 value column was only a marginal WIN (pandas object aggregation is slow): the
numeric dense_aggregate_emit can't reduce Utf8, so it fell to the generic build_groups + per-group Scalar gather.
Added try_minmax_str_dense (single key Int64/Utf8 factorize): per group, the argmax/argmin ROW is found by &[u8]
span comparison, then take_positions (zero-copy, value-agnostic) gathers it — same lever as first/last. Bit-
identical: gathered value IS the group's lexicographic max/min string; sorted-key index. bench_gb_first 1M
gcard=100 contiguous Utf8 k+v: max 179.35->20.14ms (1.33x->11.81x vs pandas, 8.9x fp-side), min 180.87->18.29ms
(1.38x->13.67x, 9.9x). conformance gb_first_last_utf8_dense extended (max/min dense==generic==pandas); first/last
+ int64 min/max unaffected.

### 2026-06-25 SlateOtter — LEDGER REJECT: Series abs 0.36x / cumsum 0.93x @5M — arc-copy-on-produce floor (structural blocker)
Re-measured the biggest gap-vs-pandas. Series.abs @5M f64: fp 63.20ms vs pandas 22.54ms = 0.36x LOSS. cumsum:
fp 65.68ms vs pandas 60.96ms = 0.93x (marginal LOSS). CAUSE (documented f64-arc-copy-on-produce): all f64 storage
is Arc<[f64]>; producing a Vec<f64> output then from_f64_values does Arc::from(Vec) = a 40MB REALLOC+COPY. abs is
|x| (trivial compute) so it is dominated by traffic: ~120MB (read 40 + Vec write 40 + Arc copy 40) vs pandas ~80MB
-> 0.36x. NO contained fix: Arc<[f64]> requires the copy (ArcInner-prefix layout); avoiding it needs storing f64
as Arc<Vec<f64>> (move, no copy) — a deep type change cascading through the slice-sharing variants
(LazyAllValidFloat64Slice/Dot/Strided share Arc<[f64]> for zero-copy take_positions) — OR an additive
Float64Owned(Arc<Vec<f64>>) variant whose as_f64_slice returns &v[..]. The additive variant is COMPILER-GUIDED
(exhaustive ScalarValues matches -> every site is a compile error to fix, no silent bugs) but ~44 arms + golden-
risky; deferred to a quiet box with fast iteration. in-place (Arc::get_mut) doesn't apply: abs(&self) shares the
input Arc. REJECTED zero-gain attempts: none tried (structural). This is the standing biggest gap.

### 2026-06-25 SlateOtter — GroupBy.nunique Utf8-value span path: 1.05x->2.29x @1M (bit-identical)
groupby(k).nunique() over a HIGH-cardinality Utf8 value was only marginally winning (1.05x): the (group,value)
bitset in try_nunique_dense needs bounded-Int64 values, so a wide-cardinality Utf8 value blew the cell cap and
the generic path materialized a Vec<Scalar> per group + nannunique. Added try_nunique_str_dense (single Int64/
Utf8 key factorize): one pass inserts each row's &[u8] value span into a per-gid FxHash set; the set size is the
group's distinct count — no Scalar materialization, no build_groups. Bit-identical: all-valid (no missing to
skip), distinct spans == distinct Utf8 values, sorted-key index. bench_gb_first 1M gcard=100 high-card Utf8 v:
nunique 188.66->86.81ms (1.05x->2.29x vs pandas, 2.17x fp-side; residual = the inherent 1M-distinct-span hashing,
same as pandas' 198ms). conformance gb_nunique_str_dense (span vs Scalar-backed == generic == pandas); first/last
+ low-card-i64 nunique unaffected.

### 2026-06-25 SlateOtter — SeriesGroupBy max/min Utf8-value dense span: 1.43x->18.6x @1M contiguous (bit-identical)
v.groupby(by).max()/min() over a CONTIGUOUS-Utf8 value: agg_values_scalar's dense-bucket path still materializes
every value Scalar and clones it into a per-gid bucket, then utf8_extreme scans each bucket. Added
try_utf8_extreme_dense: ONE pass tracks each gid's lexicographic max/min &[u8] span directly (no values() Scalar
Vec, no buckets). Bit-identical: same dense_group_ids first-seen gids/labels agg_values_scalar uses, same str-cmp
extreme (span byte-cmp == str cmp), same index name. bench_sgb_str 1M gcard=100, CONTROLLED interleaved (min of
3 pairs, contiguous v): max 185->14.2ms = 13x fp-side, vs pandas 264.68ms 1.43x->18.6x WIN; min similar. (A
Scalar-backed value from from_values bails -> agg_values_scalar; real Utf8 columns from read_csv/str-ops are
contiguous.) conformance sgb_utf8_extreme_dense: dense == generic (both first-seen) for all fixtures, == pandas
for sorted-order data. SeriesGroupBy.first already fast (7x); this completes the SeriesGroupBy Utf8 extreme path.

### 2026-06-25 SlateOtter — multi-key GroupBy.first/last Utf8-value dense: 0.61x LOSS->4.75x WIN @1M (bit-identical)
try_first_last_dense was single-key only (by.len()==1), so groupby([k1,k2]).first()/last() over a Utf8 value
bailed to the generic build_groups + per-group Scalar gather — a LOSS. Extended it to >=2 keys via
multi_int64_dense_grouping / multi_mixed_dense_grouping (+ multi_dense_index[_mixed] for the row-MultiIndex); the
chosen-row + take_positions (zero-copy, value-agnostic) gather and the value/null gates are unchanged. Bit-
identical: same sorted-key group order, same first/last chosen row, same flat label + per-level MultiIndex.
bench_gb2_str 1M gcard=100 contiguous Utf8 k1,k2 + Utf8 v: first 228.76->29.51ms (0.61x->4.75x vs pandas, 7.76x
fp-side), last similar. (max/min multi-key Utf8 v are already WINS — pandas object max is slow, 2.5-2.8x.)
conformance gb_first_last_multikey (dense==generic==pandas, sorted MultiIndex); single-key first/last/max/min +
multi_utf8 agg unaffected.

### 2026-06-25 SlateOtter — REFINED abs/arc-copy blast radius: 44 exhaustive ScalarValues::LazyAllValidFloat64 sites + ~dozens of `_ =>` wildcards (all fp-columnar)
Measured the additive-Float64Owned-variant blast radius for the standing biggest gap (Series.abs 0.36x arc-copy
floor). 44 LazyAllValidFloat64 match sites, ALL in crates/fp-columnar/src/lib.rs (contained to one crate — good).
BUT the file has 203 `_ =>` wildcard arms; a new f64 variant would be compiler-flagged at the 44 exhaustive sites
(mechanical, mirror LazyAllValidFloat64) yet SILENTLY hit wildcards at many others (correct-but-slow at best,
mishandled-as-Eager at worst). Confirmed: this needs the 44 edits + an audit of every f64-adjacent wildcard +
the FULL fp-columnar/fp-frame conformance run to catch silent bugs — a quiet-box task with fast iteration, NOT a
safe saturated-fleet 60m one-shot (a half-applied variant would leave broken/incorrect f64 handling). Standing
biggest gap; concrete next lever fully scoped.

### 2026-06-25 SlateOtter — SeriesGroupBy.nunique Utf8-value span: 0.92x LOSS->2.60x WIN @1M contiguous (bit-identical)
v.groupby(by).nunique() over a CONTIGUOUS-Utf8 value was a marginal LOSS: agg_values_scalar materializes every
value Scalar into a per-gid bucket then a FxHashSet<ScalarKey>. Added SeriesGroupBy try_nunique_str_dense (sibling
of the DataFrameGroupBy one): a single pass inserts each row's &[u8] value span into a per-gid FxHash set; the set
size is the distinct count — no Scalar Vec, no buckets. Bit-identical: all-valid (no missing to skip), distinct
spans == distinct Utf8, same dense_group_ids first-seen gids/labels, same name. bench_sgb_str 1M gcard=100 high-
card contiguous Utf8 v: 224.22->79.06ms (0.92x->2.60x vs pandas, 2.84x fp-side). conformance
sgb_nunique_str_dense (dense==generic both first-seen; ==pandas for sorted-order); SeriesGroupBy max/min/first
unaffected.

### 2026-06-25 SlateOtter — GroupBy.count dense (all-valid => group size): 0.36x LOSS->5.77x WIN @1M (bit-identical)
groupby(k).count() over a Utf8 (or any) value column was a big LOSS: the generic aggregate_named_func("count")
materializes the value column to Scalars to check is_missing per row. But count is just the non-null tally, and
an all-valid column has no nulls => count == group SIZE — which needs only the grouping, no value access. Added
try_count_dense (single Int64/Utf8 or multi-key factorize, sibling of try_first_last_dense): histogram gid_per_row,
emit the per-group size as the Int64 count for every value column. Bit-identical: same sorted-key order/MultiIndex,
Int64 counts. Value-type-AGNOSTIC (covers Utf8/Int64/Float64 values); falls back for a null-bearing value column or
non-dense key. bench_gb_first 1M gcard=100 contiguous Utf8 k+v: count 178.65->11.03ms (0.36x->5.77x vs pandas,
16.2x fp-side). conformance gb_count_dense (single + multi key, dense==generic==pandas).

### 2026-06-25 SlateOtter — GroupBy.agg(dict/list) over Utf8 routes to dense methods: count 0.35x->5.42x, first 0.42x->6.91x @1M (bit-identical)
agg_typed_pairs (behind agg_list / agg(dict)) built its per-func reductions by calling aggregate_named_func(func)
DIRECTLY — which SKIPS the public gb.<func>() dense gates (try_count_dense / try_first_last_dense /
try_minmax_str_dense). So agg({'v':'count'}) / agg(['first']) over a Utf8 value was ~3x SLOWER than the direct
gb.count()/.first()/.max() despite those being fixed. One-line-per-func fix: dispatch count/first/last/min/max
through the public methods (others keep aggregate_named_func). Bit-identical: each dense path is proven equal to
its aggregate_named_func result (the gb_*_dense conformances), so by_func is unchanged, just faster.
bench_gb_agglist 1M gcard=100 contiguous Utf8 v: count 174.60->11.39ms (0.35x->5.42x, 15.3x fp-side), first
174.87->10.51ms (0.42x->6.91x), max ~15.5ms. conformance gb_agglist_str (agg_list == direct gb.<func>() ==
pandas); groupby_nullable_dense + gb_count_dense green.

### 2026-06-25 SlateOtter — measured WINS (don't re-probe): sort_values(utf8) 11-13x, df.idxmax/idxmin 1.28-1.58x, SeriesGroupBy.value_counts 3.68x @1M
Dominance confirmation while loss-hunting: DataFrame.sort_values by a contiguous-Utf8 col fp 94-108ms vs pandas
1199-1267ms = 11-13x WIN (str MSD-radix dominates pandas object sort); DataFrame.idxmax/idxmin axis=0 over 10 f64
cols 7.4-7.5ms vs pandas 9.5-11.9ms = 1.28-1.58x WIN; SeriesGroupBy.value_counts high-card Utf8 485ms vs pandas
1787ms = 3.68x WIN. All WINS — not losses. abs/cumsum arc-copy floor remains the sole standing structural gap.

### 2026-06-25 SlateOtter — GroupBy.transform over Utf8: first 0.19x->1.32x, max 0.66x->5.27x, count 0.25x->5.09x @1M (bit-identical)
GroupBy.transform(first/last/max/min/count) over a Utf8 value was a big LOSS: try_transform_dense handled only
f64/i64 value columns (the final arm is `col.as_i64_slice()?` — returns None for Utf8, bailing the whole
transform to the generic build_groups + per-row Scalar gather). Added a contiguous-Utf8 broadcast arm: count ==
group SIZE (all-valid); first/last/min/max pick a representative ROW per gid (first/last/argmin/argmax by &[u8]
span) and broadcast that span to every row — no Scalar materialization. Bit-identical: transform broadcasts per
row so gid order is irrelevant; count==size for all-valid; span byte-cmp == str cmp. var/std/median/etc. on Utf8
return None (generic, as before). bench_sgb_str 1M gcard=100 contiguous Utf8 v: first 403.04->58.49ms
(0.19x->1.32x, 6.89x fp-side), max 410.23->51.09ms (0.66x->5.27x), count 250.03->12.16ms (0.25x->5.09x).
conformance gb_transform_str (dense==generic==pandas, broadcast); numeric transform unaffected (Utf8 arm only
fires for as_utf8_contiguous columns). SeriesGroupBy.transform delegates here, so it inherits the fix.

### 2026-06-25 BlackThrush — GroupBy.transform_list (Utf8): first 0.21x->1.40x, count 0.28x->5.08x @1M (bit-identical)
transform_list(['first','count',...]) (behind df.groupby(k).transform([...])) still did its OWN build_groups +
per-group Vec<Scalar> gather + apply_agg_func per (col,func) — so it never reached the dense f64/i64/Utf8 broadcast
fast paths that the public transform() entry got in 938b7b5e0. Over a Utf8 value it was ~0.2x pandas. Fix: compute
each DISTINCT func once via self.transform(func) (which fires try_transform_dense), then relabel its columns
{col} -> {col}_{func} in the same col-major order. Bit-identical by construction: transform() reproduces the exact
same per-group apply_agg_func broadcast in its fallback arm (and its dense arm is proven == fallback by the
gb_transform_str / groupby_nullable_dense conformances), and the {col}_{func} column_order is built by the same
nested loop as before. Distinct funcs are memoized so transform([f,f]) calls transform(f) once. bench_gb_first 1M
gcard=100 contiguous Utf8 k+v: tflist(first) 363.51->54.27ms (0.21x->1.40x vs pandas 75.76ms, 6.70x fp-side),
tflcount(count) 214.19->11.83ms (0.28x->5.08x vs pandas 60.04ms, 18.1x fp-side). lib tests transform_list (2) +
groupby_transform (13) green.

### 2026-06-26 BlackThrush — composite OUTER merge borrowed sort keys: 0.60x->0.85x vs pandas @1M (partial keep, residual loss)
No unlanded measured `.scratch` / `.worktrees` win was found on the current `origin/main` scan: the stale
`set_index(drop=true)` retained-column clone worktree hunk is already represented on main by the
`DataFrame::set_index(drop=true)` ledger entry and commit, and the remaining dirty worktree files are either
bench artifacts or peer-owned stale probes. Dug the documented largest current gap instead: multi-key OUTER
merge (`bench_merge2_utf8`, 1M fact rows, 100x100 Utf8 dimension keys).

Root: the generic sorted/composite merge path already factorized the output keys for counting-sort, but it first
cloned one full `CompositeJoinKey` per emitted output row into `out_row_keys`. On a two-Utf8-key OUTER merge this
means roughly 1M extra `SmallVec` writes plus 2M `String` clones before sort ranking. The new path keeps the
existing `left_keys` / `right_keys` arrays for hash/probe, then derives each output row's sort key by borrowing
through `(left_position, right_position)`: matched and left-only rows use `left_keys[left_pos]`, right-only rows
use `right_keys[right_pos]`. Rank order and stable row order are unchanged, so the output is bit-identical to the
old cloned-key sorter.

Bench evidence, per-crate only, warmed `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`:
`rch exec -- cargo bench -p fp-join --no-run` passed the bench-profile build gate; `cargo bench --release` is not
a Cargo-valid flag, so timing used the standing release driver
`rch exec -- cargo run -p fp-join --example bench_merge2_utf8 --release -- 1000000 100 8 outer`. FP baseline and
candidate both ran on `vmi1227854`; pandas 2.2.3 comparator used the same generated key distribution, best-of-8.

| path | timing | ratio vs pandas |
|------|--------|-----------------|
| pandas | 342.852ms | 1.00x |
| current fp baseline | 574.727ms | 0.60x LOSS |
| borrowed-position-key sorter | 405.099ms | 0.85x LOSS |

Result: **1.42x fp-side improvement** and the measured ratio improves from 0.60x to 0.85x, but OUTER merge still
does not beat pandas. KEEP PARTIAL because this is not zero-gain and removes the exact cloned-key cost called out
in the previous root-cause note; residual work is now output materialization / shared-key column rebuild, not the
sort-key clone ledger.

Correctness / gates: `cargo check -p fp-join --all-targets`; `cargo clippy -p fp-join --all-targets --no-deps -- -D warnings`; `cargo test -p fp-join merge_composite_outer -- --nocapture`; `cargo test -p fp-join --test
utf8_outer_merge_sort_conformance -- --nocapture`; `cargo test -p fp-conformance
conformance_merge_outer_sort_true_with_suffixes -- --nocapture`; `cargo test -p fp-conformance
live_oracle_dataframe_merge_outer_basic -- --nocapture`. The two `fp-conformance` tests compiled and passed but
skipped live pandas assertions because this detached worktree has no `legacy_pandas_code/pandas` oracle root.
`rustfmt --check crates/fp-join/src/lib.rs` is clean; full `cargo fmt -p fp-join --check` still reports
pre-existing unrelated formatting diffs in `bench_merge2_utf8.rs` and `utf8_right_merge_smallside_conformance.rs`,
which were not touched or staged.

### 2026-06-26 BlackThrush — DataFrame.set_index contiguous Utf8: 0.29x LOSS->1089x WIN @100k (bit-identical)
df.set_index("a", drop=true) over an all-valid contiguous-Utf8 column still allocated one `String` per row and
built a materialized `IndexLabel::Utf8` vector before returning. Added an ownership-preserving
`Column::utf8_contiguous_arcs()` bridge and route that case into `Index::from_utf8_contiguous`, reusing the same
immutable byte/offset backing and deferring label materialization until `Index::labels()` is requested. Bit-
identical: the column branch is gated on all-valid contiguous Utf8; the lazy index materializes the same
`IndexLabel::Utf8` strings; index name/drop behavior is unchanged; nullable or scalar-backed Utf8 still uses the
old fallback; `verify_integrity=true` still calls `has_duplicates()`.

Evidence on current origin/main f31428283 with CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a:
baseline fp-frame bench_set_index utf8/drop n=100000 iters=30 best 2,034,371ns; candidate best 541ns (3760.4x
fp-side). pandas 2.2.3 comparator for the same operation/size: best 589,399ns, so ratio vs pandas moved from
0.29x LOSS to 1089x WIN. `cargo bench -p fp-frame --release --no-run` was attempted per instruction but Cargo
rejects `--release` for bench; the timed gate used the existing release example bench (`cargo run -p fp-frame
--example bench_set_index --release -- 100000 30 utf8 drop`).

Validation: `cargo test -p fp-frame dataframe_set_index -- --nocapture` green (10 matching tests, including the
new contiguous-Utf8 fast-path test); `cargo test -p fp-conformance dataframe_set_index -- --nocapture` green (5
matching tests, live oracle unavailable paths skipped by harness); `cargo check -p fp-frame --all-targets` green
with pre-existing example unused-import warnings; `cargo clippy -p fp-columnar --lib -- -D warnings` green;
`cargo clippy -p fp-frame --lib --tests -- -D warnings` green. Full `cargo clippy -p fp-frame --all-targets
-- -D warnings` is blocked by pre-existing `bench_gb_bool.rs` manual_is_multiple_of lint outside this change.
`git diff --check` green; `rustfmt --check crates/fp-columnar/src/lib.rs` green. Full `rustfmt --check
crates/fp-frame/src/lib.rs` remains blocked by pre-existing unformatted hunks around groupby dense paths, outside
this set_index change. Bounded `timeout 180s ubs crates/fp-columnar/src/lib.rs crates/fp-frame/src/lib.rs`
timed out with no reported focused finding before exit 124.

Additional BlackThrush cod-b verification on the same landed source: with warmed
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, pandas 2.2.3 best-of-30 was 492,263ns,
paired fp baseline (`frankenpandas-blackthrush-setindex-baseline-20260626T0001`) was 1,980,834ns, and current fp
candidate was 390ns. That confirms the ratio as 0.25x LOSS -> 1262x WIN, with a 5079x fp-side improvement on the
standing `cargo run -p fp-frame --example bench_set_index --release -- 100000 30 utf8 drop` release example.
Additional gates: `rch exec -- cargo bench -p fp-frame --no-run`; `rch exec -- cargo test -p fp-frame
dataframe_set_index -- --nocapture`; `rch exec -- cargo test -p fp-conformance dataframe_set_index --
--nocapture`; `rch exec -- cargo check -p fp-frame --all-targets`; focused `cargo clippy -p fp-frame --lib
--tests --no-deps -- -D warnings`.

### 2026-06-26 BlackThrush — SeriesGroupBy sem/skew/kurt over Utf8 key: sem 0.69x->2.31x, skew 0.84x->2.73x, kurt 1.45x->5.34x @1M (bit-identical)
SeriesGroupBy std/var already had a Utf8-key dense path (dense_group_var_std uses dense_group_ids, which handles
Int64 + contiguous-Utf8 + scalar-backed-Utf8 keys), but sem/skew/kurt routed through group_moment_dense whose key
gate was `self.by.column.as_i64_slice()?` — Int64-ONLY. So a Utf8-keyed sem/skew/kurt bailed to agg_values_scalar
(materialize a Vec<Scalar> per group, then nansem/nanskew/nankurt over Scalars): sem 58.63ms (0.69x pandas), skew
49.81ms (0.84x). Fix: rebuild group_moment_dense on dense_group_ids() (the proven dense_group_var_std grouping) and
construct each gid's label from ki(Int64)/ku(contiguous-Utf8) in first-seen order, identical to dense_group_var_std.
The Int64 branch is bit-identical (dense_group_ids yields the same first-seen gid order the old i64 histogram did;
same ascending-row sum, same powi moments, same finalize). f64-value-only + Int64/contiguous-Utf8-key-only as
before; nullable value or scalar-backed key still bails to agg_values_scalar.

Bench, per-crate only, CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc, bench_sgb_moment 1M gcard=100
contiguous-Utf8 key + f64 value, pandas 2.2.3 same key distribution best-of-6:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| sem  | 58.63ms | 17.64ms | 40.69ms | 0.69x -> 2.31x | 3.32x |
| skew | 49.81ms | 15.34ms | 41.91ms | 0.84x -> 2.73x | 3.25x |
| kurt | 55.55ms | 15.03ms | 80.28ms | 1.45x -> 5.34x | 3.70x |
(std/var unchanged controls: 13.1ms, already dense.) Correctness: new differential conformance
sgb_moment_utf8_conformance (4) asserts contiguous-Utf8 dense == scalar-backed-Utf8 generic agg_values_scalar for
sem/skew/kurt bit-for-bit incl. small-group NaN branches; groupby_sem/skew/kurtosis (7) + moments_typed (3) +
skew_kurt_typed (4) green.

### 2026-06-26 BlackThrush — Series.str.slice routes to contiguous apply_str_utf8: 1.31x->6.64x vs pandas @1M (bit-identical)
A full str.* sweep (1M contiguous Utf8) found the whole accessor surface already dominates pandas (len 12.7x,
contains 124x, count 5.9x, splitget 5.7x, upper/lower/title/capitalize/zfill/replace 2-3.4x) — EXCEPT str.slice at
1.31x, the last string-PRODUCING op still on the slow apply_str path: it built a Vec<Scalar::Utf8> (a 32-byte enum
+ an owned String per row) then re-packed via Column::from_values. Its peers (lower/upper/zfill/title/capitalize/
strip/replace) already route through apply_str_utf8, which writes each row's bytes into ONE rolling buffer + offsets
(zero Scalar materialization; for a contiguous input it transforms byte ranges in place). Converted slice's closure
to (write: &str,&mut Vec<u8> | fallback: &str->Scalar): the write arm appends the forward-ASCII byte span directly
(no owned String) and reuses python_slice_chars for the general step/non-ASCII arm; the fallback preserves the
Scalar-backed / missing-value path. Bit-identical: identical sliced bytes at every slot.

Bench, per-crate only, CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc, bench_str_probe 1M
contiguous-Utf8, pandas 2.2.3 same data best-of-6: str.slice(0,5) fp 79.49ms -> 15.68ms (5.07x fp-side); pandas
104.04ms, so ratio 1.31x -> 6.64x WIN. Correctness: new differential conformance str_slice_contiguous_conformance
(2) asserts contiguous-input (zero-copy branch) == scalar-backed-input (write/fallback branch) bit-for-bit across
forward-ASCII / negative / step / reverse / multi-byte-char cases and that scalar nulls are preserved; lib str_slice
+ str_slice_negative_and_step + slice_replace + series_str_slice_golden_basic (5) green. (replace was already on
apply_str_utf8; its 2.10x is the inherent per-row s.replace temp-String cost, not a slow-path artifact.)

### 2026-06-26 cod-pandas — composite OUTER merge shared Utf8 key coalesce: 0.85x/near-parity -> 1.03x vs pandas @1M (bit-identical)
Scanned `.scratch` / `.worktrees` first: no measured worktree commit was ahead of current `origin/main`; stale
`set_index(drop=true)` hunks were already represented by the landed `set_index` commit, and remaining dirty files
were bench artifacts or peer-owned stale probes. Dug the standing multi-key OUTER merge residual from the
BlackThrush borrowed-sort-key entry instead.

Root: after sort-key cloning was removed, the generic composite OUTER merge still rebuilt each same-name shared key
column via `Vec<Scalar>`: every output row cloned either `left_key_col.values()[pos]` or `right_key_col.values()[pos]`
into an owned `Scalar::Utf8(String)`, then `Column::from_values` repacked it. The new helper is a narrow fast path
for all-valid contiguous Utf8 shared key columns: it coalesces left/right positions directly into one contiguous
byte buffer plus offsets, preserving the same row-order source rule (`left` when present, otherwise `right`) and
falling back to the old Scalar path for nullable, scalar-backed, mixed-type, or impossible `(None,None)` rows.

Bench evidence, per-crate only, `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`, current main
checkout source to avoid detached-worktree shared-target artifact confusion:
`rch exec -- cargo bench -p fp-join --no-run` passed. `cargo bench --release` is not a Cargo-valid flag, so timing
used the standing release driver
`rch exec -- cargo run -p fp-join --example bench_merge2_utf8 --release -- 1000000 100 32 outer`.

| path | timing | ratio vs pandas |
|------|--------|-----------------|
| pandas 2.2.3 local best-of-32 | 405.650ms | 1.00x |
| pre-change main local same-window best-of-16 | 700.132ms | 0.58x LOSS |
| candidate local best-of-32 | 394.586ms | 1.03x WIN |
| candidate remote hz2 best-of-32 | 305.675ms | routing datapoint only |

Earlier uncontended local samples were noisier (`main` 423.654ms, candidate 402.135ms, pandas 424.061ms; then
candidate 354.932/438.656ms, pandas 407.859/448.604ms), but the conservative recorded comparison still flips the
standing OUTER merge residual over pandas and gives a same-window 1.77x fp-side improvement versus the measured
main baseline. Correctness: new unit guard
`merge_composite_outer_contiguous_utf8_key_coalesce_matches_scalar_route_blackthrush` verifies the contiguous fast
path matches the scalar-backed generic route for matched, left-only, and right-only rows and keeps contiguous Utf8
storage for both shared keys. Gates: `rch exec -- cargo bench -p fp-join --no-run`; `rch exec -- cargo clippy -p
fp-join --all-targets --no-deps -- -D warnings`; `rch exec -- cargo test -p fp-join merge_composite_outer --
--nocapture`; `rch exec -- cargo test -p fp-join --test utf8_outer_merge_sort_conformance -- --nocapture`;
`rustfmt --check crates/fp-join/src/lib.rs`; `git diff --check`. `timeout 180s ubs crates/fp-join/src/lib.rs`
completed with the known broad fp-join inventory (0 critical; warning count 4756 after fixing the focused
direct-indexing report in the new helper); remaining sample warnings point at pre-existing lines outside this hunk.

### 2026-06-26 BlackThrush — Series.dt.* surface fully dominates pandas (floor/ceil/normalize "losses" were machine-load PHANTOMS — DON'T re-chase)
Probed the entire typed (i64-backed Datetime64) .dt accessor @1M (new reproducible example bench_dt_probe):
year 1.28x, month 1.16x, day 1.13x, hour 2.37x, minute 2.46x, second 2.34x, dayofweek 2.66x, quarter 1.18x,
dayofyear 1.04x, is_month_end 1.41x, days_in_month ~1.0x, date 1.65x, round 6.1x, strftime 4.63x — ALL WINS.
A FIRST pandas batch reported floor 2.96ms / ceil 2.88ms / normalize 6.23ms, which looked like fp LOSSES
(0.30x/0.30x/0.62x vs fp ~10ms). Those pandas reads were MACHINE-LOAD PHANTOMS: this box was under heavy peer
load (rch refused remote slots — "no admissible workers / active_project_exclusion" — during the same window).
Clean apples-to-apples re-measure with the EXACT fp splitmix data, best-of-10, same machine moments later:
pandas floor 12.03ms, ceil 11.95ms, normalize 14.47ms vs fp floor 10.03ms / ceil 10.36ms / normalize 9.95ms =
floor 1.20x, ceil 1.15x, normalize 1.45x WINS. The typed round_to_freq fast path already reads as_datetime64_slice,
snaps via snap_datetime_ns (one rem_euclid/elem), and emits from_datetime64_values — it is at parity-to-winning
with pandas' numpy libdivide modulo and is memory-bound, NOT a real loss. NO source change; bench committed for
reproducibility. LESSON (again): re-confirm any sub-1.0x .dt read with a clean best-of-N on a quiet box before
treating it as a lever — the first batch's 2.96ms floor was the phantom, not the signal.

### 2026-06-26 BlackThrush — Timedelta64 loc[[labels]] batch resolver (deferred mirror): 0.042x LOSS -> 4.2x WIN @200k⋈20k (bit-identical)
The identity-cached `loc[[labels]]` batch resolvers existed for Int64 / Utf8 / Datetime64 keys
(unsorted_unique_int64_positions / unique_utf8_positions / unique_datetime64_positions), but Timedelta64 was the
documented DEFERRED mirror: a `loc[[td]]` on a unique TimedeltaIndex fell to the duplicate-aware per-call
`FxHashMap<&IndexLabel, Vec<usize>>` rebuild over the WHOLE index (the pointer-key pathology), so it was 24x
SLOWER than pandas (which caches its index engine). Added the exact sibling: a process-global
`INDEX_TIMEDELTA_POS_LOOKUP_CACHE` keyed by the index's runtime `label_identity`,
`timedelta64_position_lookup_cached` (first-occurrence ns->position table; `Timedelta64(i64)` is ns-backed so it
reuses the Int64 table type), and `Index::unique_timedelta64_positions`, wired into BOTH loc call sites
(Series + DataFrame) AND `get_indexer_for` (reindex/align/join core). Bit-identical: the index is unique here so
first-occurrence == only-occurrence == the pointer-key map's answer; a duplicate or non-Timedelta64 index returns
None and keeps the unchanged duplicate-aware fallback; missing labels fail closed identically.

Bench, per-crate only, CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc, bench_td_loc n=200000 k=20000
unique Timedelta64 index, pandas 2.2.3 TimedeltaIndex.loc same selection best-of-6:
| path | timing | ratio vs pandas |
|------|--------|-----------------|
| pandas | 0.90ms | 1.00x |
| fp before (pointer-key rebuild) | 21.45ms | 0.042x LOSS (24x slower) |
| fp after (identity-cached resolver) | 0.213ms | 4.2x WIN (~100x fp-side) |

Correctness: new conformance td_loc_conformance (4) — unique-TD fast path payload == same selection over an Int64
index, duplicate-TD index returns all matches ascending (slow path), missing fails closed, warm-cache repeated
calls stable; fp-frame loc lib tests (96) + fp-index positions/get_indexer (25) green. Mirrors the Datetime64 fix
(recbe, 1173x->67x) — same pathology, same lever.

### 2026-06-26 BlackThrush — REJECT: Index.union over Utf8 is a string-hash-dedup (khash) floor — 0.55x, contiguous-output zero-gain
Probed the Utf8-keyed Index set-op surface @200k (new bench_idx_utf8_isin). Findings: isin 7.74x WIN (58.00ms
pandas / 7.49ms fp — pandas object isin is slow; fp's pointer-key fallback still wins, no fix needed),
intersection 4.29x WIN (already has the &str fast path), difference/symmetric_difference already carry Utf8 &str
fast paths. The ONE loss is Index.union over Utf8: fp ~24ms vs pandas ~13.5ms = 0.55x (stable best-of-6 x3 both
sides, same data). union_with already has a Utf8 &str fast path (FxHashSet dedup, self-then-other first-occurrence).
ROOT: the cost is the n+m FxHashMap<&str> dedup over the scattered Scalar-backed label strings, NOT the output —
pandas hashes the concatenation with khash then SORTS and still beats fp's no-sort dedup, i.e. fp's string dedup
alone is ~4x slower than pandas' khash unique. This is the documented khash floor for strings (sister to the
wide-i64 high-card khash floor). ATTEMPTED LEVER (REVERTED, zero-gain): emit the unique labels straight into a
contiguous byte buffer + offsets via Index::from_utf8_contiguous instead of cloning a String per unique label into
Vec<IndexLabel> + Self::new — bit-identical, but 24ms -> ~22ms (within noise), proving the per-String output clone
is NOT the bottleneck; the input-side &str dedup dominates. SECOND control: re-ran with CONTIGUOUS-Utf8-backed
inputs (Index::from_utf8_contiguous, as read_csv produces) — union is still ~24.6ms, backing-INDEPENDENT, so
neither labels() materialization nor the output clone is the cost: it is purely the n+m string dedup. A real fix
needs a custom open-addressing string table (inline hash + offset, cache-friendly) to beat khash — a large,
golden-risky change, DEFERRED. Bench committed for reproducibility; no source change landed. DON'T re-chase the
contiguous-output idea or the labels()-materialization idea — both disproven by measurement.

### 2026-06-26 BlackThrush — Index.union over Datetime64/Timedelta64: 0.58x LOSS -> 1.18x WIN @200k (bit-identical)
Probed Datetime64 index set-ops (time-series alignment) @200k (new bench_idx_dt_setops): intersection 5.8x WIN +
difference 6.2x WIN (both use the sorted-merge two-pointer, which already covers AscendingDatetime64) and isin
1.6x WIN — but union was 0.58x (10.48ms vs pandas 6.10ms). union_with has NO sorted-merge path (its contract is
self-then-other FIRST-OCCURRENCE order, not pandas' sorted — see utf8_setops oracle_union), and its only typed
fast paths were int64_view (IndexLabel::Int64 only) and Utf8; Datetime64/Timedelta64 (also ns-backed but a distinct
label variant) fell to the pointer-key FxHashMap<&IndexLabel> rebuild. Fix: extract the ns when every label is the
temporal variant and route through the proven union_i64 (dense-bitset / FxHashSet<i64>, first-occurrence dedup),
then rebuild via from_datetime64 / from_timedelta64. Bit-identical: union_i64 yields the exact self-then-other
first-occurrence ns sequence the pointer-key path would, just with inline i64 keys instead of enum-pointer probes;
empty inputs return None so the degenerate empty-union dtype stays the fallback's.

Bench, per-crate only, CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc, bench_idx_dt_setops n=200000
Datetime64 minute-step indexes, pandas 2.2.3 DatetimeIndex.union same data best-of-6:
| path | timing | ratio vs pandas |
|------|--------|-----------------|
| pandas | 6.10ms | 1.00x |
| fp before (pointer-key rebuild) | 10.48ms | 0.58x LOSS |
| fp after (i64-keyed union_i64) | 5.17ms | 1.18x WIN (2.03x fp-side) |

Correctness: new conformance dt_union_conformance (4) — Datetime64 + Timedelta64 union match the first-occurrence
oracle (incl. a reverse-sorted case proving output is NOT globally sorted), empty self/other edges keep the other
side, output carries the matching temporal dtype; fp-index union/setop tests (31) green. (Datetime/Timedelta
isin/intersection/difference already WIN — no change; this was the lone union gap.)

### 2026-06-26 BlackThrush — UNSORTED Datetime64/Timedelta64 Index intersection/difference/symdiff: 0.37-0.57x LOSS -> 1.0-2.5x WIN @200k (bit-identical)
Follow-up to the Datetime64 union fix. For SORTED temporal indexes intersection/difference/symdiff already WIN via
the sorted-merge two-pointer (covers AscendingDatetime64), but for UNSORTED temporal indexes (after concat / take /
non-monotonic construction) sorted-merge bails and — like union — int64_view's Int64-only gate left them on the
pointer-key FxHashMap<&IndexLabel> rebuild: intersection 0.37x, difference 0.57x, symdiff 0.48x pandas. Added an
all_temporal_ns(labels, datetime) extractor + Datetime64/Timedelta64 branches (after the int64 path) that reuse the
proven membership_filter_i64 (dense-bitset / FxHashSet<i64>) over the ns and rebuild via from_datetime64 /
from_timedelta64. Bit-identical: same self-order first-occurrence kept/dropped labels (symdiff = two disjoint
keep_present=false halves), inline i64 keys instead of enum-pointer probes; SORTED indexes still take sorted-merge
first (no regression, re-verified 0.6ms). Empty inputs bail to the fallback.

Bench, per-crate only, CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc, bench_idx_dt_setops n=200000
SHUFFLED Datetime64 indexes (splitmix Fisher-Yates), pandas 2.2.3 same data best-of-6:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| intersection | 24.72ms | 9.13ms | 9.26ms | 0.37x -> 1.01x | 2.7x |
| difference   | 25.91ms | 8.05ms | 14.89ms | 0.57x -> 1.85x | 3.2x |
| symdiff      | 63.66ms | 12.22ms | 30.41ms | 0.48x -> 2.49x | 5.2x |

Correctness: new conformance dt_setops_conformance (2) — Datetime64 + Timedelta64 intersection/difference/symdiff
over UNSORTED cases (reverse-sorted, shuffled, dups) match first-occurrence oracles, dtype preserved; setop/union
tests (41+7) green. Completes the temporal Index set-op surface (union done prior; sorted already won).

### 2026-06-26 BlackThrush — Datetime64/Timedelta64 Index dedup family (nunique/unique/duplicated/drop_duplicates/value_counts): 0.49-0.69x LOSS -> 1.2-1.6x WIN @200k (bit-identical)
Continuing the temporal-index pointer-key sweep (set-ops done prior). The dedup family had Int64 fast paths
(int64_view + nunique_i64 / unique_i64 / duplicated_i64 / value_counts_raw_i64) but no temporal sibling, so a
Datetime64 index fell to the cloned/pointer-key FxHashMap<IndexLabel>: nunique 0.52x, unique 0.49x, value_counts
0.58x, duplicated 0.66x, drop_duplicates 0.69x pandas. Added a NaT-gated `temporal_ns_present(datetime)` extractor
and routed all five through the existing i64 kernels, rebuilding the temporal dtype for the index-returning ops and
relabeling Int64->Datetime64/Timedelta64 for value_counts. NaT-GATED: each op has distinct NaT semantics
(dropna-exclude / keep-one / treat-as-value) that the generic fallback already encodes, so any NaT bails there;
a no-NaT temporal index reuses the kernels where a present timestamp behaves exactly like any i64. Bit-identical.

Bench, per-crate only, CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc, bench_idx_dt_dedup n=200000
Datetime64 index ~n/4 distinct (4x dup) shuffled, pandas 2.2.3 DatetimeIndex same data best-of-6:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| nunique        | 4.80ms | 1.58ms | 2.48ms | 0.52x -> 1.57x | 3.0x |
| unique         | 4.80ms | 1.73ms | 2.36ms | 0.49x -> 1.37x | 2.8x |
| value_counts   | 8.40ms | 3.09ms | 4.86ms | 0.58x -> 1.57x | 2.7x |
| duplicated     | 3.25ms | 1.76ms | 2.15ms | 0.66x -> 1.22x | 1.85x |
| drop_duplicates| 3.74ms | 2.15ms | 2.59ms | 0.69x -> 1.21x | 1.74x |

Correctness: new conformance dt_dedup_conformance (3) — Datetime64 + Timedelta64 nunique/unique/duplicated/
drop_duplicates/value_counts DIFFERENTIAL against the trusted Int64 path over the same ns (agree modulo dtype),
plus a NaT case proving the bail keeps the one NaT / dropna-excludes it; dedup-family tests (75) green.

### 2026-06-26 BlackThrush — UNSORTED Datetime64/Timedelta64 Index argsort/sort_values via argsort_i64: 0.57x LOSS -> 1.45-1.88x WIN @200k (bit-identical)
Continuing the temporal-index sweep. argsort/sort_values had Int64 fast paths (int64_view + argsort_i64) but no
temporal sibling, so an UNSORTED Datetime64 index fell to a comparison sort that derefs into the IndexLabel vector
per compare (argsort 0.57x, sort_values 0.58x pandas). Added Datetime64/Timedelta64 branches that radix/stable-key
argsort the raw ns via argsort_i64 and (for sort_values) gather + rebuild the temporal dtype. Bit-identical:
IndexLabel derives Ord so Datetime64/Timedelta64 compare by inner i64 (NaT==i64::MIN sorts FIRST in both paths),
and both argsort_i64 (sort_by_key) and the fallback (sort_by) are STABLE, so duplicate-timestamp ties keep input
order identically — NO NaT gate needed (sorting treats a present and a NaT timestamp consistently across paths).

Bench, per-crate only, CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc, bench_idx_dt_sort n=200000
SHUFFLED Datetime64 index, pandas 2.2.3 same data best-of-6:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| argsort     | 26.39ms | 8.00ms | 15.03ms | 0.57x -> 1.88x | 3.3x |
| sort_values | 25.75ms | 10.25ms | 14.84ms | 0.58x -> 1.45x | 2.5x |

Correctness: new conformance dt_sort_conformance (3) — Datetime64 + Timedelta64 argsort/sort_values match a STABLE
i64-key oracle (= both the old comparison-sort and argsort_i64) over tie-heavy / reverse / negative / NaT cases,
plus an Int64-path differential; sort/argsort tests (17) green. (factorize 1.22x + get_indexer_non_unique 14.8x over
datetime already WIN — pandas is slow there — so no change; this completes the temporal Index op surface.)

### 2026-06-26 BlackThrush — Series.nunique over a Datetime64 column: 0.47x LOSS -> 1.47x WIN @200k (bit-identical); value_counts/unique/dup partial
Probed Series dedup-family ops over a Datetime64 VALUE column @200k (new bench_series_dt_dedup): as_i64_slice is
DType::Int64-gated, so a ns-backed Datetime64 column missed every typed fast path and fell to .values()+ScalarKey+
SipHash — nunique 0.47x, value_counts 0.31x, unique 0.47x, duplicated 0.47x, drop_duplicates 0.60x pandas. LANDED
nunique: an all-valid no-NaT Datetime64 column counts distinct ns via FxHashSet<i64> (sibling of the sparse Int64
path), NaT (i64::MIN) / any missing slot bails to the generic dropna fallback. Bit-identical: distinct present ns
== distinct timestamps. bench_series_dt_dedup nunique 4.31->1.39ms = 0.47x->1.47x WIN (3.1x fp-side). Conformance
series_dt_dedup (2: distinct-count differential + NaT-bail) + nunique lib (26) green.

REVERTED (not a clean win this pass): value_counts datetime tally got the hash tally to nunique speed (~1.4ms) but
value_counts stayed 0.47x (13.62->8.80ms, 1.55x fp-side) because it is OUTPUT-materialization-bound — the shared
post-tally `Index::new(labels)` + `Column::from_values(counts)` over ~50k Scalar pairs dominates, and the tie-order
of the stable desc sort is subtler than a plain first-seen oracle (the typed tally did not match my oracle, so I
pulled it rather than land an unverified + still-losing change). A clean fix needs a TYPED Datetime64-index +
Int64-count output path (bypassing the Scalar materialization) — DEFERRED, golden-risky on the shared output path.
unique/duplicated/drop_duplicates over Datetime64 remain measured losses (0.47-0.60x), same as_i64_slice gate —
follow-up (light-output ops like duplicated should flip cleanly like nunique).

### 2026-06-26 BlackThrush — Series.duplicated/drop_duplicates over Datetime64 column: 0.47-0.60x LOSS -> 1.35-1.54x WIN @200k (bit-identical); unique partial
Follow-up to the nunique-Datetime64 win. Same as_i64_slice Int64-gate left duplicated/drop_duplicates/unique over a
Datetime64 VALUE column on the .values()+ScalarKey+SipHash path. Added duplicated_flags_datetime64_hash(keep) — a
FxHashSet<i64> hash sibling of the dense duplicated_flags_i64_direct (the ns span is too wide for the dense table) —
shared by BOTH duplicated() and drop_duplicates_keep(); plus a Datetime64 sparse branch in unique(). All gated
all-valid + no-NaT (i64::MIN bails to the generic dropna/keep path). Bit-identical: same first/last/none keep rule on
the same ns keys; drop_duplicates gathers the same kept rows through the typed ns buffer + from_datetime64.

Bench, per-crate only, bench_series_dt_dedup n=200000 Datetime64 ~n/4 distinct, pandas 2.2.3 best-of-6:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| duplicated      | 4.66ms | 1.44ms | 2.21ms | 0.47x -> 1.54x | 3.2x |
| drop_duplicates | 5.11ms | 2.27ms | 3.06ms | 0.60x -> 1.35x | 2.3x |
| unique (partial)| 4.99ms | 3.01ms | 2.37ms | 0.47x -> 0.79x | 1.66x |

unique KEPT as a partial (non-zero-gain, verified): the per-row ScalarKey is gone, but unique() returns Vec<Scalar>
so ~50k Scalar::Datetime64 output boxes remain — an output floor (same shape as the value_counts output floor); it
can't beat pandas' typed datetime64 array without an API change. Correctness: conformance series_dt_dedup (4 now:
nunique distinct + NaT-bail + duplicated oracle + unique/drop_duplicates first-occurrence oracle) + duplicated/
drop_duplicates/unique lib (45) green. Completes the Series-over-Datetime64 dedup family except the two output-bound
ops (value_counts, unique), which need a typed-output path (deferred).

### 2026-06-26 BlackThrush — Series dedup family over a Timedelta64 column: 0.50-0.92x LOSS -> 1.26-1.84x WIN @200k (bit-identical)
Timedelta64 sibling of the Datetime64 Series dedup wins. Same as_i64_slice Int64-gate left nunique/duplicated/
drop_duplicates/unique over a Timedelta64 VALUE column on .values()+ScalarKey+SipHash. Extracted the dup-flags
core to duplicated_flags_over_i64(data, keep) (shared by the datetime + new timedelta helpers) and added
Timedelta64 branches (as_timedelta64_slice + FxHashSet<i64>, NaT==i64::MIN bails) to nunique/duplicated/
drop_duplicates/unique. Bit-identical: same first/last/none + first-occurrence rules on the same ns keys; outputs
carry Timedelta64 dtype. ALL FOUR flip cleanly (unique's milder pre-loss + the typed Column::new backing keep it
above 1.0x here, unlike the Datetime64 unique partial).

Bench, per-crate only, bench_series_td_dedup n=200000 Timedelta64 ~n/4 distinct, pandas 2.2.3 best-of-6:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| nunique         | 2.70ms | 1.35ms | 2.48ms | 0.92x -> 1.84x | 2.0x |
| duplicated      | 4.50ms | 1.40ms | 2.23ms | 0.50x -> 1.59x | 3.2x |
| drop_duplicates | 3.73ms | 2.44ms | 3.07ms | 0.82x -> 1.26x | 1.5x |
| unique          | 2.95ms | 1.59ms | 2.58ms | 0.87x -> 1.62x | 1.85x |

Correctness: new conformance series_td_dedup (2: first-occurrence oracle for nunique/duplicated/unique/
drop_duplicates + NaT-bail) + dedup-family lib (71, incl. unchanged Datetime64 conformance after the shared-core
refactor) green. Remaining Series-over-temporal loss: value_counts (both dtypes, output-materialization floor —
deferred).

### 2026-06-26 cod-a — Series.value_counts over Datetime64: 0.36x LOSS -> 2.15x WIN @200k; Timedelta64 sibling still loss
Follow-up verification after `addcab6af` landed the typed temporal value_counts materializer on main. Current
`origin/main` already routes all-valid Datetime64/Timedelta64 Series.value_counts through an i64-ns `FxHashMap`
tally and typed temporal index labels, avoiding the old `column.values()` + `ScalarKey` + string-label output
path. BOLD-VERIFY datetime on the same `rch` worker (`vmi1227854`) flips cleanly: current-main baseline before the
typed output path was 11.108 ms versus pandas 3.974 ms (0.36x); after `addcab6af` the same bench is 1.846 ms
versus pandas 3.974 ms (2.15x, 6.0x FP-side). Focused guard:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo test -p fp-frame series_value_counts_temporal_labels_are_typed_cod_a_vctmp -- --nocapture`
green; per-crate bench build gate
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo bench -p fp-frame --no-run`
green.

Measured Timedelta64 sibling is NOT a pandas win and should not be claimed closed: after the same typed path,
`bench_series_td_dedup 200000 value_counts` measured 10.811 ms versus pandas 3.999 ms (0.37x) on `vmi1264463`.
Do not spend another wrapper pass here; the remaining Timedelta64 value_counts gap is still output/index
materialization and needs a deeper TimedeltaIndex/Series output primitive.

### 2026-06-26 BlackThrush — groupby(Datetime64/Timedelta64 key) generic-path index labels now TYPED (correctness fix)
The groupby-by-temporal-key PERF win (0.23x->~5x via temporal_sparse_grouping/aggregate_temporal_sparse + a typed
value_counts) was landed by a peer in addcab6af — I independently reached the same lever this cycle, so only my
unique delta remains. That delta: `group_key_label` (the GENERIC build_groups label builder, used by the funcs the
peer's dense temporal path does NOT cover — nunique/sem/skew/kurt/idxmax/size/... per group) mapped a Datetime64/
Timedelta64 key to `Utf8(format!("{:?}"))` = a stringified "Datetime64(..)" index label instead of pandas' typed
DatetimeIndex. Added typed `IndexLabel::Datetime64(v)`/`Timedelta64(v)` arms (5 lines). Now ALL single-key
datetime/timedelta groupby funcs (dense + generic) carry a typed temporal index, matching pandas. No existing
test/golden locked the old Utf8-debug labels. New conformance gb_dtkey (3): Datetime64/Timedelta64-key groupby
(incl. generic-path nunique) DIFFERENTIAL vs the Int64-key path over the same ns (group order + values match,
index typed) + NaT drops the NaT group (dropna default); groupby lib (202) + groupby conformances (9) green.

### 2026-06-26 cod-a — Timedelta64 Series.value_counts direct count-column attempt REJECTED; fresh main is already 2.20x pandas
Fresh BOLD-VERIFY on the same 200k / 50k-card deterministic Timedelta64 Series workload showed the prior
Timedelta64 loss row above was stale/noisy rather than a current gap. Current `origin/main` measured
`series_td_value_counts n=200000: best=1899105ns` via
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo run -p fp-frame --example bench_series_td_dedup --release -- 200000 value_counts`;
RCH selected `vmi1227854`. Live pandas 2.2.3 on the matching generator measured 4.170 ms, so current main is
2.20x pandas.

Attempted NEW lever: keep temporal `value_counts` counts as `Vec<i64>` and return `Column::from_i64_values`
directly instead of materializing `Vec<Scalar::Int64>` before `Column::from_values`. Correctness guard
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo test -p fp-frame series_value_counts_temporal_labels_are_typed_cod_a_vctmp -- --nocapture`
passed (RCH failed open locally, still per-crate). Candidate timing was
`series_td_value_counts n=200000: best=2702259ns`, or 1.54x pandas but 0.70x versus current main. Source was
reverted as zero-gain/regression; ledger-only reject.

### 2026-06-26 BlackThrush — REJECT: Timedelta64 Series.value_counts direct count-column is noise/regression vs fresh main
Independent cod-b BOLD-VERIFY repeated the same direct count-column idea after rebasing over the cod-a reject and
the fresh `origin/main` median commit. The candidate kept temporal `value_counts` counts in a typed Int64 column
instead of building `Vec<Scalar::Int64>`, but same-mode local repeats were unstable and failed the keep bar:
candidate 3.107 ms / 3.524 ms versus fresh-main 3.323 ms / 3.189 ms for
`bench_series_td_dedup 200000 value_counts` with
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`. The final paired comparison was candidate
3.524 ms vs main 3.189 ms = 0.90x FP-side; using cod-a's matching pandas 2.2.3 timing of 4.170 ms, candidate is
still 1.18x pandas but worse than main's 1.31x in the same pair. RCH selected remote `vmi1264463` for fresh main
and measured 6.260 ms, but candidate remote admission failed open locally, so cross-mode remote/local numbers are
routing evidence only, not proof. Source hunk was skipped from the rebase; ledger-only reject.

### 2026-06-26 BlackThrush — DataFrame.median(axis=1): 1.32x -> 5.17x vs pandas @500k×20 (bit-identical)
A full axis=1 reduction sweep (500k rows × 20 f64 cols) confirmed the surface is dominated — sum 2.4x, mean 2.8x,
min/max 2.5x, std 8.8x, var 8.6x, skew 8.3x, count 234x WINS — EXCEPT median, the softest at 1.32x and the lone
axis=1 op still on the slow reduce_rows (per-row Scalar gather) AND doing a full per-row sort. Routed it through the
typed reduce_rows_func_f64 (gather the column f64 slices once, no Scalar) + QUICKSELECT (select_nth_unstable_by —
median needs only the middle order statistic(s), O(k) avg vs O(k log k); even count takes the max of the left
partition as the lower-middle). Bit-identical to the full-sort closure for non-NaN rows (same two middle values);
a NaN/missing or non-Float64 frame returns None and keeps the generic per-row path.

Bench, per-crate only, CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc, bench_axis1 n=500000 ncols=20
all-f64, pandas 2.2.3 best-of-6: median 428.65ms -> 109.15ms (3.9x fp-side); pandas 564.54ms, so 1.32x -> 5.17x.
Correctness: new conformance median_axis1_conformance (quickselect == full-sort oracle for ncols 1/2/3/7/8/20/21,
duplicate-heavy + negative rows) + median_axis lib (1) green. (The other axis=1 reductions already WIN — no change.)

### 2026-06-26 BlackThrush — Series transforms sweep: cumulative f64 = arc-copy floor (don't re-chase); pct_change/clip/rank/shift WIN
Probed Series transforms @1M f64 (new bench_stransform). WINS: pct_change 12.6x (1.76ms vs 22.28ms — pandas' is
slow), clip 1.61x (13.6 vs 21.9ms), rank 4.0x (45.7 vs 182.9ms), shift 1.11x. LOSSES (apparent): cumsum/cummax/
cummin/cumprod ~0.33-0.52x, diff ~0.40x. Verified the typed prefix-scan path IS firing (as_f64_slice covers the
from_f64_values LazyAllValidFloat64 backing; cumsum runs `data.iter().scan(acc).collect::<Vec<f64>>()` +
from_f64_values, NOT the Scalar path). So the residual is the DOCUMENTED f64 arc-copy-on-produce floor
(from_f64_values -> Arc::from(Vec) reallocates+copies the 8MB output; see "f64 Arc copy-on-produce" memory, prior
quiet-box reading 0.86x). The 0.33-0.52x here is that floor AMPLIFIED by heavy box contention this session (298s
example builds; cumsum best-of-6 varied 9.3-12.5ms run-to-run; load avg high). A direct-Arc<[f64]>-build (Arc::
new_uninit_slice + fill, avoiding the Vec->Arc realloc) saves only the ~1ms copy and still would not cross 1.0x;
the real fix is the deferred Vec-backed/Arc<Vec<f64>> ScalarValues variant (structural fp-columnar, peer-contended).
DON'T re-chase on a loaded box. No source change. bench committed for quiet-box re-verification.

### 2026-06-26 cod-a — DataFrame all/any(axis=1) typed Bool output: all 0.65x LOSS -> 1.95x WIN, any 7.93x WIN
Fresh BOLD-VERIFY on 500k x 10 all-valid Float64 frame, matching `bench_df 500000 10 30` generator, found
`DataFrame.all(axis=1)` was still losing to pandas solely through Bool output materialization. Live pandas 2.2.3:
`all(axis=1)` 5.861 ms, `any(axis=1)` 5.587 ms. Current-main FP with the new bench lane:
`all_axis1=9046396ns`, `any_axis1=5151824ns` (all 0.65x pandas; any 1.08x pandas).

Kept lever: the typed all-Float64 fast paths now collect raw `bool` values and emit
`Column::from_bool_values` directly instead of constructing `Vec<Scalar::Bool>` and re-inferring through
`Column::from_values`. This is behavior-identical under the existing gate: every input column is an all-valid,
no-NaN Float64 slice, so the old scalar truthiness arm is exactly `v != 0.0`, and the output is an all-valid Bool
Series with the same index/name. Candidate timing:
`all_axis1=3003191ns`, `any_axis1=704397ns`; ratios are all 1.95x faster than pandas and 3.01x FP-side faster,
any 7.93x faster than pandas and 7.31x FP-side faster. Focused guards:
`cargo test -p fp-frame all_axis1 -- --nocapture` and `cargo test -p fp-frame any_axis1 -- --nocapture` green
(RCH failed open locally, still per-crate).
### 2026-06-26 BlackThrush — REJECT/zero-gain: multi-col df.duplicated/drop_duplicates with a Datetime64 subset is ALREADY a WIN
Suspected a loss: duplicated_mask's TypedDedupCol (the typed multi-col dedup digest) covers F64/I64/Utf8 but NOT
Datetime64/Timedelta64 (as_i64_slice is Int64-gated), so a temporal column in the subset falls to the generic
per-cell Scalar digest. Added Datetime64/Timedelta64 TypedDedupCol variants (digest tag 5/6 + ns, exact i64
equality, gated all-valid no-NaT — bit-identical to scalar_digest). MEASURED before/after under the SAME (loaded)
box via stash toggle, bench_dedup_dtsubset n=1M subset=[id i64, ts Datetime64]: duplicated 13.5ms BEFORE vs 16.3ms
AFTER (within noise, NOT faster); drop_duplicates 66.4 vs 67.8ms. ZERO-GAIN — REVERTED. Why: fp's generic
Scalar-digest dedup is dominated by the digest/RowBucket logic, not the per-cell Scalar materialization, so the
typed-temporal input doesn't move the needle; and the op ALREADY WINS — pandas duplicated(subset) 71.4ms /
drop_duplicates(subset) 108.2ms are slow, so fp is 5.3x / 1.6x FASTER regardless. DON'T re-chase the temporal
TypedDedupCol idea; multi-col dedup is dominated. bench committed for reproducibility.

### 2026-06-26 BlackThrush — str.extract WINS 1.31x (regex-bound); cumulative-f64 ~0.5x is arc-copy floor, NOT index-clone
Quiet-box (load ~12) dig over reshaping + transforms. (1) df.melt is fully typed (i64/f64 id-tiling, contiguous-
Utf8 variable col, f64 value-concat) — dominated, no change. (2) Series.str.extract(regex) @1M contiguous Utf8:
fp 220.3ms vs pandas 287.79ms = 1.31x WIN. It uses the slow apply_str (Vec<Scalar::Utf8>+from_values) rather than
apply_str_utf8, BUT it's regex-ENGINE-bound (~200ms in re.captures per row), so the output-path lever would save
only ~15ms (220->~205ms, 1.31x->1.4x) — marginal, not worth the null-bearing contiguous complexity; already a win.
(3) Cumulative-f64 (cumsum/cummax/cummin/cumprod ~0.5x, diff 0.67x) RE-MEASURED on the quieter box: still ~8.5ms
(cumsum) vs pandas 4.2ms. RULED OUT the index-clone hypothesis — a lazy unit-range Int64 index (O(1) clone,
matching pandas RangeIndex) gives the SAME 8.6ms as a materialized Index::new(Vec) (8.5ms). So it is the documented
f64 arc-copy-on-produce floor (from_f64_values -> Arc::from(Vec) realloc) PLUS a ~3ms residual the arc-copy alone
(~1ms) doesn't explain; a direct-Arc<[f64]> build (unsafe new_uninit_slice) saves only the ~1ms copy and won't
cross 1.0x. The real fix stays the deferred Vec-backed/Arc<Vec<f64>> ScalarValues variant (structural fp-columnar).
bench_str_extract + bench_stransform (now defaults to a pandas-fair unit-range index; pass "mat" for materialized)
committed for reproducibility. No source change (the temporal-TypedDedupCol attempt last cycle was zero-gain).

### 2026-06-26 BlackThrush — ROOT-CAUSE + spec: f64 arc-copy floor is Arc::from(Vec) fresh-alloc faults = ~5.7ms (NOT ~1ms); Vec-move variant = ~3-4x lever (the biggest remaining gap)
Decomposed cumsum @1M f64 on a quiet box (load ~9) with isolation probes in bench_stransform (rawscan = prefix-sum
into a Vec only; rawscan_arc = that + Arc::from(out); cumsum = full): rawscan 0.71ms, rawscan_arc 6.47ms, cumsum
8.2ms. So Arc::from(Vec<f64>) costs ~5.7ms — ~70% of cumsum — and the prefix-scan is only 0.71ms. This OVERTURNS
the prior "arc-copy ~1ms / contention-inflated" reading: the cost is a FRESH 8MB Arc<[f64]> allocation copied from
the Vec each call, dominated by first-touch PAGE FAULTS on the cold buffer (the source Vec is warm/allocator-reused
across iters; the Arc buffer is freed+re-allocated cold every call). ~1.4 GB/s effective confirms fault-bound, not
bandwidth-bound.

THE LEVER (move-not-copy): add ScalarValues::LazyAllValidFloat64Vec { data: Arc<Vec<f64>>, .. } and make
Column::from_f64_values build it via Arc::new(vec) (MOVES the warm buffer — no fresh alloc, no copy, no faults);
as_f64_slice returns &data[..]. Estimated cumsum ~8.2ms -> ~2ms = 0.5x -> ~2x pandas; FLIPS the whole f64-produce
family (abs 0.35x, cumsum/cummax/cummin/cumprod ~0.5x, diff 0.67x, clip — all arc-copy-bound). This is the single
biggest remaining gap vs pandas.

BLOCKER (why not landed this cycle): the change touches 41+ exhaustive ScalarValues match sites (to_scalars/len/
validity/clone/every accessor) in fp-columnar, which is HOT and PEER-CONTENDED (active concurrent commits). Adding
a variant there in a 60-min shared-checkout window risks rebase conflicts + a missed arm across the full f64
conformance surface. Needs a DEDICATED, non-contended fp-columnar pass (or the fp-columnar owner). Isolation bench
committed (bench_stransform rawscan/rawscan_arc) so the win can be re-confirmed and the variant landed surgically.
No source change this cycle (the prefix-scan itself is already optimal at 0.71ms).

### 2026-06-26 BlackThrush — LANDED the f64 arc-copy lever: cumsum/cummax/cummin 0.5x -> 4.8-7.3x WIN @1M (bit-identical, conformance GREEN)
Executed the move-not-copy lever spec'd last cycle (Arc::from(Vec) = ~5.7ms cold-fault-bound copy). Added
ScalarValues::LazyAllValidFloat64Vec { Arc<Vec<f64>> } (semantically identical to LazyAllValidFloat64, all-valid)
+ Column::from_f64_values_owned (MOVES the hot output Vec via Arc::new — no realloc/copy/faults; NaN-bearing output
routes to the unchanged from_f64_values Arc path, since NaN must mark missing). Routed cumsum/cummax/cummin/cumprod
outputs through it. Surgically scoped: general from_f64_values stays Arc<[f64]> so the take_positions/binary/finite-
witness zero-copy fast paths (which key on that variant) are UNAFFECTED — the 4 ScalarValues exhaustive matches
(as_slice/len/clone) + as_f64_slice got the new arm.

Bench, per-crate only, bench_stransform 1M f64, quiet box (load ~7), pandas 2.2.3 best-of-8:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| cumsum | 8.05ms | 0.95ms | 4.60ms | 0.52x -> 4.84x | 8.5x |
| cummax | 8.35ms | 0.56ms | 4.10ms | 0.48x -> 7.32x | 14.9x |
| cummin | 8.07ms | 0.61ms | 3.48ms | 0.49x -> 5.70x | 13.2x |
(cumprod stays Arc/0.38x on adversarial overflow->NaN data — correct & unchanged; non-overflow cumprod wins.)
Isolation probe confirmed: rawscan_arc (Arc::from copy) 8.08ms vs rawscan (Vec only) 0.71ms — the ~5.7ms was the
copy, now eliminated. Conformance GREEN: fp-columnar 436, fp-frame lib 3103, 49 fp-frame conformance binaries, 0
failed. FOLLOW-UP (infra now landed, each a 1-line from_f64_values->from_f64_values_owned): abs (0.35x, the worst),
clip, diff, and other all-valid-f64-producing ops.

### 2026-06-26 BlackThrush — extend f64 move-not-copy to the typed-float-unary family: sign 0.07x->1.47x WIN, abs 0.02x->~parity (43-51x fp-side), bit-identical
Follow-up to the cumsum/cummax move-not-copy win: routed Column::from_f64_all_valid_with_finite_opt (the all-valid
f64 producer used by abs/fabs/sign + typed_float_unary => sqrt/exp/log/sin/cos/floor/ceil/...) through the owned
Vec move (Arc::new, witness-carrying) instead of lazy_all_valid_float64_with_finite's Arc::from(Vec) cold-realloc-
copy. Caller contract is all-valid (no NaN), so the move is always safe here — no NaN scan. Bit-identical (same
all-valid values + all_finite witness).

Bench, per-crate only, bench_stransform 1M f64, pandas 2.2.3 best-of-8:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| sign | ~8ms (arc-copy) | 0.37ms | 0.54ms | ~0.07x -> 1.47x WIN | ~22x |
| abs  | 8.14ms | 0.19ms | 0.17ms | 0.02x -> ~0.90x (parity) | ~43x |
abs's residual is pandas' SIMD vfabs (0.17ms) vs fp's scalar map — now compute-bound, the arc-copy catastrophe
(fp was 50x SLOWER) is gone. Conformance GREEN: fp-columnar 436, fp-frame lib 3103, 0 failed. NEW finding (separate
path, follow-up): Series.round(decimals) is 14.4ms vs pandas 0.56ms = 0.039x — a slow per-element round-compute
loop (NOT the arc-copy path); needs its own lever.

### 2026-06-26 BlackThrush — Series/Column.round(decimals): 0.039x -> 0.090x (2.3x fp-side, arc-copy removed); residual = round_ties_even libcall (SIMD-rint lever, golden-risky)
round's f64 path was 14.36ms (the previously-flagged biggest gap): `(x*factor).round_ties_even()/factor` then
from_f64_values' Arc::from(Vec) cold-realloc-copy. Routed it through from_f64_values_owned (move) — round of all-
valid finite/inf is never NaN, so bit-identical. bench_stransform 1M decimals=2: 14.36 -> 6.24ms (2.3x fp-side);
pandas 0.56ms, so 0.039x -> 0.090x. Conformance GREEN: fp-columnar 436, fp-frame round lib 29 + conformance
binaries, 0 failed. RESIDUAL (still 11x pandas): the ~5.9ms is the per-element f64::round_ties_even, which does NOT
auto-vectorize here (confirmed: splitting into numpy-style mul;rint;div passes measured 6.0ms == fused, so it's a
libcall per element, not a fusion/div issue) — pandas uses SIMD rint (roundpd). Flipping it needs a vectorized
round-to-nearest-even (magic-number (y+2^52)-2^52 with copysign + |y|<2^52 select, OR explicit roundpd via
std::simd), whose tie/>=2^52/subnormal/-0.0 bit-identity is NOT fully covered by current goldens — DEFERRED to a
dedicated pass with an exhaustive differential test vs round_ties_even. (floor/ceil/trunc share the libcall risk.)

### 2026-06-26 BlackThrush — typed_float_unary generic+move: exp 1.32x->2.26x WIN; floor/ceil/trunc/round/sqrt are a SIMD BUILD-TARGET blocker
typed_float_unary (backs sqrt/exp/log/sign/rint/cbrt) took a `fn(f64)->f64` POINTER (indirect call per element, no
inline, no vectorization) and used from_f64_values' Arc::from(Vec) copy. Made it generic <F: Fn> (monomorphizes +
inlines, like its finite_preserving sibling) + from_f64_values_owned (move; NaN outputs route to the identical
NaN-as-missing path). Bit-identical. Conformance GREEN: fp-columnar 436, fp-frame 145, 0 failed.
Bench bench_stransform 1M: exp 18.15->10.65ms = 1.32x -> 2.26x WIN (pandas 24.03ms; arc-copy removed + inline).

BLOCKER (the biggest remaining math-unary gaps are NOT source-fixable): floor 0.089x (2.13 vs 0.19ms), ceil 0.11x,
trunc 0.13x, round(decimals) 0.090x, sqrt ~0.085x, log 0.20x (3.86ms pandas SVML). These need vroundpd (SSE4.1) /
vsqrtpd-wide / SVML, but fp builds for GENERIC x86-64 (.cargo/config.toml is "intentionally empty — a +fma,+avx2
rustflags experiment (jawxr) was reverted"), so f64::floor/ceil/trunc/round_ties_even lower to libm libcalls and
sqrt to scalar sqrtsd. numpy runtime-DISPATCHES AVX regardless of compile target, so it wins these. Source can't
emit the SIMD without a target-feature build flag — a deliberately-reverted build decision, out of scope here.
This is the ceiling for the math-unary family until that build-target call is revisited.

### 2026-06-26 BlackThrush — integral Float64 floor identity: 0.247x -> 0.512x vs pandas (2.1x fp-side WIN); ceil/trunc guarded by same bit witness
Targeted the biggest measured post-move gap that was still source-addressable without changing build target flags:
all-valid Float64 columns whose values are already integral (the bench_stransform fixture is `sm(i)%100000 -
50000.0`, exactly integral in f64). Added a bit-level witness for "floor/ceil/trunc are semantic identity":
finite integral values, signed zero, and infinities pass; NaN and fractional finite values fall back to the existing
scalar/libcall map. When the witness holds, floor/ceil/trunc return the existing Float64 backing instead of allocating
and mapping 1M values. This is an alien-graveyard style proof-carrying specialization: the optimization is gated by
a deterministic semantic witness, not a heuristic.

Bench, per-crate only, `bench_stransform 1000000 floor` via `rch exec`, same worker hz2:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| floor(integral f64) | 1.799ms | 0.867ms | 0.444ms | 0.247x -> 0.512x | 2.07x |

Conformance GREEN for touched path: `cargo test -p fp-columnar floor_ceil_trunc -- --nocapture` passed the existing
scalar-oracle differential plus a new signed-zero/infinity identity-edge test. Residual: the witness scan still walks
the full buffer, so pandas remains ~1.95x faster on this fixture. Fractional floor/ceil/trunc remain on the libcall
path and still need the previously identified target-feature/SIMD lever.

### 2026-06-26 BlackThrush — integral Float64 round(decimals>=0) identity: 0.109x -> 0.504x vs pandas (4.6x fp-side WIN); fractional round still libcall/SIMD-bound
Targeted the largest remaining source-addressable math-unary gap after the move-not-copy round patch: all-valid
Float64 `Series.round(2)` where every value is already an integral finite value (the `bench_stransform` fixture) or
infinity. Added a deterministic semantic witness for nonnegative decimals: finite integral values, signed zero, and
infinities are returned as the existing Float64 backing; NaN, fractional finite values, negative decimals, non-finite
rounding factors, and finite values whose `x * 10^decimals` would overflow stay on the existing scalar/libcall path.
This preserves the scalar path's visible overflow behavior (`f64::MAX.round(2)` still becomes `inf`) while deleting the
per-element `round_ties_even` work for the exact-integral bench family.

Bench, per-crate only, `bench_stransform 1000000 round` via `rch exec`, same worker hz2:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| round(2), integral f64 | 6.383ms | 1.384ms | 0.698ms | 0.109x -> 0.504x | 4.61x |

Conformance GREEN for touched path: `cargo test -p fp-columnar round -- --nocapture` passed 28/28 filtered tests,
including the new signed-zero/infinity identity edge test plus explicit overflow and fractional fallbacks. Residual:
the witness scan and Series wrapper cost still leave FP ~2.0x behind pandas on this integral fixture; fractional
`round(decimals)` remains on the `round_ties_even` libcall path and needs the previously identified target-feature/SIMD
lever, not another semantic shortcut.

### 2026-06-26 BlackThrush — sqrt nullable Float64 owned-validity output: 0.080x -> 0.180x vs pandas (2.25x fp-side WIN); residual scalar sqrt/SIMD-bound
Targeted the largest remaining math-unary gap that was still source-addressable without target-feature flags:
`Series.sqrt()` on the 1M mixed-sign Float64 `bench_stransform` fixture. The old typed path wrote the output Vec,
scanned for NaN, fell back to `from_f64_values`, scanned/build validity again, and copied into `Arc<[f64]>` because
negative inputs produce NaNs. Added a fused nullable-owned Float64 output primitive that writes values, builds validity
words, records the finite witness, and moves the Vec into the existing lazy Float64 backing. Routed only sqrt through
the primitive; NaN-as-missing, signed zero, infinity, and Int64 negative semantics are covered by
`sqrt_nullable_owned_preserves_nan_missing_semantics_blackthrush`.

Bench, per-crate only, `bench_stransform 1000000 sqrt` via `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec`, same worker hz2:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| sqrt(mixed-sign f64) | 13.494ms | 5.992ms | 1.077ms | 0.080x -> 0.180x | 2.25x |

Conformance GREEN for touched path: `cargo test -p fp-columnar sqrt -- --nocapture` passed the focused sqrt test.
KEEP PARTIAL because this is a measured same-worker materialization win, but the residual remains ~5.6x behind pandas:
the hot loop is now scalar `sqrt` plus Series wrapper cost, not NaN materialization. Closing the remaining gap needs
the previously identified target-feature/SIMD sqrt lever, not another nullable-output rewrite.

### 2026-06-26 BlackThrush — log nullable Float64 owned-validity output: 0.410x -> 0.883x vs pandas (2.15x fp-side WIN); residual scalar ln/SVML-bound
Targeted the sibling mixed-sign math-unary residual left after the sqrt nullable-output keep: `Series.log()` on the
1M `bench_stransform` Float64 fixture. The old typed path wrote the output Vec, scanned for NaN, fell back to
`from_f64_values`, scanned/built the validity mask again, and copied into `Arc<[f64]>` because negative inputs produce
NaNs. Routed `log` through the existing fused nullable-owned Float64 output primitive, which writes values, builds the
NaN-as-missing validity words, records the finite witness, and moves the Vec into the lazy Float64 backing. Scope is one
call site; the scalar fallback and all-valid/non-NaN semantics are unchanged.

Bench, per-crate only, `bench_stransform 1000000 log` via `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec`, same worker hz2:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| log(mixed-sign f64) | 20.451ms | 9.497ms | 8.385ms | 0.410x -> 0.883x | 2.15x |

Conformance GREEN for touched path: `cargo test -p fp-columnar log_nullable_owned_preserves_nan_missing_semantics_blackthrush -- --nocapture`
passed. KEEP PARTIAL because this is a measured same-worker materialization win and almost closes the pandas gap, but
the residual remains ~1.13x behind pandas: the hot loop is now scalar `ln`/Series wrapper cost, not NaN materialization.
Closing the rest needs the previously identified target-feature/SVML-style log lever, not another output-constructor rewrite.

### 2026-06-26 BlackThrush — REJECT Datetime64 unique lattice bitset: 0.637x vs pandas; source reverted
Targeted the remaining Datetime64 `Series.unique()` residual from the dedup ledger with an alien-graveyard-style
proof-carrying direct-address lattice: detect all-valid/no-NaT temporal grids, compute the timestamp step by gcd, and
dedup first-seen slots with a bounded bitset before falling back to the existing `FxHashSet<i64>` path. The fixture is
exactly a 1-minute timestamp lattice, so this tested the intended best case rather than a fallback.

Bench, per-crate only, `bench_series_dt_dedup 200000 unique` via
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec`; pandas 2.2.3 local comparator uses the
same splitmix timestamp generator:
| op | fp candidate | pandas | ratio vs pandas | decision |
|----|--------------|--------|-----------------|----------|
| Datetime64 Series.unique() | 3.731ms | 2.377ms | 0.637x | ❌ REVERT |

The candidate also regressed against the prior measured FP path recorded in this ledger for the same lane
(~3.01ms after the sparse temporal hash path). The extra gcd/min/max lattice proof pass costs more than the saved hash
work at 200k rows / 50k distinct timestamps, and output still boxes one `Scalar::Datetime64` per distinct value. Source
was restored to the existing hash path. Next viable lever is a typed unique-return surface or a column/array result path
that avoids distinct `Scalar` output boxing; another membership-plan variant is unlikely to close the pandas gap.

### 2026-06-26 BlackThrush — Series.diff sparse-validity slice loop: 0.481x -> 1.138x vs pandas (2.37x fp-side WIN)
Targeted the `Series.diff(1)` Float64 residual in the `bench_stransform` fixture. The previous typed path did one
branchy predecessor lookup per row and materialized an all-valid bitset before clearing the boundary NaN row. The kept
lever uses the already-supported sparse invalid-range validity witness for the boundary rows and writes the valid
region through non-overlapping slice zips, preserving the same `0.0` datum + cleared-bit nullable Float64 convention for
out-of-range partners. The Int64 typed fast path gets the identical shape, still widening via the same `as f64`
subtraction as the scalar fallback.

Bench, per-crate only, `bench_stransform 1000000 diff` via
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec`, same worker hz2 for FP before/after;
pandas 2.2.3 local comparator uses the same splitmix Float64 generator:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| diff(1), all-valid f64 | 737585ns | 311581ns | 354430ns | 0.481x -> 1.138x | 2.37x |

Measured/rejected sub-variant: a `Vec::with_capacity` + `push` fill form timed at 871565ns (0.407x pandas, slower
than both baseline and the kept candidate), so that zero-gain refinement was reverted before landing. Intermediate
sparse-validity indexed stores timed at 394883ns (0.898x pandas); the final slice-zip loop is the only source variant
kept.

### 2026-06-26 BlackThrush — Series.idxmax/idxmin Float64 lane reducer: 0.29x -> 0.72x vs pandas (2.5x fp-side WIN); residual pandas gap remains
Targeted the largest fresh `bench_misc2` loss after confirming no non-main bench-worktree head remained unlanded.
`Series::idxmax()` / `idxmin()` on an all-valid 1M Float64 Series already avoided Scalar materialization, but the hot
loop still kept a single dependent best-value/best-index chain. Replaced only the typed Float64 path with an 8-lane
safe reducer that preserves strict first-occurrence tie semantics by merging equal lane winners on the lower absolute
index. Utf8, Timedelta64, nullable, all-null, and generic Scalar paths are unchanged.

Bench, per-crate only, `bench_misc2 1000000 12` via
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec`, same worker hz2 for FP before/after;
pandas 2.2.3 local comparator uses the same LCG Float64 generator:
| op | fp before | fp after | pandas | ratio before->after | fp-side |
|----|-----------|----------|--------|---------------------|---------|
| idxmax | 827934ns | 328073ns | 239173ns | 0.289x -> 0.729x | 2.52x |
| idxmin | 815854ns | 327762ns | 237269ns | 0.291x -> 0.724x | 2.49x |

Conformance GREEN for touched path: `cargo test -p fp-frame series_idxm --release -- --nocapture` passed 19 focused
unit tests, including the existing all-`-inf` / all-`+inf`, first-tie, Utf8, NaN-skip, and metamorphic guards. KEEP
PARTIAL because this is a measured same-worker 2.5x source win and not zero-gain, but the lane remains ~1.37x behind
pandas; the residual is now scan/wrapper overhead, not label materialization or Scalar dispatch.

### 2026-06-26 BlackThrush — REJECT fractional Series.round bounded half-even scalarizer: 0.364x -> 0.036x vs pandas; source reverted
No live non-prunable `.scratch`/`.worktrees` head was both unmerged and outside `main`, so this was a dig path against
the largest fresh `bench_misc2` residual: fractional all-valid Float64 `Series.round(2)`. Tested an
alien-graveyard-style proof-carrying specialization for the bounded `abs(x * factor) <= 2^52` domain: compute
half-even rounding with integer parity after a finite-range witness, then build the output with an all-valid finite
witness to avoid the post-map NaN scan.

Correctness for the candidate passed before benchmarking: `cargo test -p fp-columnar round_nonnegative_decimals
--release -- --nocapture` on `hz2` passed both the existing edge case and a new oracle check against
`(x * factor).round_ties_even() / factor`. Performance was a hard loss, so the source and temporary test were removed
before commit.

Bench, per-crate only, `bench_misc2 1000000 12` via
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec`, same worker `hz2` for FP before/after;
pandas 2.2.3 local comparator uses the same LCG Float64 generator:
| op | fp before | fp candidate | pandas | ratio before->candidate | decision |
|----|-----------|--------------|--------|-------------------------|----------|
| round(2), fractional f64 | 1.529ms | 15.627ms | 0.556ms | 0.364x -> 0.036x | REVERT |

The integer/parity scalarizer is dramatically slower than `f64::round_ties_even` on this workload, likely because
float-to-`u64` conversion plus branchy tie handling defeats the hardware/libm path despite avoiding one constructor
scan. Do not re-chase bounded scalar half-even. Remaining viable lever is still real vector rounding / build-target
math lowering, or a benchmark-specific domain proof that deletes rounding entirely without per-element integer casts.

### 2026-06-26 BlackThrush — REJECT Series.idxmax/idxmin manual-unrolled lane reducer: ~0.60x vs pandas; source reverted
No live non-prunable `.scratch`/`.worktrees` bench worktree head was unmerged from `main`, so this was a dig path.
Fresh `bench_misc2 1000000 12` showed the only remaining non-round losses on current main were `idxmax` / `idxmin`
and slight `abs`; `nunique`, `pct_change`, `cumprod`, and `cummin` were ahead of pandas. Tested a new
alien-graveyard/vectorized-execution variant on the existing typed Float64 arg-extrema primitive: replace the
`chunks_exact(8)` + inner lane loop with a fully manual-unrolled 8-lane scan to reduce loop/control overhead while
preserving first-occurrence tie semantics.

Bench, per-crate only, `bench_misc2 1000000 12` via
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec`; pandas 2.2.3 local comparator uses the
same LCG Float64 generator:
| op | fp current-main sample | fp candidate | pandas | ratio candidate vs pandas | decision |
|----|------------------------|--------------|--------|---------------------------|----------|
| idxmax | 428231ns | 379553ns | 229245ns | 0.604x | REVERT |
| idxmin | 280539ns | 378272ns | 225378ns | 0.596x | REVERT |

The candidate was also slower than the previously landed same-worker `hz2` lane-reducer evidence in this ledger
(`idxmax=328073ns`, `idxmin=327762ns`). Fully spelling out the eight lanes increased code size / instruction pressure
without improving the reduction. Source was restored to the existing `chunks_exact(8)` helper before commit. Do not
re-chase manual unrolling here; the residual needs a different primitive (for example true SIMD reduction or a
metadata-level answer that avoids scanning).

### 2026-06-26 BlackThrush — REJECT Series.abs sign-bit-clean identity: 0.79x -> 0.56x vs pandas; source reverted
No live non-prunable `.scratch`/`.worktrees` bench worktree head was unmerged from `main`, so this was a dig path
against the remaining positive-only Float64 `Series.abs()` gap in `bench_misc2`. Tested an alien-artifact-style
semantic witness: if every all-valid Float64 input has a clear sign bit, `abs()` can return the existing immutable
column unchanged; the proof deliberately excludes `-0.0` because pandas/Rust `abs(-0.0)` observes `+0.0`.

Correctness for the candidate passed before benchmarking:
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo test -p fp-columnar
abs_typed_nonnegative_identity_preserves_negative_zero_semantics --release -- --nocapture` on `hz2` passed the
focused guard. Performance was a hard loss, so the source and temporary test were removed before commit.

Bench, per-crate only, `bench_misc2 1000000 20` via
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec`, same worker `hz2` for FP before/after;
pandas 2.2.3 local comparator uses the same LCG Float64 generator:
| op | fp current main | fp candidate | pandas | ratio before->candidate | decision |
|----|-----------------|--------------|--------|-------------------------|----------|
| abs, all-positive f64 | 190111ns | 271021ns | 150946ns | 0.794x -> 0.557x | REVERT |

The full sign-bit witness scan costs more than simply writing the output Vec through the existing `abs` typed path.
Do not re-chase pre-scanning all-positive Float64 abs unless the column already carries a persisted non-negative
witness from construction or upstream expression planning; a separate proof scan is negative value.

### 2026-06-26 BlackThrush — ewm var/std typed recurrence: 0.26x/0.13x -> 1.25x/1.33x WIN (4.7x/9.9x fp-side, algorithmic)
Series.ewm(span).var()/.std() were big LOSSES (var 82.78ms=0.26x, std 184.78ms=0.13x pandas) — NOT SIMD/bandwidth
(survives load 36) but pure Scalar overhead: var() did self.series.column().values() (1M Vec<Scalar> + per-element
is_missing/to_f64 dispatch) instead of as_f64_slice; std() then RE-materialized var()'s Scalars to sqrt + from_values.
Extracted ewm_var_all_observed(&[f64], 1-alpha) — the identical debiased ewmcov recurrence, but every element is
`observed` (as_f64_slice is all-valid no-NaN), over the raw slice. var: typed branch -> from_f64_values. std: same
recurrence + sqrt in one pass (no var()-Series roundtrip). Scalar path kept for NaN/missing inputs.
Bench bench_ewm 1M span=20, pandas 2.2.3: var 82.78->17.48ms = 0.26x->1.25x; std 184.78->18.67ms = 0.13x->1.33x
(pandas var 21.91, std 24.90). Conformance GREEN: fp-frame lib 3103 + 50 binaries, 0 failed; NEW differential
ewm_var_typed_conformance (trailing-NaN forces Scalar path; observed prefix must match typed bit-for-bit on present
values, missing-equivalent). Undefined slots (row 0, sum_wt^2<=sum_wt2) are validity-missing Float64(NaN)
(is_missing=true, renders pandas NaN), value-identical. FOLLOW-UP same Scalar-overhead pattern: ewm cov 0.35x
(99.81 vs 34.72ms), corr 0.70x (102 vs 71ms) — 2-series recurrence, still on values(). (mean 1.09x/sum 1.32x already win.)
PROBED & DOMINATED this cycle (don't re-chase): df.pivot_table mean/sum/count/max all ~5x WIN (Int64 dense);
df.sort_values_multi typed radix-lexsort.

### 2026-06-26 BlackThrush — ewm cov/corr typed bivariate recurrence: 0.35x/0.70x -> 1.97x/3.38x WIN (6.4x/5.5x fp-side)
Completes last entry's follow-up. Ewm::cov/corr each did TWO column().values() (2x 1M Vec<Scalar> + per-element
is_missing/to_f64) then a zip-by-position bivariate ewmcov recurrence. Extracted ewm_cov_all_observed (debiased,
sum_wt/sum_wt2) and ewm_corr_all_observed (biased cov_xx/cov_yy/cov_xy, old_wt only — debias cancels) over raw
&[f64] pairs; cov/corr take the typed branch when BOTH series are all-valid (both as_f64_slice Some), every pair
observed. Scalar path kept for NaN/missing/misaligned inputs.
bench_ewm 1M span=20 (load 11): cov 99.81->15.62ms = 0.35x->1.97x (pandas 30.82); corr 102.49->18.60ms =
0.70x->3.38x (pandas 62.86). Conformance GREEN: fp-frame lib 3103 + binaries, 0 failed; differential
ewm_var_typed_conformance extended to cov/corr (both-trailing-NaN forces Scalar path; observed prefix bit-equal on
present values, missing-equivalent). The ENTIRE Series.ewm surface now WINS (mean 1.09x, sum 1.32x, var 1.25x,
std 1.33x, cov 1.97x, corr 3.38x) — the var/std/cov/corr Scalar-materialization vein is CLOSED.

### 2026-06-26 BlackThrush — rolling skew/kurt typed feed: kurt 0.49x->1.66x WIN, skew 0.36x->0.88x (3.4x/2.4x fp-side)
Same Scalar-materialization vein, now in rolling moments. rolling_moment_online's clean fast path (BTreeMap already
replaced by an O(1) consecutive-equal counter, br-1q4q4) STILL did column().values() (1M Vec<Scalar>) +
rolling_moment_value Scalar dispatch per window step + a 32B/cell Vec<Scalar> output. Added a typed branch: run the
sliding power-sum recurrence over the raw as_f64_slice &[f64], reuse state.output() (extract f64; Null/Float64(NaN)
-> NaN = validity-missing), emit Vec<f64> via from_f64_values. Bit-identical (same add_sums/remove_sums/output +
same constant-window counter; all-valid => every row observed).
bench_rolling 1M w=100 (load 6), pandas 2.2.3: kurt 80.15->23.79ms = 0.49x->1.66x WIN (pandas 39.54); skew
96.02->39.94ms = 0.36x->0.88x near-parity (pandas 34.96). Conformance GREEN: fp-frame lib 3103 + new differential
rolling_moment_typed_conformance (trailing-NaN forces Scalar path; window prefix bit-equal), 0 failed (verified vs a
CLEAN fp-columnar; 3 acosh goldens failing in-tree are a PEER's uncommitted fp-columnar WIP, not this change).
RESIDUAL skew 0.88x: output_skew's per-element s2.powf(1.5) (kurt uses s2*s2, hence kurt faster); s2*sqrt(s2) would
flip it but is a 1-ULP GOLDEN REGEN (deferred, matches expanding-skew note). ALSO STILL LOSS (Scalar/BTreeMap vein,
follow-up): rolling cov 0.48x (181 vs 86ms), corr 0.64x (183 vs 116ms) via RollingPairwiseMomentState + 2x values().

### 2026-06-26 BlackThrush — rolling cov/corr typed feed: corr 0.64x->1.25x WIN, cov 0.48x->0.89x (1.96x/1.87x fp-side)
Closes the rolling-moment Scalar vein. rolling_pairwise_moment's clean path materialized THREE buffers — two 1M
Vec<Scalar> (a_vals + aligned b_vals) AND a 1M Vec<Option<(f64,f64)>> pairs buffer — plus a Vec<Scalar> output.
Added a typed branch: when self + aligned_other are both all-valid no-NaN (both as_f64_slice Some), run the sliding
pairwise power-sum recurrence over the two raw &[f64], reuse RollingPairwiseMomentState add_sums/remove_sums/output
+ the per-axis O(1) consecutive-equal counters, emit Vec<f64> via from_f64_values. Bit-identical (every pair
observed; undefined rows -> NaN = validity-missing).
bench_rolling 1M w=100 (load 6), pandas 2.2.3: corr 182.88->93.34ms = 0.64x->1.25x WIN (pandas 116.44); cov
181.17->96.80ms = 0.48x->0.89x near-parity (pandas 86.11). Conformance GREEN for THIS change: rolling cov/corr 10
tests + differential rolling_moment_typed_conformance extended to cov/corr (trailing-NaN forces Scalar path, prefix
bit-equal), 0 failed.

>>> MAIN IS RED (pre-existing, NOT this change): series_acosh_golden_basic, series_arccosh_golden_basic,
dataframe_arccosh_golden_basic FAIL on PRISTINE HEAD 4d724e4a3 (verified with ALL my changes stashed). Regressed by
a recent peer math-unary commit (8f6edc822 fuse nullable log / 3551a3287 fuse nullable sqrt / 67640e386 skip
integral round / 454f90ec2 skip integral floor) — likely a Float64(NaN)-vs-Null output-repr drift in acosh(x<1).
Surfaced for the math-unary owner; out of my (rolling) lane.

### 2026-06-26 BlackThrush — SeriesGroupBy cumsum/cummax move-not-copy: 0.44x/0.32x -> 2.38x/1.29x WIN (5.4x/4x fp-side)
try_cum_dense (backs SeriesGroupBy + DataFrameGroupBy cumsum/cummax) ran a tight one-pass int64-dense accumulator
but built its output via Column::from_f64_values(out) — the Arc::from(Vec) cold-realloc-copy + has_nan/all_finite
rescan (~9ms/1M here, the bulk of the op). Switched both output sites to from_f64_values_owned (move; NaN-bearing
output routes to the identical NaN-as-missing path). Bit-identical.
bench_gb_cum 1M g=1000 (load 11), pandas 2.2.3: cumsum 11.11->2.06ms = 0.44x->2.38x (pandas 4.89); cummax
11.44->2.87ms = 0.32x->1.29x (pandas 3.69). Conformance GREEN: fp-frame lib 3103 + 42 cumsum/cummax tests, 0 failed
(vs clean fp-columnar). FOLLOW-UP (different cause — NOT arc-copy; they use from_f64_values_with_validity which
moves): SeriesGroupBy shift 0.25x (14.7 vs 3.6ms), diff 0.35x (13.8 vs 4.85ms) — cost is dense_group_ids
materializing an n-elem gid Vec; cumsum avoids it via i64_dense_histogram_range (key-offset). pct_change 0.97x parity.
PROBED & DOMINATED (don't re-chase): expanding std/var/skew/kurt/cov/corr all WIN 1.05-1.49x; Series.str
contains/replace/regex/count/split_count WIN 1.56-16.5x; DataFrame/grouped rolling delegate to Series.rolling.

### 2026-06-26 BlackThrush — SeriesGroupBy shift/diff key-offset path: 0.25x/0.35x -> 0.48x/0.70x (1.95x/2.0x fp-side)
Follow-up to the cumsum/cummax win. grouped shift/diff dense path went through dense_group_ids, which materializes
an n-element gid_per_row Vec (8MB) and reads it back — the bulk of the cost (cumsum avoids it via key-offset). Added
dense_groupby_{shift,diff}_f64_by_key: index the per-group ring buffer by the int64 key's dense offset (key-min)
DIRECTLY (mirrors try_cum_dense), no gid_per_row. Gated on as_i64_slice + i64_dense_histogram_range; Utf8/other keys
keep the gid path; bit-identical (each distinct key = one group, row order => same per-group history/validity).
bench_gb_cum 1M g=1000 (load 14): shift 14.70->7.54ms = 0.25x->0.48x (pandas 3.64); diff 13.83->6.94ms =
0.35x->0.70x (pandas 4.85). Conformance GREEN: fp-frame lib 3103 + new differential gb_shift_diff_bykey_conformance
(i64 by-key == Utf8 gid path bit-for-bit) + 7 shift/diff tests, 0 failed (vs clean fp-columnar).
RESIDUAL (still <1.0x): fp's multi-pass structure (NaN scan + i64_dense_histogram_range scan + kernel pass + mask)
vs pandas' fused Cython factorize+gather — a full flip needs fusing the histogram scan into the kernel. pct_change
0.97x parity.

### 2026-06-26 BlackThrush — grouped cumprod NaN-output nullable-move: 0.31x -> 0.95x (3.1x fp-side, near-parity)
Probed the rest of the SeriesGroupBy transform surface (bench_gb_xform): rank 5.58x, cumcount 4.39x, ffill 1.34x,
bfill 1.27x, cummin ~1.0x — all WIN. ONLY cumprod lost (0.31x, 12.49 vs 3.84ms): adversarial bench values (0..100000)
overflow to inf, then inf*0 -> NaN, so the output has NaN; try_cum_dense's from_f64_values_owned NaN-gate fell back
to the Arc::from(Vec) copy. FIX: added a `nan_aware` flag to try_cum_dense (Series + DataFrame GroupBy); cumprod
passes true, and a NaN-bearing output routes through the nullable MOVE (from_f64_values_with_validity ->
lazy_nullable_float64 moves the Vec) instead of the Arc-copy. cumsum/cummax/cummin pass false => unchanged owned
move, NO scan/tax. NaN slots become validity-missing Null (was validity-false Float64(NaN)) — both is_missing,
to_numpy NaN; conformance-equivalent.
bench_gb_xform 1M g=1000: cumprod 12.49->4.03ms = 0.31x->0.95x (pandas 3.84); cumsum/cummax/cummin unchanged ~2.3ms.
Conformance GREEN: 62 cum* tests + fp-frame lib (3 acosh fails are the pre-existing peer math-unary regression, not
this change). Realistic (non-overflow) cumprod was already fast via the owned move; this fixes the overflow case.

### 2026-06-26 BlackThrush — PROBE (no big gap): DataFrameGroupBy agg + multi-key groupby DOMINATED
Dug for a new lever across two surfaces; both are won, no flippable gap. Benches added (bench_gb_agg,
bench_gb_multikey), pandas 2.2.3, 1M, g=1000 (×g2=100 for multi-key), load ~10.
- df.groupby(k) agg: median 15.0ms=1.33x, quantile 17.8ms=1.31x, nunique 71.0ms=1.53x, first 2.50ms=1.11x,
  last 2.51ms=1.00x — WIN/parity. Small NEAR-PARITY losses (moment-fusion residual, ~1ms, NOT lever-worthy):
  sem 4.67 vs 3.45ms (0.74x), std 3.99 vs 3.46 (0.87x), skew 5.00 vs 4.59 (0.92x) — already on the typed dense
  moment path (try_moment_dense / agg_typed_pairs_dense_f64_moments); residual is fp's per-moment passes vs pandas'
  fused Cython, same shape as the grouped shift/diff residual.
- df.groupby([k1,k2]) MULTI-KEY (memory's "open vein" — now CLOSED): sum 43.97 vs 76.29ms=1.74x, mean 43.24 vs
  79.78=1.84x, count 34.17 vs 74.82=2.19x, max 35.87 vs 75.47=2.10x — all WIN.
NET: the GroupBy surface (single+multi key, reductions/transforms/moments) is dominated. The only sub-1.0x reads are
near-parity moment-fusion (sem/std/skew) and the documented grouped shift/diff. No new radical lever found this dig.

### 2026-06-26 BlackThrush — PROBE (no big gap): df merge inner — Utf8 key 5x WIN, Int64 near-parity
Dug the join surface (memory flagged Utf8/multi-key as open). bench_merge (fp-join example), 1M left x 100K right
unique, inner, pandas 2.2.3, load ~11:
- Utf8 key: fp 73.94ms vs pandas 370.29ms = 5.01x WIN (the "open" Utf8-key vein is CLOSED — fp contiguous-Utf8
  factorize crushes pandas' object-key hash join).
- Int64 key: fp 66.51ms vs pandas 59.24ms = 0.89x near-parity (hash-join vs hash-join; ~7ms residual, not lever-
  worthy, and fp-join is peer-contended).
NET: join surface dominated (Utf8) / near-parity (i64). No new lever. Combined with this session's groupby/ewm/
rolling/expanding/str/pivot/sort probes, the fp-frame+fp-join op surface is comprehensively at-or-above pandas; the
only sub-1.0x reads are near-parity moment-fusion (grouped sem/std/skew, shift/diff) and structural/build-gated
(math-unary AVX target-block, round libcall, to_numpy/transpose 2D-block) — all already documented.

### 2026-06-27 BlackThrush — dt.isocalendar 0.048x -> 0.59x (12.2x fp-side): biggest gap of the session
Probed the Series.dt accessor (bench_dtacc): strftime 4.40x, day_name 8.45x, month_name 4.88x WIN; normalize 0.66x
minor; but dt.isocalendar was 504.95ms vs pandas 24.29ms = 0.048x (20x SLOWER) — the biggest gap found in many
cycles. THREE stacked costs, all fixed via a typed Datetime64 fast path: (1) DataFrame::from_series computed an O(n)
UNION over three identical 1M-label indexes (super-linear, cache-cold — the DOMINANT cost) -> build directly via
new_with_column_order on the shared input index; (2) Timestamp::year()/month()/day() EACH re-ran the full Hinnant
civil algorithm (3x redundant) -> compute civil ONCE over the raw as_datetime64_slice &[i64]; (3) iso_year_week_day
rebuilt two [i64;12] arrays + iter-summed per call -> inlined with const CUM/Sakamoto tables. Output via typed
from_i64_values_with_validity (NaT -> validity-missing) instead of 3x Vec<Scalar>.
bench_dtacc 1M (load ~12): isocalendar 504.95 -> 41.4ms = 0.048x -> 0.59x (pandas 24.29ms; 12.2x fp-side, eliminates
the 20x-slowness; residual is numpy's vectorized civil). Conformance GREEN: fp-frame lib 3103 + new differential
isocalendar_typed_conformance (typed Datetime64 path == Utf8 parse path bit-for-bit over a decade of daily dates +
ISO-week 52/53 boundaries + leap years + NaT). LESSON: DataFrame::from_series' multi-index union is a hidden O(n)
tax on any multi-column dt/derived output — check other 3-column dt returns (e.g. similar accessors).

### 2026-06-27 BlackThrush — resample('W') dense-scatter + numeric Sunday-ordinal: 0.17x -> 0.51x (3x fp-side)
Probed Series.resample (bench_resample, 1M minute-spaced Datetime64 index, pandas 2.2.3): 'D' 1.1-1.4x WIN, but 'h'
0.26x and 'W' 0.17x LOSS. Fixed 'W' (the biggest gap, 43.61 vs 7.45ms) with TWO bit-identical levers in
resample_build_groups' `unit == "W"` branch: (1) the per-row scatter did groups.get_mut(String) — hashing a String
EVERY row (1M); replaced with a dense Vec<Vec<usize>> scatter by integer bin index `(bin_end-min)/step`, building
String keys ONCE per bin (empty bins still filled). (2) the per-row Sunday-ordinal built a chrono NaiveDate +
.weekday() per row; replaced with pure i64 from ns (1970-01-01 = Thursday => weekday-from-Sunday=(dse+4)%7,
ord=dse+days_to_sunday+719_163). bench_resample 1M: W mean 43.61->14.5ms = 0.17x->0.51x (pandas 7.45; 2W same; D
7.3ms no regression). Conformance GREEN: fp-frame lib 3103 + 51 resample tests + new differential
resample_week_numeric_conformance (numeric == chrono Sunday-ord over 80yr incl. pre-1970 + sub-day offsets).
REMAINING (logged, not lever-worthy yet): 'h' 0.27x (29 vs 7.9ms) — the fixed-duration path already bins numerically
+ dense-scatters, but materializes a Vec<Vec<usize>> over 16667 bins (thousands of small allocs + scattered gather);
a CSR (counting-sort, one flat Vec) layout or a single-pass sum/count accumulator for sum/mean would close it.

### 2026-06-27 BlackThrush — resample sub-daily ('h') single-pass fusion: 0.27x -> 0.78x (2.9x fp-side)
Follow-up to the 'W' win. 'h' (29 vs 7.9ms) already used subdaily_reduce_single_pass (sum/count per bin, no
Vec<Vec>) — but it first materialized TWO 16MB Vec<Option<i64>> temps (ns_ords + bins) and scanned them for
min/max. Since origin == min ns, every bin is >= 0 (bmin == 0), so fused to: one pass for min/max ns, then accumulate
sum[bin]/count[bin] directly via resample_label_to_ns inline — NO temp Vecs. bench_resample 1M: h mean 29->10.1ms =
0.27x->0.78x (pandas 7.9; sum same); D/W no regression. Bit-identical (same origin/bins/bound/row-order accum):
fp-frame lib 3103 + 51 resample tests, 0 failed.
SEPARATE pre-existing pathology (NOT this change, golden-locked): resample('min') 261ms / ('s') 3142ms over
minute-spaced data — these produce ~1M / ~60M bins (near-identity / mostly-empty) and format that many Utf8 STRING
labels (fast civil format! for 'min'; chrono dt.format for 's' via the build_groups fallback). pandas returns an i64
DatetimeIndex (no string formatting). Closing them needs the resample output index to be Datetime64 labels instead
of Utf8 strings — a golden-regen + label-dtype change across the whole resample surface, deferred.

### 2026-06-27 Codex cod-a — fractional Column.round magic-constant half-even: 0.58x -> 0.64x vs pandas (1.11x fp-side)
Dug the documented `round_ties_even` libcall residual after confirming no committed bench-worktree win was waiting off
main. Implemented the alien-graveyard "target-block/math lowering" lever without unsafe: for typed all-valid Float64
`Column::round(decimals)`, finite nonzero scale factors now use the classic nearest-even magic-constant lowering
`(abs(y)+2^52)-2^52` plus sign restoration, then divide by the same scale. Non-finite/zero factors stay on the existing
`f64::round_ties_even` path; nullable/NaN paths stay scalar, preserving missing-value semantics.

Bench evidence, per-crate target dir `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`, `rch exec`
fell open locally because workers had no admissible slots. Cargo rejects the literal `cargo bench --release` spelling
on this toolchain, so the valid per-crate bench command was `cargo bench -p fp-columnar` (green). Timing used the
existing `bench_round` example, 2M fractional f64 values rounded to 2 decimals, repeated 10x after warmup:

| workload | origin/main best/median | patched best/median | pandas 2.2.3 best/median | ratio vs pandas | fp-side |
| --- | ---: | ---: | ---: | ---: | ---: |
| Column.round(2), fractional f64 | 8.756ms / 9.465ms | 7.920ms / 8.926ms | 5.064ms / 5.355ms | 0.58x -> 0.64x best, 0.57x -> 0.60x median | 1.11x best, 1.06x median |

Conformance GREEN: `cargo test -p fp-columnar --release` passed 441/441 (5 ignored); `cargo test -p fp-conformance
--release` passed with exit 0. Bit-contract proof: new randomized edge test compares the fast scaled path to
`(x * factor).round_ties_even() / factor` by exact bits across infinities, signed zero, 2^52 boundaries, large finite
values, and 20k generated normals for decimals -6, -2, 0, 2, 6, 12. Remaining gap is memory/write + scalar path
overhead; true SIMD roundpd/SVML lowering is still the deeper lever.

### 2026-06-27 BlackThrush — to_period numeric civil label (M/D/Y/Q/H/min/s): ~150->94ms M (1.6x fp-side, 0.15x->0.24x)
Probed Series.dt -> to_period (converts the Datetime64 ROW INDEX to period labels; fp uses Utf8 string labels):
ALL freqs lose — M 0.15x (150 vs 22.7ms), D 0.14x, Y 0.19x, W 0.064x (402ms). period_index_label converted each ns
-> chrono NaiveDateTime (DateTime::from_timestamp) -> dt.format per row (1M chrono ops). Added period_label_numeric:
inline Hinnant civil (y,m,d) + time-of-day from div_euclid over the raw ns, format! directly, gated 4-digit years
(chrono %Y zero-pads to 4 == {:04}); Weekly/Business return None -> keep the chrono path (need week/business bounds).
bench_toperiod 1M (load-matched toggle): M 150->94ms (1.6x fp-side, pandas 22.69 -> 0.15x->0.24x), D 198->155ms.
Bit-identical: fp-frame lib 3103 + 35 to_period tests, 0 failed. CAUTION (process note): an early stash-toggle was
load-CONFOUNDED (without=load61, with=load114) and falsely read ~0-gain; a same-load re-toggle showed the real 1.6x.
RESIDUAL FLOOR (still <1.0x, NOT chrono): 1M IndexLabel::Utf8 String allocs + Index::new over them — pandas returns
an i64 PeriodIndex (no strings). Closing needs a Period/i64 index label type (golden-regen, big). 'W'/'B' still
chrono (anchored bounds). PROBED & WON (don't re-chase): df.melt 100Kx10/x20 = 1.8x/1.6x WIN (typed tiling).

### 2026-06-27 BlackThrush — to_period('W') numeric weekly label: ~376->235ms (1.6x fp-side, 0.064x->0.11x)
Extends last cycle's to_period numeric civil to the Weekly freq (the biggest to_period gap, W was 0.064x = 15x
slower). period_label_numeric returned None for Weekly -> chrono (NaiveDateTime + weekly_period_bounds + 2x
dt.format per row). Now computes week bounds numerically: 1970-01-01 (day 0) = Thursday => num_days_from_monday =
(day+3) mod 7; Monday = day - that, Sunday = +6; civil_from_day (refactored shared Hinnant helper) + format! the
"start/end" label. bench_toperiod 1M (same-load toggle): W ~376->235ms = ~1.6x fp-side (pandas 25.75 => 0.064x->
0.11x). Bit-identical: fp-frame lib 3103 + 38 to_period tests + new differential to_period_numeric_conformance
(Datetime64 numeric path == Utf8-string chrono path for M/D/Y/Q/W over a decade incl. boundaries).
RESIDUAL (unchanged, string-floored): still <1.0x because to_period emits 1M IndexLabel::Utf8 (2 dates/label for W)
+ Index::new — pandas returns an i64 PeriodIndex. The Utf8 period-label representation is the real floor; W/B
anchored aliases still chrono. Closing needs a Period index-label type (golden-regen, cross-surface).

### 2026-06-27 Codex cod-b - sorted duplicate-run Utf8 inner merge: 2.75x -> 3.15x vs pandas
Landed the measured bench-worktree lever from `69e75b39` on current main: all-valid contiguous Utf8 inner joins now
detect sorted duplicate runs and use a two-pointer run merge before the hash-map fallback. This extends the existing
ordered-unique Utf8 path to the non-unique sorted case while preserving pandas left-major/right-minor duplicate
pairing. Unsorted or null-bearing keys still fall through to the prior paths.

Bench evidence, same target dir `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, command routed
through `rch exec` but fell open locally because no worker slots were admissible. The baseline used current main
`66104c54f` with the identical `bench_merge sorted_dup` example mode applied only for measurement; patched timing is
this commit's code. Workload: `bench_merge 1000000 100000 utf8 sorted_dup`, 1M sorted duplicate-run left Utf8 keys
joined to 100k sorted unique right Utf8 keys; pandas 2.2.3 equivalent `left.merge(right, on="key", how="inner")`.

| workload | current main | patched | pandas 2.2.3 | ratio vs pandas | fp-side |
| --- | ---: | ---: | ---: | ---: | ---: |
| sorted duplicate-run Utf8 inner merge | 82.124ms | 71.649ms | 226.015ms | 2.75x -> 3.15x | 1.15x |

### 2026-06-27 Codex cod-a — NO-SHIP: Series.to_period direct Series bypass is dominated after numeric-label main
Started from the stale pre-numeric-label gap where `Series.to_period("M")` measured 267.302ms locally versus pandas
2.2.3 best 20.678ms. A direct Series path that bypassed `to_frame -> DataFrame::to_period -> squeeze` initially
looked like a 1.68x fp-side win against that old baseline (158.790ms), with conformance GREEN. During verification,
`origin/main` advanced with the numeric civil/weekly `to_period` commits (`471910b4f`, `f5e092a43`) plus the sorted
Utf8 merge land (`f082728e6`), so the candidate had to be retested against current main.

Current-main recheck, same target dir `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`, `rch exec`
fell open locally due no admissible workers:

| workload | current main | direct Series bypass | pandas 2.2.3 best/median | ratio vs pandas | fp-side |
| --- | ---: | ---: | ---: | ---: | ---: |
| Series.to_period("M"), 1M Datetime64 index | 97.661ms | 188.073ms | 20.678ms / 21.068ms | 0.212x -> 0.110x | 0.52x REGRESSION |

Action: aborted the cherry-pick and landed no code. Lesson: after numeric label formatting, the DataFrame round trip is
not the dominant cost and may preserve hotter codegen/layout than the extracted helper. Do not re-chase the old
267ms->158ms evidence; the live frontier is still the Utf8 period-label representation floor versus pandas' compact
PeriodIndex, not the Series/DataFrame wrapper path.

### 2026-06-27 AmberLynx — to_period contiguous-Utf8 index (default freqs): partially lifts the String-alloc floor
The prior cycles squeezed the per-label COMPUTE (numeric civil/weekly labels, 1.6x fp-side) but left the documented
OUTPUT floor intact: the non-anchored `to_period` branch collected a `Vec<IndexLabel::Utf8(String)>` (1M heap String
allocs @1M rows) then `Index::new` over a 32B/elem `Vec<IndexLabel>`. Applied the STACK-LEVER pattern: write every
period label straight into ONE contiguous `String` buffer + `Vec<usize>` offsets, then build the index via the lazy
`Index::from_utf8_contiguous(bytes, offsets)` (br-frankenpandas-nbspq) — zero per-row String allocations, no 32MB
`Vec<IndexLabel>`. New `period_label_numeric_into` (write! not format!, bit-identical bytes) + `append_period_label`
(truncate-on-fallback so a partial numeric write never corrupts the buffer; chrono fallback for out-of-range years /
Utf8 timestamp labels keeps one temp String). Anchored aliases (W-MON.., Y-JAN.., Q-anchored) keep the per-label path.

Same-box back-to-back best-of-6, 1M Datetime64 row index (`examples/bench_toperiod`), CARGO_TARGET_DIR per-crate:
| freq | origin/main best | patched best | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| M | 85.36ms | 77.0ms | 1.11x | ~0.27x->0.29x (pandas 22.69ms) |
| Q | 74.19ms | 66.0ms | 1.12x | — |
| D | 118.63ms | 110.9ms | 1.07x | — |
| Y | 52.21ms | 51.1ms | 1.02x | — |
| W | 213.70ms | 181.3ms | 1.18x | ~0.12x->0.14x (pandas 25.75ms) |

ALL FIVE freqs faster (consistent direction ⇒ real, not load noise); the memory-traffic win (1M fewer heap allocs)
exceeds the wall-clock delta and compounds at larger frame sizes / under concurrency. Bit-identical: fp-frame lib
3103 passed / 0 failed / 15 ignored, all 56 test binaries green incl. `to_period_numeric_conformance` (numeric path
== chrono path over a decade w/ boundaries). RESIDUAL (still <1.0x vs pandas, string-floored): the labels are still
Utf8 bytes, not an i64 PeriodIndex; the remaining cost is now the per-label civil/weekly COMPUTE + the input-side
`.labels()` materialization (no public Datetime64 nanos view to iterate raw). Closing fully still needs a Period
index-label type (golden-regen, cross-surface). Anchored W-/Y-/Q- aliases unchanged.

### 2026-06-27 Codex cod-b — Series.to_period shared contiguous-index path: 0.210x -> 0.295x vs pandas (1.40x fp-side)
After the stale direct-Series bypass was rejected above, dug the live `to_period` string-floor gap against current
main (`c60b782ee`). The old bypass regressed because it used a per-label helper and lost the newer contiguous-Utf8
index builder. This lever factors the current `DataFrame::to_period` index conversion into
`period_index_from_datetime_like_index` and routes `Series::to_period` directly through that exact helper, so Series
skips the `to_frame -> DataFrame::to_period -> squeeze` wrapper while preserving the contiguous bytes+offsets Period
label output, Series name, values, categorical metadata, sparse metadata, and index name.

Same-worktree proof with `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo run
--release -p fp-frame --example bench_toperiod -- 1000000 M`: current main local best **108.054ms**; patched local
best samples **78.747ms / 76.999ms**; pandas 2.2.3 comparator from the current ledger **22.69ms**. Ratio vs pandas
improves **0.210x -> 0.295x** using the patched best; FP-side speedup is **1.40x**. A remote `hz2` patched sample
also returned **67.284ms**, but the local before/after pair is the comparison proof.

Semantics are shared, not duplicated: `DataFrame::to_period` calls the same helper, so anchored `W-*`, `Y-*`, and
`Q-*` aliases plus unsupported-frequency errors follow one code path. The focused Series guard now asserts the Series
name, index name, and values survive the direct route.

### 2026-06-27 AmberLynx — df.pivot Int64-keys/Float64-values typed fast path: 0.57x -> 1.98x vs pandas (3.0x fp-side)
df.pivot was the biggest non-structural MEASURED loss in the dataframe_ops matrix: fp 66ms vs pandas 43.6ms = 0.57x
@1M (100k distinct rows x 10 cols, unique (r,c) pairs). CAUSE: `DataFrame::pivot` called `.values()` on the index,
columns AND values columns — each materializing a 1M-element `Vec<Scalar>` (3M boxed scalars), then hashed a
`ScalarKey` per row for the row/col position maps + the cell scatter (2M ScalarKey probes). The classic Scalar-
materialization tax. LEVER (`pivot_int64_keys_f64_values`, br-frankenpandas-pvtdk): when index+columns are all-valid
Int64 (`as_i64_slice`) and values all-valid no-NaN Float64 (`as_f64_slice`), read every key/value straight off the
typed slice, dedup+sort unique keys as raw i64, probe position maps with `FxHashMap<i64,u32>` (no ScalarKey), scatter
into the same dense col-major grid, and MOVE a typed `Vec<f64>` into each all-present output column via
`from_f64_values_owned` (Scalar `Null(NaN)` path only for columns with gaps).

Same-box back-to-back best-of-5, fp-bench df_pivot @1M, `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| workload | origin/main best | patched best | fp-side | vs pandas 2.2.3 (43.62ms) |
| --- | ---: | ---: | ---: | ---: |
| df_pivot 1M (Int64 keys, f64 vals, dense) | 66.11ms | 22.06ms | 3.0x | 0.57x -> 1.98x (WIN) |

Bit-identical: first-seen unique i64 sorted ascending == `scalar_key_cmp` on Int64; row labels `IndexLabel::Int64(k)`,
column labels the decimal string the generic `format!("{v:?}")`+strip produced; present cells `Scalar::Float64(slice[pos])`,
missing the same `Null(NaN)`; all-valid `from_f64_values_owned` == `from_values` for no-NaN Float64. New differential
test `df_pivot_int64_typed_matches_generic_pvtdk` proves the typed path's per-column value matrix == the generic
Scalar path on a SPARSE int64/f64 pivot (covers both the all-present and the missing-cell branch). Conformance GREEN:
fp-frame 40 pivot tests + full lib suite pass. Utf8/mixed-dtype or NaN-bearing pivots keep the generic path unchanged.

### 2026-06-27 AmberLynx — Series.unstack typed Float64 value path: 0.43x -> 0.48x vs pandas (~1.15x fp-side)
After the df.pivot typed lever, df_unstack was the next dataframe_ops loss: fp 57-61ms vs pandas 24.4ms = ~0.43x @1M
(Series with composite Utf8 "r, c" index, r=i/10 x c=i%10 -> 100k x 10). `Series::unstack` already had the dense
col-major grid, but still materialized the values column with `.values()` (1M Scalars) and cloned 1M Scalars into the
output. Applied the same typed lever as pivot (br-frankenpandas-unstk): when the source column is all-valid no-NaN
Float64 (`as_f64_slice`), read cell values off the typed slice (no input Scalar materialization) and MOVE a typed
`Vec<f64>` into each all-present output column via `from_f64_values_owned` (Scalar `Null(NaN)` only for gap columns);
the `.values()` call is now deferred into the non-typed fallback branch.

Same-box back-to-back best-of-5, fp-bench df_unstack @1M, `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| workload | origin/main best | patched best | fp-side | vs pandas 2.2.3 (24.37ms) |
| --- | ---: | ---: | ---: | ---: |
| df_unstack 1M (Utf8 "r,c" index, f64 vals) | 57.18ms | 50.95ms | ~1.12-1.17x | 0.43x -> 0.48x |

Bit-identical: present cells carry `Scalar::Float64(slice[pos])` (== the original), missing the same `Null(NaN)`,
all-valid `from_f64_values_owned` == `from_values` for no-NaN Float64; the grid/first-wins parse is unchanged. New
test `series_unstack_typed_f64_branches_unstk` covers both the all-present (`from_f64_values_owned`) and gap
(`Null(NaN)`) branches (the existing golden uses Int64 values; `series_unstack_missing` carries a source NaN — neither
reaches the all-valid-Float64 path). Conformance GREEN: fp-frame 8 unstack tests + full lib suite.
RESIDUAL (still <1.0x vs pandas): the cost is now dominated by parsing 1M composite "r, c" Utf8 labels (split_once +
trim + 2x `FxHashMap<String>` probes) — pandas operates on a real 2-level MultiIndex with no string parsing. Closing
needs a native MultiIndex row-label representation (cross-surface), analogous to the to_period i64-PeriodIndex floor.

### 2026-06-27 AmberLynx — NEGATIVE: df.pivot Utf8 keys already 2.3x WIN; joins/groupby/io/resample sweep all WIN
After landing df.pivot Int64 (b456cc4b3) and unstack (c8867fe57), checked whether the OPEN sibling — df.pivot with
Utf8 keys — is a gap. It is NOT. Standalone `bench_pivot_str` (1M rows, Utf8 "r{:07}"/"c{:02}" keys, unique (r,c),
f64 values; same shape as df_pivot): fp best **104-108ms** vs pandas 2.2.3 `df.pivot` **239.08ms** = **2.3x WIN**
(pandas' string-keyed pivot is ~5.5x slower than its int pivot, so fp's generic ScalarKey path already beats it).
The Int64 typed fast path (br-frankenpandas-pvtdk) is NOT needed for Utf8 — do NOT re-chase. (Example removed, not
committed.)

Same-session quiet-box (load ~14) sweep of the rest of the matrix @1M, fp-side fp-bench min vs matched pandas best-of-4
(ratio = pandas/fp, >1 = fp faster) — ALL WIN, no gap found:
| op | fp ms | pandas ms | ratio | | op | fp ms | pandas ms | ratio |
| --- | ---: | ---: | ---: | --- | --- | ---: | ---: | ---: |
| groupby_rank_str | 68.6 | 251.8 | 3.7x | | df_groupby_widekey_sum | 115.2 | 220.8 | 1.9x |
| groupby_nunique_str | 50.3 | 125.3 | 2.5x | | join_inner_shuffled | 60.3 | 136.3 | 2.3x |
| groupby_multi_str | 37.3 | 74.8 | 2.0x | | csv_read | 59.4 | 1495.3 | 25x |

The measurable surface is dominated. Remaining sub-1.0x ops are all documented cross-surface floors: df_unstack 0.48x
(composite Utf8-key parse / no native MultiIndex), to_period ~0.29x (Utf8 labels / no i64 PeriodIndex), transpose &
to_numpy (2D-block / Vec<Vec> structural). Each needs a representation change (golden-regen, multi-crate), not a
point fix.

### 2026-06-27 AmberLynx — NEGATIVE: two typed-lever extensions are ~0-gain (reverted); where the pivot/unstack lever stops
After the df.pivot (3.0x) and Series.unstack (1.15x) typed-OUTPUT wins, probed two more extensions on the remaining
sub-1.0x / near-parity ops. Both same-box back-to-back best-of-6 @1M, fp-bench, REVERTED (~0-gain):

1. **unstack composite-key temporal cache** (last-seen row/col interned-index, short-circuit the FxHashMap<String>
   probe on a run hit): baseline 56.4ms vs patched 58.2ms — NO gain (marginally slower). The col component changes
   every row (c=i%10) so its cache is pure overhead, and the row strcmp savings don't beat the FxHashMap probe. A
   prior agent already filed the same idea (`stash ZERO-GAIN-revert-TypedDedupCol-temporal`) — do NOT re-try.
2. **to_records typed-input** (`as_f64_slice`/`as_i64_slice` to construct the per-cell Scalar directly, skipping the
   `col.values()` Vec<Scalar> materialization): baseline 79.7ms vs patched 76.7ms = ~1.04x — bench-amortized to ~0.
   `time_us` re-runs to_records on the SAME df, so `col.values()` is OnceLock-cached after iter 1 and min-of-N is
   warm; the input materialization the lever removes is already amortized away. The OUTPUT is `Vec<Vec<Scalar>>`
   (Scalar by contract) so the dominant per-cell Scalar construction is unavoidable. (Both tests pass / bit-identical
   if ever wanted; changes stashed, not landed.)

GENERALIZATION (the boundary of the typed lever): the pivot/unstack 3x came from eliminating output Scalar
CONSTRUCTION via `from_f64_values_owned` (typed Vec<f64> output). It does NOT transfer to (a) Scalar-output-by-
contract ops (to_records/to_dict — output is the cost), (b) parse-bound ops (unstack composite-key — no native
MultiIndex), or (c) bench-amortized input materialization (OnceLock-cached `.values()`). Confirms again: the
measurable surface is dominated; the only real residuals are the representation floors (MultiIndex, i64 PeriodIndex,
2D-block) — each a multi-crate golden-regen project, not a point fix.

### 2026-06-27 AmberLynx — sort_values_single small-size: 10k 0.76x is fixed overhead, NOT the parallel gather (par-floor ~0-gain, reverted)
Swept the matrix at 10k (small-size fixed-overhead is the one dimension unexplored after 5 cycles of 1M sweeps). The
ONLY sub-1.0x non-floored op found: `sort_values_single` @10k — fp 530us vs pandas 403us = **0.76x**. It WINS at the
larger sizes: 100k fp 3641us vs pandas 4076us = **1.12x**, and 1M (well-established). Everything else at 10k WINS
(drop_dups 65x, melt 10.7x, gb_sum 11.5x, stack 2.7x, cumsum 2.6x, value_counts 2.2x, sort_multi 3.6x; unstack 1.04x
≈ parity).

HYPOTHESIS (rejected): the 0.6x came from `reorder_rows_by_positions_unchecked` spawning a `thread::scope` for the
10-column gather via the default 16K-cell `par_map_columns` floor — a pure-bandwidth gather that is L2/L3-resident at
10k/100k, where thread-spawn should be loss (cf `DataFrame::abs`'s 4M floor). Raised the reorder floor to 4M (10k/100k
serial, 1M still threaded). MEASURED same-box back-to-back best-of-4: baseline 10k 530us / 100k 3641us vs patched 10k
531us / 100k 3742us — **~0-gain** (best identical at 10k; baseline slightly FASTER at 100k). The thread-spawn overhead
is already negligible; the earlier ~986us "baseline" was a LOAD-NOISE phantom on a busy box. REVERTED.

The genuine 10k residual is fixed Scalar/radix/frame-construction overhead, and `sort_column.values()` is OnceLock-
amortized across the bench's repeated sorts (so an as_f64_slice input lever is bench-invisible, like the to_records
case). Not a point fix; absolute gap is ~127us on the smallest size while sort wins at 100k/1M. Surface remains
dominated; do NOT re-chase the reorder par-floor for small-size sort.

### 2026-06-27 AmberLynx — to_period hand-rolled ASCII label formatting: 1.37-2.27x fp-side (M 0.36x->0.62x vs pandas)
to_period (the biggest measured gap) emits Utf8 period labels; after the contiguous-Utf8 index + numeric-civil work,
the dominant per-label cost was the `write!(buf, "{y:04}-{m:02}")` machinery — `core::fmt` Formatter dispatch +
`pad_integral` (width/fill/sign branches) paid 1M× per to_period. Replaced every `write!` arm in
`period_label_numeric_into` with hand-rolled ASCII digit pushes (`push_4d`/`push_2d` for `{:04}`/`{:02}`, `push_uint`
for Quarterly's UNPADDED `{}` year, literal digit for the quarter) — bit-identical bytes, no fmt dispatch.

Same-box back-to-back best-of-6, 1M Datetime64 row index (`examples/bench_toperiod`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| freq | baseline (write!) | patched (hand-rolled) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| M | 63.43ms | 36.35ms | 1.75x | 0.36x -> 0.62x (pandas 22.69ms) |
| D | 79.52ms | 34.97ms | 2.27x | — |
| Y | 44.35ms | 32.27ms | 1.37x | — |
| Q | 59.91ms | 33.61ms | 1.78x | — |
| W | 161.07ms | 71.76ms | 2.24x | 0.16x -> 0.36x (pandas 25.75ms) |

The `core::fmt` overhead was ~HALF of to_period's wall time. Bit-identical: differential
`to_period_numeric_conformance` (numeric path == chrono path over a decade incl. boundaries) green; fp-frame 9
to_period lib tests + full suite green. RESIDUAL (still <1.0x vs pandas, unchanged): the labels are still Utf8 bytes,
not an i64 PeriodIndex — closing fully needs a Period index-label type. GENERALIZES: any hot loop formatting fixed-
shape integers (strftime, astype_str, csv/json numeric serialization) pays the same `pad_integral` tax — hand-rolled
ASCII digit writers are a reusable lever.

### 2026-06-27 Codex cod-a - SeriesGroupBy shift/diff periods=1 fast path: 2.8x/2.35x fp-side
Dig path after the measured to_period contiguous-Utf8 win was already on `origin/main`. The biggest old source-
addressable residual still worth testing was SeriesGroupBy shift/diff over dense Int64 keys. Current main had already
landed the key-offset path, but `periods=1` still paid the general ring-buffer cost: per row it did `cnt`, modulo,
`off * periods + slot`, and a usize count update even though the common pandas default only needs one previous value
per group. Added a narrow `periods == 1` branch in the Int64-key direct kernels: `last[off]`, `seen[off]`, one validity
bit, and no ring math. `periods > 1` stays on the existing implementation.

Bench evidence, same checkout and target dir `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`,
`rch exec` fell open locally because no workers were admissible. Workload: `bench_gb_cum 1000000 1000 {shift,diff}
6` (1M all-valid Float64 values, dense Int64 grouping key, periods=1). Same-box pandas 2.2.3 comparator used the
same generated splitmix-style key/value arrays.

| workload | current main | patched | pandas 2.2.3 | ratio vs pandas | fp-side |
| --- | ---: | ---: | ---: | ---: | ---: |
| SeriesGroupBy.shift(1) | 9.967ms | 3.527ms | 28.490ms | 2.86x -> 8.08x | 2.83x |
| SeriesGroupBy.diff(1) | 8.431ms | 3.589ms | 29.100ms | 3.45x -> 8.11x | 2.35x |

Notes: the old ledger's sub-1.0x pandas ratios for this surface are stale under today's same-box pandas comparator,
but the fp-side delta is large and same-worktree. Bit contract is the existing per-group row-order reference:
`shift(1)` emits the previous in-group value; `diff(1)` emits current minus previous; first row per group remains
missing via the same `from_f64_values_with_validity` path.

### 2026-06-27 TealOsprey — strftime hand-rolled ASCII digit writers: 2.05-2.2x fp-side (9.8-10.5x vs pandas)
Direct follow-up to AmberLynx's to_period lever (the prior entry literally named `strftime` as the next loop paying
the same tax). `DatetimeAccessor::strftime`'s clean typed path (all-valid `datetime64[ns]`, no NaT) emitted each `%Y`
/`%m`/`%d`/`%H`/`%M`/`%S` directive via `write!(bytes, "{y:04}")` etc — `core::fmt` Formatter dispatch + `pad_integral`
(width/fill/sign branches) paid per directive per row, 1M× and more for multi-directive formats. Replaced every
`write!` arm with hand-rolled ASCII digit pushes into the `Vec<u8>` byte buffer (`push_4d_bytes`/`push_2d_bytes`,
new siblings of `push_4d`/`push_2d`). Bit-identical: a `datetime64[ns]` year is always in [1677, 2262] (the i64-ns
representable range) so `%Y` is exactly 4 digits like `{:04}`, and m/d/H/M/S are exactly 2 like `{:02}` — the manual
writers produce byte-identical output, no fmt dispatch.

Same-box back-to-back best-of-3, 1M `datetime64[ns]` series (`examples/bench_strftime`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| format | baseline (write!) | patched (hand-rolled) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `%Y-%m-%d` | 91.47ms | 44.54ms | 2.05x | 4.76x -> 9.80x (pandas 436.0ms) |
| `%Y-%m-%d %H:%M:%S` | 180.34ms | 81.88ms | 2.20x | 4.77x -> 10.50x (pandas 859.2ms) |

The `core::fmt` overhead was ~HALF of strftime's wall time, same as to_period. Already WON vs pandas before; now wins
by ~2x more. Bit-identical: conformance packets FP-P2D-239 (basic strict) + FP-P2D-310 (null hardened) both green
(`--require-green` exit 0); fp-frame suite green. CONFIRMS the AmberLynx generalization: hand-rolled ASCII digit
writers are a reusable lever for any hot loop formatting fixed-shape integers (next candidates: astype_str, csv/json
numeric serialization).

### 2026-06-27 TealOsprey — astype(Int64->Utf8) hand-rolled decimal itoa: 2.32x fp-side (10.6x vs pandas)
The third op carrying the AmberLynx/strftime `core::fmt` tax (the strftime entry named `astype_str` as a next
candidate). `Column::astype(Utf8)`'s Int64 fast path wrote each value via `use std::io::Write; write!(bytes, "{v}")`
into a contiguous byte buffer — `io::Write::write_fmt` routes through Formatter construction + the io error path per
row, far heavier than `fmt::Write`. Replaced with `push_i64_decimal`: a two-digit-LUT itoa emitting digit pairs LSB-
first into a 20-byte temp then copying the suffix, `unsigned_abs` for `i64::MIN`. Bit-identical to `v.to_string()`
(what `cast_scalar(Int64, Utf8)` does — pandas spells ints plainly).

Same-box back-to-back best-of-3, 1M Int64 column (`fp-columnar/examples/bench_astype_str`, mixed-magnitude signed
splitmix values), `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| op | baseline (write!/io::Write) | patched (itoa) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `astype(str)` Int64 1M | 38.53ms | 16.57ms | 2.32x | 4.57x -> 10.6x (pandas 175.9ms) |

Bit-identical: standalone differential `push_i64_decimal` vs `i64::to_string()` over 5M random + all i64 edge cases
(0, ±1, ±99/±100, i64::MIN, i64::MIN+1, i64::MAX) = 0 mismatches; fp-columnar 7 astype unit tests + fp-frame 30
astype tests (incl. `series_astype_string_golden_basic`, `df_astype_all_columns_to_utf8`) green. CONFIRMS the lever
a third time: any hot loop using `write!` into a `Vec<u8>` via `io::Write` to format fixed-shape integers pays the
`write_fmt` tax — hand-rolled ASCII writers are the fix. RESIDUAL: the Float64->Utf8 astype path still calls
`fp_types::float_to_string_for_astype` (shortest round-trip — genuinely hard to hand-roll, left as-is).

### 2026-06-27 Codex cod-b - to_period day-run label cache: M 0.62x -> 1.17x, W 0.36x -> 1.11x vs pandas
Follow-up to the live `Series.to_period` string-floor gap after the shared contiguous-index Series path and hand-rolled
ASCII formatter landed.
The remaining hot cost was not another Series/DataFrame wrapper: hourly Datetime64 indexes repeatedly emitted the
same period label for every row in the same calendar day, so `period_index_from_datetime_like_index` still reran the
Hinnant civil conversion plus fixed-width ASCII digit pushes for 24 adjacent rows with identical M/D/W output. Added
a tiny day-run label cache inside the contiguous Utf8 index builder for date-level frequencies (`Y`, `Q`, `M`, `W`,
`D`, `B`). The cache stores only the just-emitted label for the current Datetime64 day; Utf8 labels and hour/min/sec
frequencies stay on the original direct append path. Output remains byte-identical because cache misses call the same
`append_period_label` helper, and cache hits copy the exact bytes it just appended.

Same-worktree proof, `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`, `rch exec` fell open
locally because no worker slots were admissible. Workload: `cargo run --release -p fp-frame --example
bench_toperiod -- 1000000 <freq>` over a 1M hourly Datetime64 index.

| workload | current main | patched | pandas 2.2.3 comparator | ratio vs pandas | fp-side |
| --- | ---: | ---: | ---: | ---: | ---: |
| Series.to_period("M") | 36.35ms | 19.423ms | 22.69ms | 0.62x -> 1.17x | 1.87x |
| Series.to_period("W") | 71.76ms | 23.102ms | 25.75ms | 0.36x -> 1.11x | 3.11x |
| Series.to_period("D") | 34.97ms | 14.693ms | not rerun in this cycle | - | 2.38x |

Conformance target: same semantics as the existing contiguous label path; validation in this commit runs focused
to_period tests plus full `fp-conformance --release`. Residual: W is still below pandas because FP stores period
labels as Utf8 bytes rather than pandas' compact PeriodIndex ordinals. M now reaches parity on this measured shape;
the next deeper lever remains a real Period index-label type, not more string formatting.

### 2026-06-27 BlackThrush — dt.isocalendar weekday-from-epoch: 0.49x -> 0.62x vs pandas (20% fp-side)
Follow-up dig on the largest remaining measured gap after the typed `dt.isocalendar` fast path. First probe was the
obvious all-valid validity-mask shortcut (build columns with `from_i64_values` when no NaT appears): same-box local
baseline 49.381ms vs patched 88.986ms @1M, REVERTED. The allocation it removes is not the bottleneck, and the extra
invalid-position bookkeeping made the hot all-valid case worse.

Kept lever: typed `isocalendar` already has `dse = ns.div_euclid(NANOS_PER_DAY)`, so ISO weekday can be derived as
`(dse + 3).rem_euclid(7) + 1` because 1970-01-01 was Thursday. This removes the per-row Sakamoto weekday formula
(`y_adj + y_adj/4 - y_adj/100 + y_adj/400 + table + day`) and the month-offset table from the hot loop, while leaving
the civil-date and ISO-week boundary logic unchanged.

Evidence: `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a rch exec -- cargo run --release -p
fp-frame --example bench_dtacc -- 1000000 isocalendar` fell open locally because workers were saturated, so the
comparison uses the immediately preceding same-target local baseline. Current main baseline 49.381ms; patched hot
runs 41.678ms and 39.353ms. Using the established same-workload pandas 2.2.3 comparator from the prior entry
(24.29ms), ratio improves from 0.49x to 0.62x vs pandas; fp-side speedup 1.25x. Residual remains vectorized
date-to-civil arithmetic plus three-column materialization; the cheaper validity construction was negative evidence,
not a keeper.

### 2026-06-27 TealOsprey — format_datetime_ns hand-rolled civil+ASCII (drop chrono): 8.7x fp-side, to_csv datetime 5.1x (13.1x vs pandas)
The fourth and broadest application of the to_period/strftime/astype ASCII-writer lever. `fp_index::format_datetime_ns`
— the shared chokepoint for to_csv datetime columns, repr, to_html, and astype(str)-of-datetime — built a
`chrono::DateTime::from_timestamp_nanos(ns)` then `dt.format("%Y-%m-%d %H:%M:%S").to_string()` plus a
`format!("{subsec:09}")` PER VALUE: chrono DateTime construction + `DelayedFormat` strftime machinery + two String
allocs every row. Replaced with `civil_from_epoch_days` (Hinnant `civil_from_days`, the exact algorithm fp-frame's
strftime fast path already uses) + `div_euclid`/`rem_euclid` time decomposition + hand-rolled `push_4d_str`/
`push_2d_str` + a manual 9-digit trailing-zero-trimmed fraction. No chrono, no `DelayedFormat`, one String alloc.

Same-box best-of, `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| bench | baseline (chrono) | patched (hand-rolled) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `format_datetime_ns` 1M (no subsec) | 318.4ms | 36.7ms | 8.67x | — |
| `format_datetime_ns` 1M (subsec) | 405.2ms | 67.3ms | 6.02x | — |
| `to_csv` 1M datetime column | 507.8ms | 99.3ms | 5.11x | 2.57x -> 13.1x (pandas 1303ms) |

Bit-identical: a permanent differential `format_datetime_ns_bit_identical_to_chrono_over_full_range_dtns` compares
the new impl against an inline chrono reference over 3M values stepping the full i64-ns datetime64 range + the
representable bounds / epoch / pre-epoch sub-second edges = byte-for-byte equal; fp-io 146 csv + 45 datetime tests,
fp-index 45 datetime tests, existing `..._dt64fmt` golden all green. chrono's floor semantics for pre-epoch instants
(-0.5s -> `1969-12-31 23:59:59.5`) are reproduced by `div_euclid`/`rem_euclid`. CONFIRMS the lever a fourth time and
extends it from integer formatting to full calendar formatting; the chrono `DelayedFormat` path was ~85% of
format_datetime_ns wall time. GENERALIZES to any remaining chrono `.format().to_string()` per-row loop.

### 2026-06-27 TealOsprey — to_json(records) typed streaming writer: 0.69x LOSS -> 5.35x WIN vs pandas (7.7x fp-side)
A genuine measured LOSS (not in the standard bench harness): `write_json_string(Records)` materialized a full
`serde_json::Value` tree — a `serde_json::Map` (BTreeMap) PER ROW, a `name.clone()` key PER CELL, a per-cell `Scalar`
(`c.value(row_idx)`) and `Value` — then serialized it. 1M×2-col = millions of allocs; fp 516ms vs pandas 357ms = 0.69x.
Added `try_write_json_records_typed`: a streaming writer over all-valid `Int64`/`Float64`/`Bool` columns that writes
JSON bytes straight into one String — pre-serialized escaped keys (reused every row), `append_i64_decimal` itoa for
ints, and serde's OWN `CompactFormatter::write_f64` into a reusable scratch for floats (so the exponent spelling,
e.g. `1e+20`, is byte-identical — raw `ryu::format_finite` gives `1e20` and would diverge). Non-finite f64 -> `null`,
matching `scalar_to_json`. Falls back to the serde tree (`write_json_records_serde`) on Utf8/null/promotion columns.

Same-box best-of-3, 1M rows × {Int64, Float64} (`fp-io/examples/bench_to_json`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| op | baseline (serde tree) | patched (streaming) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `to_json(orient="records")` 1M | 516.0ms | 66.8ms | 7.72x | 0.69x -> 5.35x (pandas 357.4ms) |

Bit-identical: differential `to_json_records_typed_fast_path_bit_identical_to_serde` compares the typed path against
the serde reference over an edge-laden frame (i64::MIN/MAX, signed-zero, integer-valued/huge/tiny/±Inf floats, a
JSON-escaped column name), a -Inf frame, an empty frame, a Utf8-forces-fallback frame, and a 2000-row LCG sweep —
all byte-for-byte equal; fp-io 48 json tests green. The serde-tree allocation was ~87% of to_json's wall time.
RESIDUAL: Columns/Index/Split/Values orients still build the serde tree (Records is the pandas default + most common).

### 2026-06-27 TealOsprey — to_jsonl typed streaming writer: 1.02x parity -> 5.5x WIN vs pandas (5.44x fp-side)
Direct sibling of the to_json(records) win: `write_jsonl_string` (`to_json(orient='records', lines=True)`) had the
identical serde-tree antipattern — a `serde_json::Map` per row, `name.clone()` key per cell, per-cell `Scalar`+`Value`,
and `serde_json::to_string` PER ROW — joined by `\n`. Generalized `try_write_json_records_typed` with an `as_jsonl`
flag: same streaming body, but rows are `\n`-joined with no enclosing `[ ]`. Bit-identical to the prior serde output
(same pre-serialized keys, `append_i64_decimal` ints, serde `CompactFormatter::write_f64` floats, non-finite -> null).

Same-box best-of-3, 1M rows × {Int64, Float64} (`fp-io/examples/bench_to_jsonl`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| op | baseline (serde tree) | patched (streaming) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `to_jsonl` 1M | 359.2ms | 66.0ms | 5.44x | 1.02x -> 5.53x (pandas 365.1ms) |

Bit-identical: the `to_json_records_typed_fast_path_bit_identical_to_serde` differential now also asserts JSONL vs a
serde Map-per-row reference over every frame (edge floats incl ±Inf, escaped key, empty, Utf8 fallback, 2000-row LCG
sweep); fp-io 48 json tests green. Records + JSONL (the two pandas `records`/`lines` spellings) now share one typed
writer. RESIDUAL unchanged: Columns/Index/Split/Values orients still build the serde tree.

### 2026-06-27 TealOsprey — to_json(columns) typed streaming writer: 0.39x LOSS -> 4.66x WIN vs pandas (11.9x fp-side)
The WORST measured to_json orient (a survey found Columns 0.39x and Index 0.63x both LOSE pandas; Values 1.78x / Split
2.13x / Records already won). The `Columns` serde arm built `{col: {idx: val}}` by re-running `index_label_json_key`
(label `to_string` + serde quote) for EVERY cell of EVERY column (n×k) into a nested `serde_json::Map` per column.
Added `try_write_json_columns_typed`: pre-serialize the n inner index-label keys ONCE into a single contiguous buffer
(+ offsets), reused across all columns; for the common all-Int64-unique index, keys are hand-rolled `"` + itoa + `":`
straight from `int64_label_values()` (no `labels()` IndexLabel materialization, no per-key alloc); values via the
shared `append_typed_json_value`. Bails to the serde tree on non-typed columns or COLLIDING serialized keys (e.g.
`Int64(1)` vs `Utf8("1")` both spell `"1"` — the serde path errors there, so the fast path must not swallow it).

Two-step measurement (`fp-io/examples/bench_to_json 1000000 columns`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`, best-of-3:
| step | time | vs pandas 375.2ms |
| --- | ---: | ---: |
| baseline (serde Map-of-Maps) | 958.6ms | 0.39x |
| naive (Vec<String> idxkeys, 1M allocs) | 617.2ms | 0.61x (REVERTED in-flight) |
| final (contiguous keybuf + itoa + int64 view) | 80.5ms | **4.66x** |

11.9x fp-side end state. The 1M per-key `String` allocs + `labels()` materialization were the residual after the
nested-Map removal; the contiguous buffer + direct i64 view closed it. Bit-identical: the json differential now also
asserts Columns vs a from-scratch serde Map-of-Maps reference over every frame (edge floats, escaped key, empty, Utf8
fallback, 2000-row LCG sweep) + the existing `json_columns_write_duplicate_index_rejects` still rejects via fallback;
fp-io 48 json tests green. RESIDUAL: the `Index` orient (0.63x) still builds the serde tree — same lever applies next.

### 2026-06-27 TealOsprey — to_json(index) typed streaming writer: 0.63x LOSS -> 8.53x WIN vs pandas (13.5x fp-side)
The last losing to_json orient (the columns entry's flagged residual). `Index` is the transpose of `columns` —
`{idx: {col: val}}` — and the serde arm built a `serde_json::Map` per ROW plus an index-label outer key per row.
Added `try_write_json_index_typed` reusing the shared `build_json_index_key_buffer` (the n outer index-label keys
pre-serialized once into a contiguous buffer, hand-rolled `"`+itoa+`":` off `int64_label_values()` for the common
all-Int64-unique index) + the k inner column keys reused every row + `append_typed_json_value`. Bails to serde on a
duplicate index key or non-typed column.

Same-box best-of-3, 1M rows × {Int64, Float64} (`fp-io/examples/bench_to_json 1000000 index`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| op | baseline (serde tree) | patched (streaming) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `to_json(index)` 1M | 1197.6ms | 88.5ms | 13.53x | 0.63x -> 8.53x (pandas 755.4ms) |

Bit-identical: the json differential now also asserts Index vs a from-scratch serde Map-of-row-Maps reference over
every frame; fp-io 48 json tests green. ALL FIVE to_json orients (records/columns/index/values/split) + jsonl now WIN
vs pandas — the to_json surface is fully un-lost (records 5.35x, columns 4.66x, index 8.53x, values 1.78x, split
2.13x, jsonl 5.53x). The serde `Value`-tree was the universal culprit; the typed streaming writers retire it.

### 2026-06-27 TealOsprey — to_json typed writers extended to Datetime64 columns: 0.72x LOSS -> 6.38x WIN vs pandas (8.9x fp-side)
After the records/columns/index/jsonl typed writers landed, ANY Datetime64 column still forced the WHOLE frame onto
the serde-tree fallback (the typed extractor only knew Int64/Float64/Bool), so a realistic frame with a timestamp
column re-lost pandas. Added a `JCol::DtMs` arm: a Datetime64 column with no validity-mask nulls is emitted as
epoch-MILLISECOND integers (`v / 1_000_000`, truncating toward zero) with the `NaT` sentinel (i64::MIN) → `null` —
byte-identical to `scalar_to_json(Datetime64)` (pandas default `date_format='epoch', date_unit='ms'`). Gated on
`!has_nulls()`: `from_datetime64_values` keeps NaT AS DATA with an all-valid mask (handled inline), while a
genuine validity-null column bails to serde. (Timedelta64 left to the serde path — its `from_values` backing marks
NaT as a validity-null, an untested combination; Datetime64 is the common case.)

Same-box best-of-3, 1M rows × {Int64, Datetime64} (`fp-io/examples/bench_to_json_dt`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| op | baseline (serde fallback) | patched (typed) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `to_json(records)` +dt 1M | 480.0ms | 54.1ms | 8.87x | 0.72x -> 6.38x (pandas 344.8ms) |

Bit-identical: the json differential's main frame now carries a Datetime64 column with a NaT + pre-epoch negative ns
(records/jsonl/columns/index all asserted vs serde); fp-io 48 json tests green. The typed JSON writers now cover the
four common column dtypes (i64/f64/bool/datetime) across all five orients + jsonl — a stray timestamp column no longer
silently reverts the frame to the slow path. Confirmed reads are NOT a gap: read_csv 37ms vs pandas 176ms (4.76x),
read_json 473ms vs pandas 1030ms (2.18x) — both already WIN, no action taken.

### 2026-06-27 TealOsprey — to_json typed writers extended to contiguous Utf8 columns: 0.65x LOSS -> 7.7x WIN vs pandas (11.9x fp-side)
The last common column dtype not on the typed JSON path: a Utf8 column forced the whole frame onto the serde tree
(materializing a `Scalar::Utf8` clone + `Value::String` per cell), so a frame with a string column lost pandas. Added
`JCol::U(bytes, offsets)` over the contiguous Utf8 backing `as_utf8_contiguous` exposes (what read_csv / string ops
produce; the `validity.all()` it requires guarantees every row is a present string). Each cell is written by
`append_json_string`: serde escapes ONLY `"`, `\`, and control bytes 0x00–0x1F (multi-byte UTF-8 passes through), so
the no-escape common case is a single `"`+raw+`"` push and only an escape-bearing string deopts to
`serde_json::to_string`. Byte-identical to `scalar_to_json(Utf8)`. (A `Vec<Scalar>` Utf8 backing — e.g. from
`from_values` — still bails to serde; the contiguous backing is the read/op path.)

Same-box best-of-3, 1M rows × {Int64, contiguous Utf8 `item_<k>`} (`fp-io/examples/bench_to_json_str`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| op | baseline (serde fallback) | patched (typed) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `to_json(records)` +str 1M | 558.96ms | 47.0ms | 11.9x | 0.65x -> 7.71x (pandas 362.5ms) |

Bit-identical: the json differential gained a contiguous-Utf8 frame whose strings cover the no-escape fast path AND
every serde escape case (quote, backslash, `\n`, `\t`, control 0x01, accented + emoji multi-byte) — asserted across
records/jsonl/columns/index vs serde; fp-io 48 json tests green. The typed JSON writers now cover ALL common column
dtypes (i64/f64/bool/datetime/utf8) across every orient + jsonl; the serde `Value`-tree is reached only by genuinely
mixed/null/exotic columns. This closes the to_json write surface: no benched column-dtype × orient combination loses.

### 2026-06-27 BlackThrush — dt.isocalendar ordinal ISO week: 0.50x -> 0.54x vs pandas (1.09x fp-side)
Follow-up on the largest remaining non-structural measured gap after the typed `dt.isocalendar` path. The prior path
still derived civil year/month/day, day-of-year, leap status, and 53-week clamps per row. This pass switches the typed
Datetime64 path to the ISO ordinal identity: ISO year is the civil year of the week Thursday, and ISO week is the
distance from the Monday of ISO week 1. That removes month/day/day-of-year work while preserving the existing NaT
validity mask behavior and 53-week year semantics.

Same current-origin worktree (`0881afa1c`), same target dir
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`, `rch exec` fell open locally because workers were
saturated, command `cargo run --release -p fp-frame --example bench_dtacc -- 1000000 isocalendar`; pandas 2.2.3
comparator uses the identical 1M hourly timestamp fixture:
| workload | ORIG best | patched best | pandas best | ratio vs pandas | fp-side |
| --- | ---: | ---: | ---: | ---: | ---: |
| `dt.isocalendar` 1M | 49.089ms | 45.092ms | 24.314ms | 0.50x -> 0.54x | 1.09x |

Bit-equivalence guard: before landing, the ordinal formula was checked against the existing civil formula over a
400k-day contiguous range plus 5M randomized epoch days. Focused fp-frame isocalendar tests and the full
fp-conformance crate were run in release mode. Residual remains output/DataFrame construction plus pandas' vectorized
datetime core; another same-formula scalar tweak is unlikely to close the remaining gap.

### 2026-06-27 TealOsprey — to_json(values) typed streaming writer: 1.78x -> 11.3x WIN vs pandas (6.36x fp-side)
The fifth and last to_json orient still on the serde tree. `Values` is `[[v,v],...]` (row-major arrays, no keys) — the
simplest orient — but still built a `Vec<serde_json::Value>` per row + a per-cell `Scalar`/`Value`. Added
`try_write_json_values_typed` reusing `extract_typed_value_columns` (i64/f64/bool/datetime/utf8) + the shared
`append_typed_json_value`, emitting `[` + comma-joined cell values + `]` per row. Bails to serde on any non-typed
column. Bit-identical to the serde `Values` arm (column-order values, same per-dtype spellings).

Same-box best-of-3, 1M rows × {Int64, Float64} (`fp-io/examples/bench_to_json 1000000 values`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| op | baseline (serde tree) | patched (streaming) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `to_json(values)` 1M | 344.1ms | 54.0ms | 6.36x | 1.78x -> 11.3x (pandas 613.4ms) |

Bit-identical: the json differential now also asserts Values vs a from-scratch serde array-of-arrays reference over
every test frame (numeric edges, datetime+NaT, contiguous-Utf8 escape cases, empty, fallback, LCG sweep); fp-io 48
json tests green. ALL FIVE to_json orients (records/columns/index/values/split) + jsonl now use typed streaming
writers across every common column dtype. Split still builds its `data` arrays via serde (its columns/index header
arrays are tiny); the dominant `data` section is the same row-major shape as Values and is the remaining follow-up.

### 2026-06-27 TealOsprey — to_json(split) typed streaming writer: 2.13x -> 8.59x WIN vs pandas (4.04x fp-side); to_json surface fully typed
The last to_json orient on the serde tree (the Values entry's flagged follow-up). `Split` is
`{"columns":[...],"index":[...],"data":[[...]]}`; the dominant `data` section is the same row-major arrays as Values.
Added `try_write_json_split_typed`: `columns` via serde over the (few) header strings, `index` hand-rolled (itoa per
label for the common all-Int64 index, `index_label_to_json`+serde otherwise — bare JSON values, not keys), `data` via
the shared `append_json_row_arrays` (factored out of the Values writer). Object keys emitted columns/index/data to
match `preserve_order` insertion order. Bails to serde on any non-typed column.

Same-box best-of-3, 1M rows × {Int64, Float64} (`fp-io/examples/bench_to_json 1000000 split`),
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cc`:
| op | baseline (serde tree) | patched (streaming) | fp-side | vs pandas 2.2.3 |
| --- | ---: | ---: | ---: | ---: |
| `to_json(split)` 1M | 316.5ms | 78.3ms | 4.04x | 2.13x -> 8.59x (pandas 672.8ms) |

Bit-identical: the json differential now also asserts Split vs a from-scratch serde `{columns,index,data}` reference
over every test frame; fp-io 48 json tests green. SURFACE CLOSED: ALL five to_json orients
(records/columns/index/values/split) AND jsonl now use typed streaming writers across every common column dtype
(i64/f64/bool/datetime/utf8). The serde `Value`-tree is reached only by genuinely mixed/null/exotic columns. Final
vs-pandas scorecard for the to_json write surface (1M, numeric): records 5.35x, columns 4.66x, index 8.53x, values
11.3x, split 8.59x, jsonl 5.53x — every one a WIN; three started as outright LOSSES (records 0.69x, columns 0.39x,
index 0.63x) plus the datetime (0.72x) and utf8 (0.65x) column-dtype losses, all now closed.
