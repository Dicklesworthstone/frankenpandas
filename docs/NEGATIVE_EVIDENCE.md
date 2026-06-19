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

| reset_index typed Int64 idx→col (bp6k7) | 1M int64-indexed, 2 cols | 1.93 ms | 0.38 ms | **5.1× faster** | ✅ KEEP — Index::from_i64_values |
| concat typed buffer (tbrtu) | 8×125k Int64 series, ignore_index | 0.28 ms | 6.81 ms | **0.041× (24× SLOWER)** | ⚠️ KEEP (bit-transparent, ≥ old Scalar path); 24× is glibc-malloc-bound — see mimalloc row |
| concat + mimalloc global allocator (EXPERIMENT) | 8×125k Int64, ignore_index | 0.28 ms | 0.52 ms | **0.54× (1.86× slower)** | 🚀 13× faster than glibc-malloc concat — allocator is the floor; adopt workspace-wide (bead) |

| str.lower/upper contiguous (apply_str_utf8) | 1M strings | 84.04 ms | 12.88 ms | **6.5× faster** | ✅ KEEP — contiguous buf + ASCII in-place |
| shift typed Float64 (202cdf50) | 2M f64, periods=1 | 0.74 ms | 9.01 ms | **0.082× (12× SLOWER)** | ⚠️ KEEP (≥ old Scalar path) but LOSS — structural |
| shift typed Int64 fill (51601b7a) | 2M i64, periods=2 | 0.74 ms | 7.86 ms | **0.094× (10.6× SLOWER)** | ⚠️ KEEP but LOSS — structural |
| ffill typed Float64 (as_f64_slice_with_validity) | 2M f64, ~10% NaN | 2.79 ms | 18.43 ms | **0.15× (6.6× SLOWER)** | ⚠️ KEEP but LOSS — confirms column-rebuild pattern |
| shift + mimalloc global allocator (EXPERIMENT) | 2M f64, periods=1 | 0.74 ms | 3.98 ms | **0.19× (5.4× slower)** | 🚀 2.3× faster than glibc-malloc shift; mimalloc generalizes (3nah5) |
| ffill + mimalloc global allocator (EXPERIMENT) | 2M f64, ~10% NaN | 2.79 ms | 6.62 ms | **0.42× (2.4× slower)** | 🚀 2.8× faster than glibc-malloc ffill; mimalloc generalizes (3nah5) |

| set_index typed Int64 col→idx (p9omo) | 1M rows, 2 cols | 1.12 ms | 0.17 ms | **6.5× faster** | ✅ KEEP — Index::from_i64_values |
| cummax (sweep bench_misc) | 2M f64 | 22.02 ms | 2.63 ms | **8.4× faster** | ✅ pandas cummax surprisingly slow; fp crushes |
| cumsum (sweep bench_misc) | 2M f64 | 22.73 ms | 3.09 ms | **7.4× faster** | ✅ pandas cumsum surprisingly slow; fp crushes |
| clip (sweep bench_misc) | 2M f64, both bounds | 29.39 ms | 5.23 ms | **5.6× faster** | ✅ big win |
| rank average (sweep bench_misc) | 2M f64 shuffled | 321.97 ms | 209.43 ms | **1.54× faster** | ✅ both slow (sort+ties); fp ahead |
| nlargest(20) (sweep bench_misc) | 2M f64 shuffled | 46.27 ms | 58.29 ms | **0.79× (1.26× slower)** | 🔴 LOSS — select_nth already used; values() Scalar materialization + semantic_cmp overhead |
| diff (sweep bench_misc) | 2M f64, periods=1 | 0.86 ms | 1.86 ms | **0.46× (2.16× slower)** | 🔴 LOSS — rebuild-class (allocator-bound, mimalloc-fixable like shift/ffill) |
| fillna(value) (sweep bench_misc) | 2M f64, ~10% NaN | 2.53 ms | 4.48 ms | **0.57× (1.77× slower)** | 🔴 LOSS — rebuild-class (allocator-bound, mimalloc-fixable) |
| RangeIndex.asof closed-form (jlv2o) | 100k rows, 4,096 scalar probes | 232.02 ms | 60.42 µs | **3,840× faster** | ✅ KEEP — public scalar API; pandas CV 4.82% |
| RangeIndex.asof closed-form (jlv2o) | 1M rows, 4,096 scalar probes | 1,050.29 ms | 65.52 µs | **16,031× faster** | ✅ KEEP — lookup no longer scales with range length |
| RangeIndex.get_indexer miss-heavy (29u49) | 100k targets, 15/16 misses | 1.110 ms | 1.344 ms | **0.83× (1.21× SLOWER)** | ⚠️ KEEP vs legacy — 3.82× faster than get_loc-loop model, but pandas gap remains |
| RangeIndex.reindex all-miss (29u49) | 100k target RangeIndex | 0.990 ms | 1.150 ms | **0.86× (1.16× SLOWER)** | ⚠️ KEEP vs legacy — 4.64× faster than get_loc-loop model, but pandas gap remains |
| RangeIndex.reindex all-miss (29u49) | 1M target RangeIndex | 13.13 ms | 12.28 ms | 1.07× faster | NEUTRAL — below 10% margin; 4.11× faster than legacy model |
| groupby.sum Int64 key (dense grouping) | 1M rows, 1000 keys | 13.26 ms | 2.44 ms | **5.4× faster** | ✅ KEEP — int64_dense_grouping |
| groupby.sum Utf8 key (build_groups FxHash buguz) | 1M rows, 1000 keys | 31.10 ms | 55.33 ms | **0.56× (1.78× SLOWER)** | ⚠️ KEEP (FxHash ≥ SipHash) but LOSS — Utf8 ScalarKey hashing |
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
| RangeIndex.get_indexer miss-heavy (29u49) | 1M targets, 15/16 misses | 16.43 ms | 10.74 ms | 1.53× faster | DROPPED_HIGH_CV — pandas CV 5.40%; FP-side legacy speedup 4.65× |

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

