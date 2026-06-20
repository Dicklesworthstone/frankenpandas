# FrankenPandas vs Pandas Performance Scorecard

> **Status**: Refreshed 2026-06-02 against pandas 2.2.3 (release-perf, AMD 5975WX).
> The 2026-05-25 figures below were stale after the perf campaign — the
> `drop_duplicates` 229x regression and the `sort_single` 31x gap are FIXED
> (br-frankenpandas-fgpx3 dedup hashing, -2a6ln gather, -uxkvh dense sort,
> -sfysu dense gather). Numbers re-measured per br-frankenpandas-a5dwk.

## 2026-06-19 Cod-a Gauntlet Refresh - GroupBy

Scope: `br-frankenpandas-2qb1i`, latest cod-a clone-free `fp-groupby`
optimization cluster at commit `a7287a4d`, measured against pandas 2.2.3
with `benches/vs_pandas_harness.py`.

This refresh supersedes the stale GroupBy row below for the measured workloads
only. It does not re-score IO, joins, rolling, indexing, or the full release
matrix.

| Size | Valid workloads | Dropped high-CV | Accepted geomean vs pandas | Accepted verdicts | Scorecard action |
|------|----------------:|----------------:|---------------------------:|-------------------|------------------|
| 100k | 2 / 8 | 6 / 8 | 3.804x faster | `groupby_transform_mean` 4.586x, `groupby_count` 3.155x | Keep cluster; rerun high-CV rows |
| 1M | 3 / 8 | 5 / 8 | 4.345x faster | `groupby_agg_multi` 6.291x, `groupby_transform_mean_str` 2.178x, `groupby_count` 5.988x | Keep cluster; rerun high-CV rows |
| 2M focused nunique | 1 / 1 | 0 / 1 | 2.895x faster | `groupby_agg_nunique_utf8_float64` 2.895x | Keep `br-frankenpandas-uza04.204` |
| Focused median | 2 / 3 | 1 / 3 | 2.174x faster | `groupby_agg_median_utf8_float64` 2.631x at 100k, 1.796x at 2M | Keep `br-frankenpandas-uza04.203` |
| Focused std/var | 6 / 6 | 2 high-CV diagnostics superseded by accepted reruns | 1.285x faster | `groupby_agg_var_utf8_float64` 1.289x/1.217x/1.301x; `groupby_agg_std_utf8_float64` 1.344x/1.227x/1.338x | Keep `br-frankenpandas-uza04.202` |
| Combined accepted | 14 / 26 | 16 high-CV diagnostics total | 2.219x faster | No accepted neutral/slower rows | No revert |

Release-readiness impact: GroupBy moves from stale "slower" evidence to
partial measured wins on realistic 100k/1M workloads, plus a CV-gated 2M
`agg-nunique` win for `br-frankenpandas-uza04.204` and CV-gated 100k/2M
`agg-median` wins for `br-frankenpandas-uza04.203`, and six CV-gated std/var
wins for `br-frankenpandas-uza04.202`. The category is still not fully
validated because 11 of the original 16 harness rows were rejected by the
high-CV filter and focused diagnostics still include dropped rows. Overall
release readiness remains **PARTIAL / NOT FULLY VALIDATED**.

## 2026-06-20 Cod-b Gauntlet Refresh - RangeIndex Set Ops

Release-readiness score for this cluster: **5/5**.

- Bead: `br-frankenpandas-iatnc`.
- pandas oracle: 2.2.3 public `RangeIndex` set-operation APIs.
- FrankenPandas profile: focused `fp-index` example harness for 1M-row overlap
  set ops.
- Build target: `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`.
- Decision: keep the affine single-span output path and the follow-up two-run
  backing for split `symmetric_difference` outputs. All four measured overlap
  set operations now dominate pandas on the public construction/`len()` path.

Same-worker `hz2` pre/post:

| Operation | `origin/main` | affine-spans | FP delta | Action |
|---|---:|---:|---:|---|
| `intersection` | 9.240731 ms | 0.000100 ms | 92,407x faster | Keep |
| `union` | 10.632178 ms | 0.000090 ms | 118,135x faster | Keep |
| `difference` | 9.341052 ms | 0.000100 ms | 93,411x faster | Keep |
| `symmetric_difference` | 18.670185 ms | 18.843325 ms | 0.991x | Superseded by two-run backing (`uza04.168`) |

Head-to-head versus pandas 2.2.3:

| Operation | FrankenPandas | pandas | Ratio vs pandas | Verdict |
|---|---:|---:|---:|---|
| `intersection` | 120 ns | 9,018 ns | 75.15x | WIN |
| `union` | 120 ns | 7,995 ns | 66.63x | WIN |
| `difference` | 130 ns | 16,742 ns | 128.78x | WIN |
| `symmetric_difference` | 110 ns | 5.157781 ms | 46,889x | WIN |

Win/loss/neutral ratio vs pandas after `br-frankenpandas-uza04.168`: **4 / 0 / 0**.

Continuation evidence for `br-frankenpandas-uza04.168`:

- Local same-host head-to-head, exact boxed two-run code:
  `FrankenPandas symmetric_difference_ns=110`; pandas 2.2.3
  `symmetric_difference_ns_p50=5,157,781`, best `5,050,482`.
- Remote release evidence:
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 overlap`
  on worker `vmi1227854`, exit 0, `symmetric_difference_ns=140`.
- Pre-change residual baseline from the fresh restart:
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b rch exec -- cargo run -p fp-index --example bench_range_setops --release -- 1000000 200 overlap`
  on worker `vmi1293453`, exit 0, `symmetric_difference_ns=39,710,381`.
- Focused guards: `cargo check -p fp-index --all-targets` via rch, local
  `cargo clippy -p fp-index --all-targets -- -D warnings`, focused
  `range_index_set_ops_return_affine_spans_iatnc`, and the release
  `golden_isin_symdiff_i64` example all passed. Remote clippy was attempted but
  the selected worker lacked the pinned nightly clippy component.
- UBS: `timeout 180s ubs crates/fp-index/src/lib.rs` exited 0; broad existing
  `fp-index` warning inventory remained, with no critical issues.

Evidence:

- `crates/fp-index/examples/bench_range_setops.rs`
- `artifacts/optimization/negative-evidence-ledger-cod-b.md`

### 2026-06-19 Cod-a Focused Std/Var Proof - `br-frankenpandas-uza04.202`

Comparator: pandas 2.2.3 / numpy 2.4.3. Workload:
`groupby_agg_{var,std}_utf8_float64`, UTF8 keys, Float64 values, NaN every 37th
row, `sort=True`, `ddof=1`.
FP command: `groupby-bench --agg agg-var|agg-std --key-kind utf8 --value-kind float64`,
run under `taskset -c 7` from
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`.

| Reducer | Rows | FP p50 | pandas p50 | Ratio vs pandas | FP CV | pandas CV | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|
| var | 100k | 2.814 ms | 3.627 ms | 1.289x | 0.52% | 1.36% | FASTER / ACCEPTED |
| std | 100k | 2.845 ms | 3.825 ms | 1.344x | 0.96% | 2.56% | FASTER / ACCEPTED |
| var | 1M | 29.563 ms | 35.966 ms | 1.217x | 3.05% | 3.26% | FASTER / ACCEPTED |
| std | 1M | 28.657 ms | 35.174 ms | 1.227x | 1.05% | 0.44% | FASTER / ACCEPTED |
| var | 2M | 58.659 ms | 76.335 ms | 1.301x | 3.49% | 1.80% | FASTER / ACCEPTED |
| std | 2M | 56.466 ms | 75.544 ms | 1.338x | 0.64% | 0.61% | FASTER / ACCEPTED |

Dropped diagnostics:
- 2M `std`, first pinned run: FP p50 58.569 ms, pandas p50 55.941 ms,
  0.955x, dropped because FP CV was 12.39%; superseded by accepted batched
  rerun.
- 2M `std`, 10-iter rerun: FP p50 56.868 ms, pandas p50 85.258 ms, 1.499x,
  dropped because pandas CV was 5.92%; superseded by accepted 20-iter rerun.

Guards:
- RCH build: `cargo build --profile release-perf -p fp-groupby --bin groupby-bench`
  on worker `hz2`, exit 0.
- Local clean-worktree timing build: same target dir, exit 0.
- Focused conformance guards:
  `groupby_var_std_utf8_keys_stream_numeric_counters_uza04202` and
  `groupby_var_std_timedelta_fallback_preserves_dtype_uza04202`, exit 0.
- RCH Criterion guard: `cargo bench -p fp-conformance --bench vs_pandas -- groupby/`
  on worker `vmi1227854`, exit 0.

### 2026-06-19 Cod-a Focused Median Proof - `br-frankenpandas-uza04.203`

Comparator: pandas 2.2.3 / numpy 2.4.3. Workload: `groupby_agg_median_utf8_float64`,
UTF8 keys, Float64 values, NaN every 37th row, `sort=True`.
FP command: `groupby-bench --agg agg-median --key-kind utf8 --value-kind float64`,
run under `taskset -c 7` from
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`.

