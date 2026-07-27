## GroupBy Matrix Coverage — frankenpandas vs pandas 2.2.3

Generated: 2026-07-26 · Bead: `br-frankenpandas-uza04.218`

**Scope: the five-workload GroupBy matrix only.** No whole-suite weighted score is
computed here, and no other scorecard category is measured by this run.

### Measured ratios (higher = frankenpandas faster)

Harness: `benches/vs_pandas_harness.py`, which runs frankenpandas and pandas 2.2.3
side by side in the same invocation. Binary: `release-perf` `fp-bench`, rebuilt at
HEAD and freshness-checked. Pinned to `taskset -c 48-63`.

| Workload | 10k | 100k |
|---|---:|---:|
| `groupby_multi_str` | 4.136× | 2.828× |
| `groupby_agg3_str` | 5.517× | 2.998× |
| `df_groupby_str_sum` | 2.524× | 3.178× |
| `df_groupby_2key_sum` | 1.209× | 1.505× |
| `df_groupby_2strkey_sum` | 0.856× | 1.402× |

`groupby_agg3_str @100k` and `df_groupby_2strkey_sum @10k` are the medians of four
runs each; per-run values agree within 3% (2.81–3.03× and 0.837–0.872×
respectively). Every attempt, admitted or not, is recorded in the retry artifact.

### What these numbers are

Coverage ratios against a live incumbent arm. They are **not** median-CI-gated
verdicts: this run carries no A/A null control, so a ratio close to 1.0 is not
separable from harness noise. The two ratios furthest from 1.0 (`groupby_agg3_str`
and `df_groupby_2strkey_sum`) are the least sensitive to that limitation.

Promote a row here to a decided verdict by re-measuring with an executing-ELF
identity, a same-invocation A/A null control, and the median-CI gate.

### Artifacts

- `artifacts/bench/cod_fp_uza04_218_groupby_matrix_remaining_10k_100k.json`
- `artifacts/bench/cod_fp_uza04_218_groupby_matrix_retry_10k_100k.json`
