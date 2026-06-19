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
| max typed Int64 (4qs3h) | 2M int64 | 219 µs | 1.15 ms | **0.19× (5.2× SLOWER)** | ⚠️ KEEP (beats old fp Scalar path) but LOSES to numpy SIMD — gap |
| min typed Int64 (4qs3h) | 2M int64 | 230 µs | 1.17 ms | **0.20× (5.1× SLOWER)** | ⚠️ KEEP but LOSES to numpy SIMD — gap |

| reset_index typed Int64 idx→col (bp6k7) | 1M int64-indexed, 2 cols | 1.93 ms | 0.38 ms | **5.1× faster** | ✅ KEEP — Index::from_i64_values |
| concat typed buffer (tbrtu) | 8×125k Int64 series, ignore_index | 0.28 ms | 6.81 ms | **0.041× (24× SLOWER)** | ⚠️ KEEP (bit-transparent, ≥ old Scalar path) but BIG LOSS vs pandas |

| str.lower/upper contiguous (apply_str_utf8) | 1M strings | 84.04 ms | 12.88 ms | **6.5× faster** | ✅ KEEP — contiguous buf + ASCII in-place |
| shift typed Float64 (202cdf50) | 2M f64, periods=1 | 0.74 ms | 9.01 ms | **0.082× (12× SLOWER)** | ⚠️ KEEP (≥ old Scalar path) but LOSS — structural |
| shift typed Int64 fill (51601b7a) | 2M i64, periods=2 | 0.74 ms | 7.86 ms | **0.094× (10.6× SLOWER)** | ⚠️ KEEP but LOSS — structural |
| ffill typed Float64 (as_f64_slice_with_validity) | 2M f64, ~10% NaN | 2.79 ms | 18.43 ms | **0.15× (6.6× SLOWER)** | ⚠️ KEEP but LOSS — confirms column-rebuild pattern |

| set_index typed Int64 col→idx (p9omo) | 1M rows, 2 cols | 1.12 ms | 0.17 ms | **6.5× faster** | ✅ KEEP — Index::from_i64_values |
| RangeIndex.asof closed-form (jlv2o) | 100k rows, 4,096 scalar probes | 232.02 ms | 60.42 µs | **3,840× faster** | ✅ KEEP — public scalar API; pandas CV 4.82% |
| RangeIndex.asof closed-form (jlv2o) | 1M rows, 4,096 scalar probes | 1,050.29 ms | 65.52 µs | **16,031× faster** | ✅ KEEP — lookup no longer scales with range length |
| groupby.sum Int64 key (dense grouping) | 1M rows, 1000 keys | 13.26 ms | 2.44 ms | **5.4× faster** | ✅ KEEP — int64_dense_grouping |
| groupby.sum Utf8 key (build_groups FxHash buguz) | 1M rows, 1000 keys | 31.10 ms | 55.33 ms | **0.56× (1.78× SLOWER)** | ⚠️ KEEP (FxHash ≥ SipHash) but LOSS — Utf8 ScalarKey hashing |
| groupby.agg(nunique) Utf8 key (uza04.204) | 2M rows, 1000 keys, Float64 values, NaN every 37th | 153.75 ms | 53.12 ms | **2.89× faster** | ✅ KEEP — CV-gated accepted; FP CV 2.68%, pandas CV 0.95% |

### High-CV directional rows (not release proof)

| Lever (bead) | Workload | pandas p50 | fp p50 | p50 ratio | verdict |
|---|---|---:|---:|---:|---|
| groupby.agg(nunique) Utf8 key (uza04.204) | 100k rows, 1000 keys | 7.82 ms | 4.44 ms | 1.76× faster | DROPPED_HIGH_CV — FP CV 12.91%, pandas CV 13.55% |
| groupby.agg(nunique) Utf8 key (uza04.204) | 1M rows, 1000 keys, pinned CPU rerun | 84.29 ms | 27.29 ms | 3.09× faster | DROPPED_HIGH_CV — FP CV 5.52%, pandas CV 6.35% |

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

### Gap: Utf8 groupby 1.78× slower than pandas
fp groups Utf8 keys via `build_groups` → `FxHashMap<ScalarKey, Vec<usize>>`: per-row String
hashing + `ScalarKey::Utf8` (holds a `&str`/owned), Vec<usize> accumulation, then agg. The
FxHash lever (buguz) beat SipHash but the path still loses to pandas' factorize-then-aggregate
on object keys. Not a regression (kept). FIX: factorize Utf8 keys to dense codes once (like
the Int64 dense path), then group on codes — a bigger algorithmic change, cod-b's groupby
domain. Int64-key groupby already wins 5.4× via the dense path, and the later `Nunique`
value-clone elimination lane wins 2.89× on a 2M-row CV-gated workload. The remaining Utf8
gap is reducer-specific: plain sum/mean-style Utf8 grouping still needs factorize→dense.

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
— the double-call was NOT the bottleneck. Reverted (working-tree only). CONFIRMED the 24×
gap is STRUCTURAL: `Column::from_i64_values` (validity init) + `Series::new` + per-series
typed-buffer materialization, vs pandas' single flat `np.concatenate`. Closing it needs a
kernel-level concat (build one validity-free typed Column directly, skip Series wrapping) —
out of code-first fp-frame scope. Dead ends recorded: don't retry double-call dedup.

### Gap: max/min lose to numpy SIMD (~5×)
fp `Series.max/min` use `iter().max()` (Option/Ord comparison loop, doesn't auto-vectorize)
vs numpy's SIMD max. NOT a regression (the typed lever beats old fp's Scalar iteration), so
not reverted — but a real vs-pandas gap. FIX ATTEMPT (REVERTED): branchless
`fold(i64::MIN, i64::max)` — built+measured, **max stayed 1.15 ms (~0 gain)**; LLVM did not
auto-vectorize the i64 min/max reduction here. Reverted to the simpler `iter().max()`
(working-tree only, never committed). CONCLUSION: beating numpy SIMD max/min needs explicit
SIMD (portable_simd / chunked manual vectorization) — out of safe-Rust auto-vec reach; a
candidate radical lever for the kernel layer, NOT a code-first fp-frame change. Dead end
recorded — do not retry the branchless-fold approach.

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
