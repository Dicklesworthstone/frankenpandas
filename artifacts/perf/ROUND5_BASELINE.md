# ROUND5 Baseline

Command:

```bash
.target-opt/release/groupby-bench --rows 100000 --key-cardinality 512 --iters 30
```

Baseline comparator (round4 after, reused as round5 before):
- mean: `0.290649768 s`
- p50: `0.289520563 s`
- p95: `0.299307571 s`
- p99: `0.300423827 s`

Post-lever (`round5_groupby_hyperfine_after.json`):
- mean: `0.037208347 s`
- p50: `0.036986944 s`
- p95: `0.041281972 s`
- p99: `0.041480689 s`

Delta:
- mean latency improvement vs baseline comparator: `87.20%`
