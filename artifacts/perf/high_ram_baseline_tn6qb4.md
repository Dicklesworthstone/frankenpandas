# High-RAM Baseline Telemetry - br-frankenpandas-tn6qb.4

Profile: `smoke-local-2026-05-08` on Linux x86_64 with 64 available workers.

FrankenPandas command:

```bash
/data/tmp/frankenpandas-whiteotter-tn6qb4-build2/debug/high_ram_perf_baseline --profile smoke-local-2026-05-08 --rows 5000 --iters 10 --warmup 2 --frame-cols 5 --key-cardinality 256
```

Pandas companion: pandas `2.2.3`, Python `3.13.7`, same rows/iters/warmup/profile shape using in-process `time.perf_counter_ns`.

| Workload | FP p95 ms | Pandas p95 ms | FP rows/s | Pandas rows/s |
|---|---:|---:|---:|---:|
| filter_boolean_mask | 7.999 | 0.597 | 742468 | 9924770 |
| dataframe_groupby_sum | 6.240 | 1.151 | 972676 | 5636159 |
| dataframe_inner_join | 55.487 | 8.261 | 212856 | 1577519 |
| series_add_outer_alignment | 48.474 | 0.317 | 217520 | 42091014 |
| csv_write_read_roundtrip | 24.019 | 15.819 | 209520 | 346542 |

Delta artifact contract:
- Compare by `workloads[].name`.
- Carry `p50_ms`, `p95_ms`, `p99_ms`, `throughput_rows_per_sec`, input/output byte estimates, RSS high-water, and checksum.
- Flag same-profile p95 drift over 10%; block drift over 20% unless the run is on a non-comparable host.
- Keep pandas oracle rows explicit. If pandas is unavailable, record that as `pandas_oracle.status = unavailable` instead of inferring parity.

High-RAM follow-up command:

```bash
high_ram_perf_baseline --profile high-ram --rows 100000 --iters 30 --warmup 5 --frame-cols 5 --key-cardinality 1024
```

Notes:
- The CLI is measurement-only and does not apply optimization levers.
- RSS uses process high-water marks; run each workload in a separate process if isolated peak RSS attribution is needed.
- Checksums matched pandas for all comparable smoke workloads.