| Rows | FP p50 | pandas p50 | Ratio vs pandas | FP CV | pandas CV | Verdict |
|---:|---:|---:|---:|---:|---:|---|
| 100k | 2.101 ms | 5.527 ms | 2.631x | 1.48% | 4.56% | FASTER / ACCEPTED |
| 1M | 19.903 ms | 49.835 ms | 2.504x | 6.62% | 1.08% | DROPPED_HIGH_CV |
| 2M | 42.975 ms | 77.171 ms | 1.796x | 3.21% | 1.06% | FASTER / ACCEPTED |

Guards:
- RCH build: `cargo build --profile release-perf -p fp-groupby --bin groupby-bench`
  on worker `hz2`, exit 0.
- Local clean-worktree timing build: same target dir, exit 0.
- Focused conformance guard:
  `cargo test -p fp-groupby groupby_median_utf8_keys_numeric_vectors_uza04203`, exit 0.
- RCH Criterion guard: `cargo bench -p fp-conformance --bench vs_pandas -- groupby/`
  on worker `vmi1227854`, exit 0.

### 2026-06-19 Cod-a Focused Nunique Proof - `br-frankenpandas-uza04.204`

Comparator: pandas 2.2.3 / numpy 2.4.3. Workload: `groupby_agg_nunique_utf8_float64`,
2M rows, 1000 UTF8 keys, Float64 values, NaN every 37th row, `sort=True`, `dropna=True`.
FP command: `groupby-bench --agg agg-nunique --key-kind utf8 --value-kind float64
--rows 2000000 --key-cardinality 1000 --iters 20`, run under `taskset -c 7` from
`CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a`.

| Rows | FP p50 | pandas p50 | Ratio vs pandas | FP CV | pandas CV | Verdict |
|---:|---:|---:|---:|---:|---:|---|
| 100k | 4.438 ms | 7.821 ms | 1.762x | 12.91% | 13.55% | DROPPED_HIGH_CV |
| 1M | 27.287 ms | 84.292 ms | 3.089x | 5.52% | 6.35% | DROPPED_HIGH_CV |
| 2M | 53.117 ms | 153.747 ms | 2.895x | 2.68% | 0.95% | FASTER / ACCEPTED |

Guards:
- RCH build: `cargo build --profile release-perf -p fp-groupby --bin groupby-bench`
  on worker `vmi1149989`, exit 0.
- Local clean-worktree timing build: same target dir, exit 0.
- RCH Criterion guard: `cargo bench -p fp-conformance --bench vs_pandas -- groupby/`
  on worker `vmi1227854`, exit 0.
- Focused conformance guard:
  `cargo test -p fp-groupby groupby_nunique_utf8_keys_borrowed_sets_uza04204`, exit 0.