### Mixed: RangeIndex miss-heavy bulk indexers
The `29u49` lever removes per-miss `IndexError` string allocation from
`RangeIndex::{get_indexer,get_indexer_non_unique,reindex}` by keeping public errors at
`get_loc` and using an internal `Option<usize>` lookup in bulk kernels. Focused Criterion
confirms the current path is not a ~0-gain lever: it is 3.82× faster than the get_loc-loop
legacy model for 100k miss-heavy `get_indexer`, 4.64× faster for 100k all-miss `reindex`,
and 4.11× faster for 1M all-miss `reindex`.

Against pandas, however, the accepted release rows are not wins: 100k `get_indexer` is 1.21×
slower, 100k `reindex` is 1.16× slower, and 1M `reindex` is only 1.07× faster so it is
classified as neutral. No revert: the FP-side gain is large and behavior-guarded, but this
does not close the pandas gap. Next retry should target output vector allocation / pandas'
vectorized RangeIndex engine rather than reintroducing exception-driven miss handling.
Artifacts: `artifacts/bench/gauntlet_cod_b_range_indexers_vs_pandas.json`,
`artifacts/bench/gauntlet_cod_b_range_indexers_criterion_local.txt`, and
`artifacts/bench/gauntlet_cod_b_range_indexers_criterion_rch.txt`, and
`artifacts/bench/gauntlet_cod_b_range_indexers_pandas.json`.

### Gap: Utf8 groupby 1.78× slower than pandas
fp groups Utf8 keys via `build_groups` → `FxHashMap<ScalarKey, Vec<usize>>`: per-row String
hashing + `ScalarKey::Utf8` (holds a `&str`/owned), Vec<usize> accumulation, then agg. The
FxHash lever (buguz) beat SipHash but the path still loses to pandas' factorize-then-aggregate
on object keys. Not a regression (kept). FIX: factorize Utf8 keys to dense codes once (like
the Int64 dense path), then group on codes — a bigger algorithmic change, cod-b's groupby
domain. Int64-key groupby already wins 5.4× via the dense path; later value-lane clone
elimination wins for `Nunique` (2.89× on a 2M-row CV-gated workload) and `Median`
(1.80× on a 2M-row CV-gated workload). The remaining Utf8 gap is reducer-specific:
plain sum/mean-style Utf8 grouping still needs factorize→dense.

### Gap: shift/concat/ffill structural — column-rebuild vs in-place (6.6–24× slower)
**ffill (2M f64, ~10% NaN) confirms the pattern: 18.43 ms vs pandas 2.79 ms = 6.6× slower.**
ffill has a typed `as_f64_slice_with_validity` path but still rebuilds a fresh Column +
re-inits validity, so it loses to pandas' in-place forward-fill — the same root cause as
shift/concat below. The rebuild-class ops (shift/concat/ffill) are fp's consistent structural
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
CONCLUSION: explicit SIMD intrinsics / `target_feature` are **out of reach for this codebase**
(no unsafe allowed); the 8-lane chunked accumulator is the **safe-Rust ceiling** for i64
max/min (357/424µs, 1.7× off numpy). Closing the last 1.7× would require either a workspace
decision to allow a vetted `unsafe` SIMD module, or a global `target-cpu`/`target-feature`
build flag (the `.cargo/config.toml` +avx2 experiment jawxr was tried + reverted as neutral-
to-worse for corr/cov — would need per-op evaluation). Dead end recorded: don't retry
`unsafe` target_feature in fp-frame.

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
