# Lane M string-backend incumbent admission

Date: 2026-07-28 America/New_York

Bead: `br-frankenpandas-ltmk9`

Result class: `incumbent-win` for all nine measured rows. Competitive
headlines use pandas `string[pyarrow]`, the fastest pandas backend in every
1M screen row. The object rows are secondary diagnostics only.

## Workloads and semantic proof

All fixtures contain 1,000 repeating keys (`g0000` through `g0999`), unique
ordered names (`item_0000000000`, ...), and exact Float64 values `0..n-1`.
Frame construction is outside the timed region.

- `str_sort`: sort the full three-column frame by unique `name`.
- `str_value_counts`: count the repeating `key` Series.
- `str_groupby_sum`: group by `key` and sum only `val`.

The FP arm is unchanged for backend aliases: `_object` and `_arrow` both
route to the existing contiguous-Utf8 workload. A 1,000-row pandas probe
proved object and Arrow produce identical values and index ordering for all
three operations. Existing FP guards cover stable Utf8 sorting,
nullable-Utf8 value-count equivalence, and Utf8-key groupby sum. Production
code did not change.

Behavior-preservation checklist:

- Ordering preserved: yes; backend aliases change only pandas storage.
- Tie-breaking unchanged: yes; sort names are unique, and the 1,000-key
  count/groupby probe produced identical ordered indexes.
- Floating point: identical; the same exactly representable input values are
  summed by both pandas storage arms.
- RNG seeds: N/A.
- Production golden outputs: unchanged; this is harness-only routing.

## Contract

- Worker: `vmi1264463`.
- Disk guard before each Cargo invocation: 316 GiB and 309 GiB free,
  respectively, both above the 120 GiB floor.
- Strict remote commands:

  ```text
  RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category strings --sizes 1M --dtypes float64 --workloads str_sort_object,str_sort_arrow,str_value_counts_object,str_value_counts_arrow,str_groupby_sum_object,str_groupby_sum_arrow --output artifacts/bench/proud_lane_m_string_backends_1m_20260728.json --json-stdout
  ```

  ```text
  RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category strings --sizes 10M --dtypes float64 --workloads str_sort_arrow,str_value_counts_arrow,str_groupby_sum_arrow --output artifacts/bench/proud_lane_m_string_arrow_10m_20260728.json --json-stdout
  ```

- Invocation IDs:
  `vs-pandas-20260728T063309.699807Z-pid1758322` (1M) and
  `vs-pandas-20260728T064511.318611Z-pid1785126` (10M).
- Harness source SHA-256 in both invocations:
  `55c7f737d6d18b460b566338e6f8859dcc9dcc51ff5b0c91b1aa3373c4b8de47`
  (62,017 bytes).
- Self-reported FP ELF SHA-256:
  `4ac48e4c83e07a1750c64e8d3d48aa0f9ce43ebe2e4fda298b4dd77df9f2dd2d`
  (70,294,216 bytes) at 1M and
  `86614d5dd206fa0573677787907006e5b21e06b25db5703725e586c68b2e8ba2`
  (70,294,368 bytes) at 10M.
- Python executable SHA-256:
  `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`.
- pandas 2.2.3 content-tree SHA-256:
  `051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb`.
- pyarrow 24.0.0 content-tree SHA-256:
  `2e701e78b2e69a481b6e901b584db29c4151221f59568dcb7cde7f036bca5f17`.
- Every row used 25 alternating A/A pairs per engine in the same invocation
  as its A/B comparison.
- Decision gate: twice the larger FP/pandas bootstrap-median 95% CI log
  half-width. CV is provenance only and had no vote.
- Raw JSON SHA-256:
  `079a8aa05c9e00442c88b7e8929638deb75a77cbfd3c153e02548b89c9b736bf`
  (1M) and
  `431a097cd70dbf5fce9054529913a9e9c4c9d2580aadb46d5ef49d778da450ec`
  (10M).

## Results

| pandas backend | workload | size | FP p50 | pandas p50 | ratio | FP A/A median CI | pandas A/A median CI | effect / required | verdict |
|---|---|---:|---:|---:|---:|---|---|---:|---|
| object | `str_sort` | 1M | 86.492 ms | 660.871 ms | **7.641x** | [0.897760, 1.144962] | [0.975916, 1.019528] | 2.03351189 / 0.27074301 | **FASTER** |
| `string[pyarrow]` | `str_sort` | 1M | 87.174 ms | 154.807 ms | **1.776x** | [0.781551, 1.190955] | [0.954701, 1.086849] | 0.57427493 / 0.49294904 | **FASTER** |
| object | `str_value_counts` | 1M | 22.301 ms | 98.563 ms | **4.420x** | [0.953335, 1.034170] | [0.933980, 1.044445] | 1.48606678 / 0.13660123 | **FASTER** |
| `string[pyarrow]` | `str_value_counts` | 1M | 22.359 ms | 33.124 ms | **1.481x** | [0.962780, 1.068560] | [0.976184, 1.051414] | 0.39305010 / 0.13262302 | **FASTER** |
| object | `str_groupby_sum` | 1M | 14.918 ms | 61.871 ms | **4.147x** | [0.958645, 1.036576] | [0.974951, 1.041829] | 1.42248959 / 0.08446952 | **FASTER** |
| `string[pyarrow]` | `str_groupby_sum` | 1M | 14.191 ms | 45.405 ms | **3.200x** | [0.985028, 1.117088] | [0.978088, 1.063541] | 1.16301003 / 0.22145032 | **FASTER** |
| `string[pyarrow]` | `str_sort` | 10M | 656.105 ms | 1,657.029 ms | **2.526x** | [0.946730, 1.037833] | [0.988864, 1.023574] | 0.92646044 / 0.10948346 | **FASTER** |
| `string[pyarrow]` | `str_value_counts` | 10M | 218.066 ms | 318.024 ms | **1.458x** | [0.952319, 1.014717] | [0.943298, 1.019154] | 0.37732781 / 0.11674512 | **FASTER** |
| `string[pyarrow]` | `str_groupby_sum` | 10M | 145.070 ms | 432.653 ms | **2.982x** | [0.945491, 1.038132] | [0.950300, 1.032075] | 1.09272211 / 0.11210124 | **FASTER** |

The strongest-incumbent geomean across the six Arrow rows is **2.126x**.
String sort is the scale-amplified result: its ratio grows from 1.776x to
2.526x (+42.2%), while its absolute median advantage grows from 67.6 ms to
1.001 seconds. Value-counts and groupby-sum remain decisive but their ratios
are approximately scale-stable (1.481x to 1.458x and 3.200x to 2.982x);
their absolute advantages still grow to 100.0 ms and 287.6 ms.

## Retry predicates

- These incumbent rows stand until the workload boundary, fixture, FP string
  implementation, harness source, pandas/pyarrow artifact, allocator, or
  worker ISA changes.
- Re-screen all available pandas string backends after a pandas or pyarrow
  version change. Carry only the fastest semantically identical backend to
  the large-N follow-up.
- Do not quote object-only ratios as competitive claims. Object remains a
  diagnostic arm; `string[pyarrow]` is the admitted incumbent here.
- Re-run `str_sort` only for a tail claim with fresh child processes and peak
  RSS/major-fault counters. The current evidence admits medians, not p95/p99.
- Do not infer an FP source optimization from this comparison. Any FP lever
  still requires a fresh profile naming a non-zero-self frame and a computed
  Amdahl ceiling.

Raw evidence:
`artifacts/bench/proud_lane_m_string_backends_1m_20260728.json` and
`artifacts/bench/proud_lane_m_string_arrow_10m_20260728.json`.
