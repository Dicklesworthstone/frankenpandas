# FrankenPandas vs Pandas Performance Scorecard

> **Status**: Infrastructure complete, awaiting benchmark run

## Categories

| Category | Weight | FP p50 | PD p50 | Ratio | Verdict |
|----------|--------|--------|--------|-------|---------|
| IO | 0.25 | TBD | TBD | TBD | PENDING |
| DataFrameOps | 0.20 | TBD | TBD | TBD | PENDING |
| GroupBy | 0.20 | TBD | TBD | TBD | PENDING |
| Joins | 0.15 | TBD | TBD | TBD | PENDING |
| Rolling/Expanding | 0.10 | TBD | TBD | TBD | PENDING |
| Indexing | 0.10 | TBD | TBD | TBD | PENDING |
| **WEIGHTED** | **1.00** | - | - | TBD | PENDING |

## Methodology

Per BENCH_MATRIX_SPEC.md:
- Release-perf profile (LTO=thin, opt-level=3)
- Identical workloads run on FrankenPandas and pandas 2.2.3
- CV > 5% measurements dropped
- p50/p95/p99 captured per workload
- Geometric mean per category

## Verdicts

- **FASTER**: FP is >1.05x faster than pandas
- **PARITY**: FP is 0.95x-1.05x (equivalent)
- **SLOWER**: FP is <0.95x (pandas wins)

## Regenerate

```bash
# Run benchmarks
python benches/vs_pandas_harness.py --all --sizes 10k,100k --output artifacts/bench/latest.json

# Generate scorecard
python scripts/gen_perf_scorecard.py --input artifacts/bench/latest.json --format md --output artifacts/perf/SCORECARD.md

# Apply ratchet gate
./scripts/apply_ratchet.sh
```

## Thresholds (Ratchet Gate)

| Metric | Regression Threshold |
|--------|---------------------|
| Primary (single p50) | -3% |
| Category geomean | -5% |
| Per-category weighted | -10% |
| p90 tail | -15% |
| Throughput | -5% |
