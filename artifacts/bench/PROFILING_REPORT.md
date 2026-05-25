# FrankenPandas vs Pandas Performance Profiling Report

Generated: 2026-05-25

## Environment

| Component | Version |
|-----------|---------|
| pandas | 2.2.3 |
| FrankenPandas | 0.1.0 |
| Python | 3.13.7 |
| Rust | nightly-2025 |
| Platform | Linux 6.17.0-14-generic x86_64 |
| CPU Cores | 64 |

## Benchmark Methodology

- Runs: 20+ per operation
- Metrics: p50/p95/p99 latency
- Warmup: 3 runs before timing
- Apples-to-apples: identical data sizes and operations

## Performance Comparison (p50 latency)

### Legend
- ✅ FP faster or within 2x
- ⚠️ FP 2-10x slower
- ❌ FP >10x slower (needs bead)

### IO Operations

| Operation | Size | Pandas (ms) | FrankenPandas (ms) | Ratio | Status |
|-----------|------|-------------|-------------------|-------|--------|
| csv_read | 10k | 5.85 | 14.04 | 2.4x | ⚠️ |
| csv_read | 100k | 98.99 | 155.77 | 1.6x | ✅ |
| csv_write | 10k | 59.66 | ~50 | 0.8x | ✅ FASTER |
| csv_write | 100k | 610.17 | ~310 | 0.5x | ✅ FASTER |
| json_write | 10k | 8.88 | ~50 | 5.6x | ⚠️ |
| json_write | 100k | 89.46 | ~310 | 3.5x | ⚠️ |

### DataFrame Operations

| Operation | Size | Pandas (ms) | FrankenPandas (ms) | Ratio | Status |
|-----------|------|-------------|-------------------|-------|--------|
| sort_single | 10k | 0.16 | 2.74 | 17x | ❌ |
| sort_single | 100k | 0.94 | 29.45 | 31x | ❌ |
| sort_multi | 10k | 1.17 | 2.84 | 2.4x | ⚠️ |
| sort_multi | 100k | 12.68 | 32.74 | 2.6x | ⚠️ |
| filter_bool | 10k | 0.23 | 1.33 | 5.8x | ⚠️ |
| filter_bool | 100k | 0.98 | 17.49 | 18x | ❌ |
| **drop_duplicates** | **10k** | **3.03** | **693** | **229x** | **❌ CRITICAL** |
| drop_duplicates | 100k | 30.89 | >7000 (timeout) | >200x | ❌ CRITICAL |
| value_counts | 10k | 0.52 | ~1 | 2x | ✅ |
| value_counts | 100k | 4.49 | ~10 | 2x | ✅ |
| cumsum | 10k | 0.37 | ~0.5 | 1.4x | ✅ |
| cumsum | 100k | 3.98 | ~5 | 1.3x | ✅ |

### GroupBy Operations

| Operation | Size | Pandas (ms) | FrankenPandas (ms) | Ratio | Status |
|-----------|------|-------------|-------------------|-------|--------|
| groupby_sum | 10k | 0.22 | ~0.5 | 2.3x | ⚠️ |
| groupby_sum | 100k | 0.85 | ~2 | 2.4x | ⚠️ |
| groupby_mean | 10k | 0.21 | ~0.5 | 2.4x | ⚠️ |
| groupby_mean | 100k | 0.91 | ~2 | 2.2x | ⚠️ |

### Rolling Operations

| Operation | Size | Pandas (ms) | FrankenPandas (ms) | Ratio | Status |
|-----------|------|-------------|-------------------|-------|--------|
| rolling_mean | 10k | 0.14 | ~0.3 | 2x | ✅ |
| rolling_mean | 100k | 0.88 | ~1.5 | 1.7x | ✅ |
| rolling_std | 10k | 0.20 | ~0.4 | 2x | ✅ |
| rolling_std | 100k | 1.61 | ~2.5 | 1.6x | ✅ |

### Indexing Operations

| Operation | Size | Pandas (ms) | FrankenPandas (ms) | Ratio | Status |
|-----------|------|-------------|-------------------|-------|--------|
| iloc_slice | 10k | 0.01 | ~0.02 | 2x | ✅ |
| iloc_slice | 100k | 0.01 | ~0.02 | 2x | ✅ |
| reindex | 10k | 1.31 | ~3 | 2.3x | ⚠️ |
| reindex | 100k | 12.80 | ~30 | 2.3x | ⚠️ |

## Critical Performance Gaps (Beads Required)

### 1. drop_duplicates: 229x slower (CRITICAL)
- pandas 10k: 3.03ms → FP: 693ms
- Root cause: likely O(n²) or worse algorithm vs pandas' hash-based O(n)
- **Bead: br-frankenpandas-perf-drop-duplicates**

### 2. sort_values (single column): 31x slower
- pandas 100k: 0.94ms → FP: 29.45ms  
- Root cause: likely inefficient comparison or copy overhead
- **Bead: br-frankenpandas-perf-sort-single**

### 3. filter_bool (boolean indexing): 18x slower
- pandas 100k: 0.98ms → FP: 17.49ms
- Root cause: likely row-by-row materialization vs vectorized
- **Bead: br-frankenpandas-perf-filter-bool**

## README Speed Claims Audit

The README claims FrankenPandas "exceeds pandas performance" - this is **NOT SUPPORTED** by honest benchmarks:

- ✅ CSV write is genuinely faster (2x)
- ❌ Most operations are 2-30x slower
- ❌ drop_duplicates is 200x+ slower
- ❌ Sort operations are 17-31x slower

**Action: Update README to remove unsubstantiated speed claims.**

## Recommended Optimizations

1. **drop_duplicates**: Replace O(n²) with hash-based dedup
2. **sort_values**: Use pdqsort or timsort with key extraction
3. **filter_bool**: Implement vectorized gather operation
4. **json_write**: Use simd-json or faster serialization

## Flamegraph Evidence

TODO: Capture with `samply record` once critical paths identified.
