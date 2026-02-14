# Round 3 Baseline

Target workload:

- `fp-groupby::groupby_sum` on aligned, duplicate-free indexes (common case in scoped packet fixtures).

Benchmark command:

- `/data/projects/frankenpandas/.target-opt/release/groupby-bench --rows 100000 --key-cardinality 512 --iters 30`

Pre-change baseline:

- source: `artifacts/perf/round3_groupby_hyperfine_before.json`
- mean: `0.899 s`
- stddev: `0.022 s`
- p50: `0.896 s`
- p95: `0.933 s`
- p99: `0.938 s`
- range: `0.864 s .. 0.939 s`

Post-change re-baseline:

- source: `artifacts/perf/round3_groupby_hyperfine_after.json`
- mean: `0.291 s`
- stddev: `0.004 s`
- p50: `0.290 s`
- p95: `0.297 s`
- p99: `0.297 s`
- range: `0.287 s .. 0.297 s`

Observed delta:

- mean improvement: `67.6%` faster (`0.899 s -> 0.291 s`)

Profile artifacts:

- flamegraph (before): `artifacts/perf/round3_groupby_flamegraph_before.svg`
- flamegraph (after): `artifacts/perf/round3_groupby_flamegraph_after.svg`
- syscall profile (before): `artifacts/perf/round3_groupby_strace_before.txt`
- syscall profile (after): `artifacts/perf/round3_groupby_strace_after.txt`

Interpretation:

- Prior path spent substantial time in alignment/reindex + hash-heavy index-map work.
- Fast-path elimination of unnecessary alignment on already-equal, duplicate-free indexes removes that dominant cost center.

