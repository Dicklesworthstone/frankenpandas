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
| max/min portable-SIMD `i64x8` (uza04.207) | 2M int64 | 185 / 182 µs | 0.811 / 0.811 ms | **0.23× / 0.22×** | ❌ REVERT — safe `std::simd` was 2.3× slower than the manual 8-lane accumulator |
| max/min portable-SIMD `i64x4` (uza04.207) | 2M int64 | 185 / 182 µs | 1.087 / 1.108 ms | **0.17× / 0.16×** | ❌ REVERT — AVX2-width `std::simd` variant was 3.1× slower than the manual accumulator |

| reset_index typed Int64 idx→col (bp6k7) | 1M int64-indexed, 2 cols | 1.93 ms | 0.38 ms | **5.1× faster** | ✅ KEEP — Index::from_i64_values |
| concat typed buffer (tbrtu) | 8×125k Int64 series, ignore_index | 0.28 ms | 6.81 ms | **0.041× (24× SLOWER)** | ⚠️ KEEP (bit-transparent, ≥ old Scalar path); 24× is glibc-malloc-bound — see mimalloc row |
| concat + mimalloc boundary allocator (3nah5) | 8×125k Int64, ignore_index | 0.223 ms | 0.479 ms | **0.46× (2.15× slower)** | ✅ KEEP — adopted in `fp-bench` + `fp-python`; 12.4× faster than current glibc-malloc concat (5.93 ms) but still a pandas loss |

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
| Series.map Float64 dense integer-key table (hbq6y, bench_map_cc materialize) | Same workload, `materialize` mode calls `out.values()` in the timed window | 12.063 ms | 27.089 ms | **0.45× (2.25× slower)** | 🔴 RESIDUAL LOSS — forced public `values()` materializes a `Vec<Scalar>` over the lazy repeated-slice Float64 column; this is not the default Series construction surface, but it is an honest consumption-path loss. Routed to the next bead: make repeated Float64 slices expose/consume numeric data without per-row `Scalar` enum materialization (cyclic/repeated-slice f64 view or typed materialization cache). |
| Series.combine_first (cmbf/grtx1, bench_combine_cc) | 2M f64 same index, self ~50% NaN, other fills | 7.35 ms local after-row (3.08 ms prior ledger host) | 17.94 ms local; rch hz2 19.38 → 18.33/18.40 ms | **0.41× local (2.44× slower)** | ✅ KEEP modest no-rescan gain — grtx1 stamps the typed identity fast-path output with `from_f64_values_all_valid_unchecked` after proving other all-valid + self-present excludes NaN, avoiding the redundant output validity scan. Same-worker rch hz2 FP-side A/B: 19.375 ms baseline → 18.332/18.400 ms after = ~1.05× faster, focused test green. Still a pandas loss; residual = output allocation/select floor (mimalloc/packed select still needed) |
| Series.update (updf, bench_update_cc) | 2M f64 same index, other ~50% NaN | 3.67 ms | 20.0 ms | **0.18× (5.5× slower)** | ✅ IMPROVED from 59× LOSS (216ms)! killed the pointer-key BTreeMap<&IndexLabel,&Scalar> (O(n log n) + 2 cache misses/row, fp-index pointer-key pathology) + both values() materializes. Typed identity fast path (mirror combine_first, other-first): same-index + all-valid Float64 self + Float64 other ⇒ out[i] = other-present ? other : self. 10.8× FP-side, bit-identical (positional on identical unique index; other-present == label-in-other-&&-not-missing), conformance green (5 tests). Residual 5.5× = output-alloc floor. (pandas update is in-place; fp builds new) |
| Index.get_indexer unsorted unique Int64 — route through cached i64 resolver (c90bo follow-on) | 1M unsorted unique Int64 self, 1000 targets, repeated best-of-100 CPU7 | 0.0140 ms | 0.0039 ms | **3.6× faster** | ✅ FIXED — was 210× SLOWER (2.92 ms repeated)! `get_indexer_i64` rebuilt its `FxHashMap<i64,usize>` every call; pandas caches its int64 engine. Unsorted unique Int64 self now reuses the cached `unsorted_unique_int64_positions`; duplicate self keeps the per-call first-occurrence builder. Bit-identical, full conformance green. |
| merge/join inner on Utf8 keys (probe_join_inner_utf8) | 1M×1M string keys → 500k rows, oneshot | 125.6 ms | 300 ms (best) | **0.42× (2.4× slower)** | ⏸️ DEFERRED (bead f1ftd) — real loss but moderate + partly output-allocation-bound (500k×3 cols); fp-join build maps key on pointer `&IndexLabel`/`&CompositeJoinKey` vs pandas khash inline. fp-join is hot/complex/peer-reserved; needs an isolated owner pass, not a quick lever. Recorded, not shipped. |
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
ffill to 2.371 ms vs pandas 3.340 ms = 1.41× faster. concat remains the active rebuild-class
loss; the algorithm-class ops (dedup/value_counts/std/grouping) consistently win.

fp shift/concat rebuild a whole new typed Column (`as_f64/i64_slice` materializes the typed
buffer for `from_values`-built columns, then `from_f64/i64_values` re-inits validity, then
`Series::new`) — multiple O(n) passes — whereas pandas shift/concat is ~one numpy memmove/
concatenate. The typed levers are NOT regressions (≥ old fp Scalar rebuild) so kept, but the
WHOLE-COLUMN-REBUILD construction is fp's structural disadvantage on these ops. INSIGHT: the
typed-slice levers win big when typed access unlocks a cheaper ALGORITHM (FxHash dedup, dense
value_counts, Welford std → 2–11× wins) but only break even (then lose on construction
overhead) for ops that merely rebuild the column (shift/concat). Kernel-level fix needed.

### Gap: concat 24× slower than pandas (biggest gap found)
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