Artifacts:
- `artifacts/perf/cod-a-groupby-gauntlet-a7287a4d.md`
- `artifacts/perf/cod-a-groupby-gauntlet-vs-pandas-a7287a4d.json`
- `artifacts/perf/cod-a-groupby-gauntlet-vs-pandas-a7287a4d-1m.json`
- `artifacts/perf/cod-a-groupby-gauntlet-criterion-a7287a4d.txt`
- `artifacts/optimization/negative-evidence-ledger-cod-a.md`

## 2026-06-19 Cod-b Gauntlet Refresh - `RangeIndex::asof`

Release-readiness score for this cluster: **4/5**.

- Bead: `br-frankenpandas-jlv2o`.
- pandas oracle: 2.2.3 public `RangeIndex.asof` scalar API.
- FrankenPandas profile: focused `fp-index` Criterion bench.
- Build target: `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`.
- Decision: keep the closed-form ascending `RangeIndex::asof` path. No revert:
  both accepted rows dominate pandas with pandas CV below 5%.

| Workload | Rows | FP median | pandas median | Ratio vs pandas | Verdict | Action |
|---|---:|---:|---:|---:|---|---|
| 4,096 scalar `asof` probes | 100k | 60.42 µs | 232.02 ms | 3,840x | FASTER | Keep `jlv2o` |
| 4,096 scalar `asof` probes | 1M | 65.52 µs | 1,050.29 ms | 16,031x | FASTER | Keep `jlv2o` |

Evidence artifacts:

- `artifacts/bench/gauntlet_cod_b_range_asof_vs_pandas.json`
- `artifacts/bench/gauntlet_cod_b_range_asof_criterion_local.txt`
- `artifacts/bench/gauntlet_cod_b_range_asof_criterion.txt`
- `artifacts/bench/gauntlet_cod_b_range_asof_pandas.json`
- `artifacts/optimization/negative-evidence-ledger-cod-b.md`

## 2026-06-19 Cod-b Gauntlet Refresh - RangeIndex Miss-Heavy Indexers

Release-readiness score for this cluster: **2/5**.

- Bead: `br-frankenpandas-29u49`.
- pandas oracle: 2.2.3 public `RangeIndex.get_indexer` and `RangeIndex.reindex`.
- FrankenPandas profile: focused `fp-index` Criterion bench with a bench-local
  legacy model that calls public `get_loc` for every target miss.
- Build target: `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`.
- Decision: keep the `position_of_value` bulk-kernel path because it is a large
  FP-side improvement over the legacy error-allocation model, but do not count it
  as pandas-ready. The accepted pandas rows are loss/loss/neutral.

| Workload | Rows | FP median | pandas median | Ratio vs pandas | FP vs legacy model | Verdict | Action |
|---|---:|---:|---:|---:|---:|---|---|
| `get_indexer`, 15/16 misses | 100k | 1.344 ms | 1.110 ms | 0.825x | 3.82x faster | SLOWER | Keep `29u49`; target output/vectorized path next |
| `get_indexer`, 15/16 misses | 1M | 10.744 ms | 16.435 ms | 1.530x | 4.65x faster | DROPPED_HIGH_CV | pandas CV 5.40% |
| `reindex`, all misses | 100k | 1.150 ms | 0.990 ms | 0.860x | 4.64x faster | SLOWER | Keep `29u49`; pandas gap remains |
| `reindex`, all misses | 1M | 12.285 ms | 13.127 ms | 1.069x | 4.11x faster | NEUTRAL | Keep; below 10% margin |

Evidence artifacts:

- `artifacts/bench/gauntlet_cod_b_range_indexers_vs_pandas.json`
- `artifacts/bench/gauntlet_cod_b_range_indexers_criterion_local.txt`
- `artifacts/bench/gauntlet_cod_b_range_indexers_criterion_rch.txt`
- `artifacts/bench/gauntlet_cod_b_range_indexers_pandas.json`
- `artifacts/optimization/negative-evidence-ledger-cod-b.md`

