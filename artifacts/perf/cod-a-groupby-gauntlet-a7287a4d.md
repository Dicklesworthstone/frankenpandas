# Cod-a GroupBy Gauntlet Verification - a7287a4d

- Date: 2026-06-19 UTC
- Agent: GrayStone / cod-a
- Commit under verification: `a7287a4d`
- Bead: `br-frankenpandas-2qb1i`
- Cluster covered: recent cod-a clone-free `fp-groupby` generic aggregation work, with direct focus on the latest Float64 sum/prod counter fast path.
- Original comparator: pandas 2.2.3, numpy 2.4.3, via `benches/vs_pandas_harness.py`.

## Commands

```bash
CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a \
  rch exec -- cargo build --profile release-perf -p fp-bench

CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a \
  cargo build --profile release-perf -p fp-bench

CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a \
  python3 benches/vs_pandas_harness.py \
    --category groupby --sizes 100k --dtypes float64 \
    --output artifacts/perf/cod-a-groupby-gauntlet-vs-pandas-a7287a4d.json

CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a \
  python3 benches/vs_pandas_harness.py \
    --category groupby --sizes 1M --dtypes float64 \
    --output artifacts/perf/cod-a-groupby-gauntlet-vs-pandas-a7287a4d-1m.json

CARGO_TARGET_DIR=/data/projects/.rch-targets/frankenpandas-cod-a \
  rch exec -- cargo bench -p fp-conformance --bench vs_pandas -- groupby/
```

The RCH `fp-bench` build completed on worker `ovh-b`, but the configured artifact sync did not materialize the local `release-perf/fp-bench` executable. The pandas harness therefore used the same per-crate release-perf build locally in the same `CARGO_TARGET_DIR`. The Criterion groupby slice completed on `ovh-b`; full stdout is in `artifacts/perf/cod-a-groupby-gauntlet-criterion-a7287a4d.txt`.

## Head-To-Head Results

Accepted ratios are pandas p50 divided by FrankenPandas p50. `DROPPED_HIGH_CV` rows are recorded with p50-implied ratios for audit only and are not accepted as keep proof.

### 100k Rows

| Workload | Verdict | Accepted ratio | p50-implied ratio | FP p50 us | pandas p50 us | FP CV% | pandas CV% |
|---|---:|---:|---:|---:|---:|---:|---:|
| `groupby_sum_int64` | DROPPED_HIGH_CV | N/A | 8.095x | 199.10 | 1611.79 | 35.81 | 20.60 |
| `groupby_mean_float64` | DROPPED_HIGH_CV | N/A | 3.721x | 264.13 | 982.78 | 2.60 | 13.06 |
| `groupby_agg_multi` | DROPPED_HIGH_CV | N/A | 7.949x | 304.53 | 2420.78 | 33.79 | 19.10 |
| `groupby_mean_str` | DROPPED_HIGH_CV | N/A | 2.008x | 1860.89 | 3736.72 | 8.69 | 4.91 |
| `groupby_transform_mean` | FASTER | 4.586x | 4.586x | 234.31 | 1074.52 | 4.09 | 4.98 |
| `groupby_transform_mean_str` | DROPPED_HIGH_CV | N/A | 2.018x | 1760.68 | 3553.61 | 13.09 | 11.40 |
| `groupby_cumcount` | DROPPED_HIGH_CV | N/A | 1.952x | 793.48 | 1548.59 | 0.97 | 13.71 |
| `groupby_count` | FASTER | 3.155x | 3.155x | 197.16 | 622.01 | 2.06 | 2.22 |

Summary: 2 accepted workloads, 6 high-CV drops, accepted groupby geomean 3.804x. Harness `claim_validated=false` because this was a groupby-only pass and most rows were rejected for CV.

### 1M Rows

| Workload | Verdict | Accepted ratio | p50-implied ratio | FP p50 us | pandas p50 us | FP CV% | pandas CV% |
|---|---:|---:|---:|---:|---:|---:|---:|
| `groupby_sum_int64` | DROPPED_HIGH_CV | N/A | 4.752x | 2612.67 | 12415.94 | 15.47 | 1.41 |
| `groupby_mean_float64` | DROPPED_HIGH_CV | N/A | 4.866x | 2748.41 | 13373.26 | 20.40 | 1.14 |
| `groupby_agg_multi` | FASTER | 6.291x | 6.291x | 3212.90 | 20211.93 | 1.34 | 1.01 |
| `groupby_mean_str` | DROPPED_HIGH_CV | N/A | 2.291x | 18609.44 | 42642.00 | 15.93 | 7.76 |
| `groupby_transform_mean` | DROPPED_HIGH_CV | N/A | 6.140x | 2889.73 | 17742.52 | 5.08 | 13.40 |
| `groupby_transform_mean_str` | FASTER | 2.178x | 2.178x | 20061.76 | 43688.75 | 1.31 | 1.34 |
| `groupby_cumcount` | DROPPED_HIGH_CV | N/A | 4.957x | 10804.23 | 53559.56 | 7.91 | 9.48 |
| `groupby_count` | FASTER | 5.988x | 5.988x | 1993.46 | 11937.48 | 2.61 | 2.13 |

Summary: 3 accepted workloads, 5 high-CV drops, accepted groupby geomean 4.345x. Harness `claim_validated=false` because this was a groupby-only pass.

## Criterion Guard

`rch exec -- cargo bench -p fp-conformance --bench vs_pandas -- groupby/` completed successfully. This benchmark is Rust-side Criterion despite the file name; it is a stability guard, not pandas-vs-Rust proof.

Key Criterion means:

| Benchmark | Mean |
|---|---:|
| `groupby/sum_int64/rows/10000` | 47.682 us |
| `groupby/sum_int64/rows/100000` | 442.41 us |
| `groupby/mean_float64/rows/10000` | 61.587 us |
| `groupby/mean_float64/rows/100000` | 499.50 us |
| `groupby/agg_multi/rows/10000` | 158.52 us |
| `groupby/agg_multi/rows/100000` | 1.7144 ms |
| `groupby/ngroup/rows/10000` | 383.23 us |
| `groupby/ngroup/rows/100000` | 3.4304 ms |

## Decision

Keep the current groupby optimization cluster. There are no accepted neutral or slower head-to-head measurements in this pass. High-CV rows are evidence gaps and must be rerun under a more stable harness before being used as proof, but their p50 direction does not justify a revert.

No code revert was made. The next gauntlet pass should either stabilize the high-CV groupby rows with longer sampling/pinned worker load or move to the next pending cod-a backlog bead with the same ledger discipline.
