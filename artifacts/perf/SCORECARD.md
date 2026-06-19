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
| Combined accepted | 5 / 16 | 11 / 16 | 4.120x faster | No accepted neutral/slower rows | No revert |

Release-readiness impact: GroupBy moves from stale "slower" evidence to
partial measured wins on realistic 100k/1M workloads, but the category is not
fully validated because 11 of 16 rows were rejected by the harness high-CV
filter. Overall release readiness remains **PARTIAL / NOT FULLY VALIDATED**.

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