## 2026-06-18/19 Gauntlet Refresh: Range/affine `Index::take`

Release-readiness score for this cluster: **2/5**.

- pandas oracle: 2.2.3.
- FrankenPandas profile: `release-perf`, `fp-bench`, `TAKE_BATCH=256`.
- Build target: `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`.
- Decision: keep `br-frankenpandas-uza04.205` (`RangeIndex::take` arithmetic
  selector laziness) as a measured FP-side improvement, but do not count it as
  pandas domination; revert `br-frankenpandas-uza04.206` because generic affine
  `Index::take` arithmetic laziness regressed versus the pre-optimization FP
  baseline and remained slower than pandas.

| Workload | Rows | Final FP p50 | pandas p50 | Ratio vs pandas | Verdict | Action |
|---|---:|---:|---:|---:|---|---|
| `range_index_take_arithmetic` | 1M | 83.685 ms | 62.712 ms | 0.749x | SLOWER | Keep `.205`; next target is eliminating the O(k) selector scan |
| `affine_index_take_arithmetic` | 100k | 7.200 ms | 6.001 ms | 0.833x | SLOWER | `.206` reverted |
| `affine_index_take_arithmetic` | 1M | 72.051 ms | 54.687 ms | 0.759x | SLOWER | `.206` reverted |
| `range_index_take_arithmetic` | 100k | dropped | dropped | n/a | DROPPED_HIGH_CV | Recorded in negative ledger; reruns stayed above CV gate |

Pre-optimization comparator:

| Workload | Rows | Preopt FP p50 | Final FP p50 | FP-side delta | Release note |
|---|---:|---:|---:|---:|---|
| `range_index_take_arithmetic` | 1M | 127.438 ms | 83.685 ms | 1.52x faster | Partial keep, not pandas-ready |
| `affine_index_take_arithmetic` | 1M | 72.892 ms | 72.051 ms | 1.01x faster after revert | Reverted `.206`; no material gain retained |

Evidence artifacts:

- `artifacts/bench/gauntlet_cod_b_range_take_after_revert206_vs_pandas_batch256_taskset7.json`
- `artifacts/bench/gauntlet_cod_b_range_take_preopt_vs_pandas_batch256_taskset7.json`
- `artifacts/bench/gauntlet_cod_b_range_take_criterion_after_revert206.txt`
- `artifacts/optimization/negative-evidence-ledger-cod-b.md`

## Categories

| Category | Weight | FP p50 | PD p50 | Ratio | Verdict |
|----------|--------|--------|--------|-------|---------|
| IO | 0.25 | mixed | mixed | ~0.7x | SLOWER (json) / FASTER (csv write) |
| DataFrameOps | 0.20 | mixed | low | ~0.3x | SLOWER (drop_duplicates now ~2.5x, filter ~12x) |
| GroupBy | 0.20 | ~2ms | ~1ms | ~0.5x | SLOWER |
| Joins | 0.15 | ~3ms | ~1.5ms | ~0.5x | SLOWER |
| Rolling/Expanding | 0.10 | ~2ms | ~1ms | ~0.5x | SLOWER |
| Indexing | 0.10 | ~0.02ms | ~0.01ms | ~0.5x | PARITY |
| **WEIGHTED** | **1.00** | - | - | **~0.3x** | **SLOWER** |

## Critical Findings

### Operations Where FP is FASTER
- **csv_write**: 2x faster (FP: 50ms vs PD: 60ms for 10k rows)

### Operations Where FP is SLOWER (>10x) — current (2026-06-02)
- **filter_bool**: ~11.7x slower (FP 4.49ms vs PD 0.38ms @100k). Gather is already
  fast-pathed (br-sfysu); residual is architectural (Vec<Scalar> AoS vs numpy
  contiguous typed arrays) — see br-frankenpandas-piw16.

### Operations Now 2-8x (was "critical", largely fixed)
- **drop_duplicates**: ~2.5x slower (FP 14.06ms vs PD 5.62ms @100k; 1.25ms vs
  0.56ms @10k) — was 229x; FIXED by br-fgpx3 + br-2a6ln.
