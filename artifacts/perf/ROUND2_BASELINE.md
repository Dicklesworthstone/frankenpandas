# Round 2 Baseline

Target hotspot:

- `fp-index::align_union` map construction and membership checks used by `Series::add`, `groupby_sum`, and join-adjacent alignment flows.

Benchmark command:

- `/data/projects/frankenpandas/.target-opt/release/groupby-bench --rows 100000 --key-cardinality 512 --iters 30`

Pre-change run (captured before lever):

- mean: `1.400 s`
- stddev: `0.051 s`
- range: `1.335 s .. 1.500 s`
- runs: `12`

Post-change run (sample-exported):

- source artifact: `artifacts/perf/round2_groupby_hyperfine_after.json`
- mean: `1.138 s`
- stddev: `0.033 s`
- p50: `1.129 s`
- p95: `1.191 s`
- p99: `1.206 s`
- range: `1.082 s .. 1.210 s`
- runs: `12`

Observed delta:

- mean improvement: `18.7%` faster (`1.400 s -> 1.138 s`)

Supporting profile signal:

- syscall profile artifact: `artifacts/perf/round2_groupby_strace_after.txt`
- interpretation: syscall overhead is negligible versus total runtime, confirming this benchmark is dominated by in-process compute/allocation behavior rather than OS I/O.

