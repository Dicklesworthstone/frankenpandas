# FrankenPandas vs Pandas Performance Scorecard

> **Status**: Current measured results are listed with their benchmark date,
> incumbent version, and evidence provenance.

## 2026-07-26 Lane M Median-CI Re-adjudication — GroupBy

All 16 rows ran on strict-remote worker `vmi1149989` under schema v4:
executing-ELF SHA-256, per-arm A/A null in the same invocation, and bootstrap
median-CI gating. CV had no vote.

| re-adjudicated rows | `FASTER` | `NULL-UNDECIDABLE` | rejected on CV | decidable geomean vs pandas |
|---:|---:|---:|---:|---:|
| 16 / 16 | **14** | 2 | **0** | **5.7333x faster** |

The two `NULL-UNDECIDABLE` rows are `groupby_agg_median_utf8_float64` at 1M
with NaN every 37th and the first independent
`groupby_agg_std_utf8_float64` 2M repeat. The second exact `std` repeat is
`FASTER` at 3.044x. The 14 decisive ratios span 1.946x–19.486x. The two null
rows retain batching-based retry predicates.

Canonical evidence:

- `artifacts/bench/cod_lane_m_gauntlet_original_100k_median_ci_20260726.json`
- `artifacts/bench/cod_lane_m_gauntlet_original_1m_median_ci_20260726.json`
- `artifacts/bench/cod_lane_m_gauntlet_nunique_median_ci_20260726.json`
- `artifacts/bench/cod_lane_m_gauntlet_median_median_ci_20260726.json`
- `artifacts/bench/cod_lane_m_gauntlet_std_repeat_a_median_ci_20260726.json`
- `artifacts/bench/cod_lane_m_gauntlet_std_repeat_b_median_ci_20260726.json`

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

Same-worker `hz2` fp-before/fp-after maintenance evidence (not campaign
output):

| Operation | `origin/main` | affine-spans | FP delta | Action |
|---|---:|---:|---:|---|
| `intersection` | 9.240731 ms | 0.000100 ms | 92,407x faster | Keep |
| `union` | 10.632178 ms | 0.000090 ms | 118,135x faster | Keep |
| `difference` | 9.341052 ms | 0.000100 ms | 93,411x faster | Keep |

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

## 2026-06-22 Cod-b Evidence Closeout - RangeIndex Values

Release-readiness score for this closeout: **3/5 evidence**, **0/5 pandas domination**.

- Bead: `br-frankenpandas-uza04.165`.
- Current implementation: `RangeIndex::values` generates the arithmetic
  progression directly.
- Build/bench target: warm
  `CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-b`.
- FP evidence: focused Criterion group added to
  `crates/fp-index/benches/range_index_indexers.rs`, measured before the
  release comparison. `range_index_values/current_direct_values/1_000_000`
  mean is 0.773 ms; the explicit legacy comparator
  `legacy_flat_index_values/1_000_000` mean was 6.571 ms.
- Oracle: pandas 2.2.3 `pd.RangeIndex(0, 2n, 2).values` plus forced `sum()`
  on the same 1M-label shape, local p50 0.090 ms.

| Workload | FrankenPandas | Legacy FP comparator | pandas | FP-side delta | Ratio vs pandas | Verdict |
|---|---:|---:|---:|---:|---:|---|
| 100k labels | 72.4 us | 236.5 us | 11.1 us | 3.27x faster | 0.15x | Existing direct path verified, still pandas loss |
| 1M labels | 0.773 ms | 6.571 ms | 0.090 ms | 8.50x faster | 0.12x | Existing direct path verified, still pandas loss |

The vs-pandas gap is structural:
pandas exposes a NumPy int64 view/cache and consumes it in C, while
FrankenPandas returns and consumes an owned `Vec<i64>`. A future win needs a
typed view/array consumer path or API-level avoidance of public materialization,
not another `IndexLabel` extraction bypass.

## 2026-06-20 Series.combine_first values residual

Release-readiness score for this surface: **0/5 pandas domination**.

- Bead: `br-frankenpandas-3gsa7`.
- Current result: `Series.combine_first(...).values()` on the 2M same-index
  Float64 NaN-fill workload remains slower than pandas because the public scalar
  API boxes every f64 into `Scalar`.

Head-to-head versus pandas 2.2.3, local CPU7 best-of-50 unless noted:

| FrankenPandas | pandas | Ratio vs pandas | Verdict |
|---:|---:|---:|---|
| 30.444 ms | 6.983 ms | 0.23x | LOSS |

Typed lanes measure `materialize=2.280 ms` and `construct=9.418 us`. The next
viable route is avoiding public
`Vec<Scalar>` for numeric consumers or changing the scalar representation size.

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
| 1M | 64.126 ms | 88.834 ms | 1.385x | 31.79% | 23.83% | NULL-UNDECIDABLE |
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
| 100k | 5.358 ms | 10.425 ms | 1.946x | 11.22% | 11.24% | FASTER / MEDIAN-CI |
| 1M | 49.178 ms | 167.009 ms | 3.396x | 8.43% | 9.83% | FASTER / MEDIAN-CI |
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
- Current implementation: closed-form ascending `RangeIndex::asof`.

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
- Current implementation: the `position_of_value` bulk-kernel path. Its
  fp-before/fp-after ratios are maintenance evidence, not campaign output; the
  incumbent rows below remain loss/loss/unadmitted.

| Workload | Rows | FP median | pandas median | Ratio vs pandas | FP vs legacy model | Verdict | Action |
|---|---:|---:|---:|---:|---:|---|---|
| `get_indexer`, 15/16 misses | 100k | 1.344 ms | 1.110 ms | 0.825x | 3.82x faster | SLOWER | Keep `29u49`; target output/vectorized path next |
| `get_indexer`, 15/16 misses | 1M | 10.744 ms | 16.435 ms | 1.530x | 4.65x faster | UNADMITTED | median-CI rerun required |
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
- Current implementation: `RangeIndex::take` arithmetic-selector laziness is
  present; generic affine `Index::take` uses the ordinary path. The current
  incumbent results are:

| Workload | Rows | FP p50 | pandas p50 | Ratio vs pandas | Status |
|---|---:|---:|---:|---:|---|
| `range_index_take_arithmetic` | 1M | 83.685 ms | 62.712 ms | 0.749x | SLOWER |
| `affine_index_take_arithmetic` | 100k | 7.200 ms | 6.001 ms | 0.833x | SLOWER |
| `affine_index_take_arithmetic` | 1M | 72.051 ms | 54.687 ms | 0.759x | SLOWER |
| `range_index_take_arithmetic` | 100k | — | — | — | UNADMITTED; median-CI rerun required |

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

### Operations 2-8x Slower
- **drop_duplicates**: ~2.5x slower (FP 14.06ms vs PD 5.62ms @100k; 1.25ms vs
  0.56ms @10k).
- **sort_single**: 3.1x slower @100k (FP 9.57 vs PD 3.04), 7.5x slower @10k.
- **series_add (AACE outer-align)**: ~7.5x slower @10k (FP 1.55 vs PD 0.21).

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
| br-frankenpandas-fgpx3 / -2a6ln | drop_duplicates ~2.5x slower | IMPLEMENTED |
| br-frankenpandas-uxkvh | sort_single 3.1x slower | IMPLEMENTED |
| br-frankenpandas-b75cc | series_add AACE witness 7.5x slower | IMPLEMENTED |
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