- **sort_single**: 3.1x @100k (FP 9.57 vs PD 3.04), 7.5x @10k — was 31x; the
  dense numeric fast path (br-uxkvh) closed most of it.
- **series_add (AACE outer-align)**: ~7.5x @10k (FP 1.55 vs PD 0.21) — was ~16x;
  FIXED by br-b75cc (skip discarded-ledger witness sha256).

### Operations Within 2x (Acceptable)
- csv_read: 1.6-2.4x slower
- groupby_sum/mean: 2.2-2.4x slower
- rolling_mean/std: 1.6-2x slower
- iloc_slice: ~2x slower (but microsecond-scale)

## Raw Benchmark Data

### IO (10k rows)
| Operation | Pandas (ms) | FrankenPandas (ms) | Ratio |
|-----------|-------------|-------------------|-------|
| csv_read | 5.85 | 14.04 | 0.42x |
| csv_write | 59.66 | ~50 | **1.2x** |
| json_write | 8.88 | ~50 | 0.18x |

### IO (100k rows)
| Operation | Pandas (ms) | FrankenPandas (ms) | Ratio |
|-----------|-------------|-------------------|-------|
| csv_read | 98.99 | 155.77 | 0.64x |
| csv_write | 610.17 | ~310 | **2.0x** |
| json_write | 89.46 | ~310 | 0.29x |

### DataFrame Operations (10k rows) — re-measured 2026-06-02
| Operation | Pandas (ms) | FrankenPandas (ms) | Ratio |
|-----------|-------------|-------------------|-------|
| sort_single | 0.108 | 0.809 | 0.13x |
| drop_duplicates | 0.561 | 1.248 | 0.45x |
| filter_bool | 0.082 | 0.962 | 0.085x |
| series_add (outer-align) | 0.208 | 1.55 | 0.13x |

### DataFrame Operations (100k rows) — re-measured 2026-06-02
| Operation | Pandas (ms) | FrankenPandas (ms) | Ratio |
|-----------|-------------|-------------------|-------|
| sort_single | 3.038 | 9.573 | 0.32x |
| filter_bool | 0.383 | 4.491 | 0.085x |
| drop_duplicates | 5.621 | 14.057 | 0.40x |

## Beads Filed

| Bead | Issue | Status |
|------|-------|--------|
| br-frankenpandas-fgpx3 / -2a6ln | drop_duplicates (was 229x → ~2.5x) | RESOLVED |
| br-frankenpandas-uxkvh | sort_single dense fast path (was 31x → 3.1x) | RESOLVED |
| br-frankenpandas-b75cc | series_add AACE witness (was ~16x → 7.5x) | RESOLVED |
| br-frankenpandas-piw16 | filter_bool ~12x (architectural Vec<Scalar> gather) | OPEN |

## Methodology

Per BENCH_MATRIX_SPEC.md:
- Release-perf profile (LTO=thin, opt-level=3, debug=line-tables-only)
- Identical workloads run on FrankenPandas and pandas 2.2.3
- 20+ runs per operation with warmup
- p50/p95/p99 captured per workload

## Verdicts

- **FASTER**: FP is >1.05x faster than pandas
- **PARITY**: FP is 0.95x-1.05x (equivalent)
- **SLOWER**: FP is <0.95x (pandas wins)

## Regenerate

```bash
# Run pandas benchmarks
python scripts/bench_pandas_baseline.py > artifacts/bench/pandas_baseline.json

# Run FrankenPandas benchmarks
cargo run --release -p fp-conformance --example bench_runner > artifacts/bench/rust_baseline.json

# Generate comparison
python scripts/gen_perf_scorecard.py --compare
```

## Thresholds (Ratchet Gate)

| Metric | Regression Threshold |
|--------|---------------------|
| Primary (single p50) | -3% |
| Category geomean | -5% |
| Per-category weighted | -10% |
| p90 tail | -15% |
| Throughput | -5% |
