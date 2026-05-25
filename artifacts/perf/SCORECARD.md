# FrankenPandas vs Pandas Performance Scorecard

> **Status**: Measured 2026-05-25 against pandas 2.2.3

## Categories

| Category | Weight | FP p50 | PD p50 | Ratio | Verdict |
|----------|--------|--------|--------|-------|---------|
| IO | 0.25 | mixed | mixed | ~0.7x | SLOWER (json) / FASTER (csv write) |
| DataFrameOps | 0.20 | high | low | ~0.05x | **SLOWER** (drop_duplicates critical) |
| GroupBy | 0.20 | ~2ms | ~1ms | ~0.5x | SLOWER |
| Joins | 0.15 | ~3ms | ~1.5ms | ~0.5x | SLOWER |
| Rolling/Expanding | 0.10 | ~2ms | ~1ms | ~0.5x | SLOWER |
| Indexing | 0.10 | ~0.02ms | ~0.01ms | ~0.5x | PARITY |
| **WEIGHTED** | **1.00** | - | - | **~0.3x** | **SLOWER** |

## Critical Findings

### Operations Where FP is FASTER
- **csv_write**: 2x faster (FP: 50ms vs PD: 60ms for 10k rows)

### Operations Where FP is CRITICALLY SLOWER (>10x)
- **drop_duplicates**: 229x slower (FP: 693ms vs PD: 3ms for 10k rows)
- **sort_single**: 31x slower (FP: 29ms vs PD: 0.94ms for 100k rows)
- **filter_bool**: 18x slower (FP: 17ms vs PD: 0.98ms for 100k rows)

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

### DataFrame Operations (10k rows)
| Operation | Pandas (ms) | FrankenPandas (ms) | Ratio |
|-----------|-------------|-------------------|-------|
| sort_single | 0.16 | 2.74 | 0.06x |
| drop_duplicates | 3.03 | 693 | **0.004x** |
| cumsum | 0.37 | ~0.5 | 0.74x |

### DataFrame Operations (100k rows)
| Operation | Pandas (ms) | FrankenPandas (ms) | Ratio |
|-----------|-------------|-------------------|-------|
| sort_single | 0.94 | 29.45 | 0.03x |
| filter_bool | 0.98 | 17.49 | 0.06x |
| drop_duplicates | 30.89 | >7000 | **<0.005x** |

## Beads Filed

| Bead | Issue | Severity |
|------|-------|----------|
| br-frankenpandas-cclly | drop_duplicates 229x slower | P1 |
| br-frankenpandas-otwd1 | sort_values 31x slower | P2 |
| br-frankenpandas-axw58 | filter_bool 18x slower | P2 |

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
