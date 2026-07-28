# Lane M split-and-explode incumbent admission

Date: 2026-07-27 America/New_York (2026-07-28 UTC)

Bead: `br-frankenpandas-aba4b`

Result class: `incumbent-win` for all four measured rows. The conservative
headline uses the fastest pandas storage backend measured at 1M.

## Workload

Each engine starts with `n` strings of the form
`a{i % 97},b{i % 89},c{i % 83}` and produces three ordered values per input
row while repeating the source index:

- FrankenPandas: `Series::explode(",")`
- pandas: `Series.str.split(",").explode()`
- pandas 1M storage screen: `object`, `string[python]`, and
  `string[pyarrow]`
- 10M follow-up: `string[python]`, the fastest pandas arm in the 1M screen

Series and frame construction are outside the timed region. The timed public
operation materializes all `3n` output values and their repeated index.
A 1,000-row semantic probe produced identical 3,000 values and repeated
indexes across the three pandas storage backends. Existing
`series_explode_delimited_il7st`, `series_explode_basic`,
`series_explode_preserves_index`, and explode golden tests cover the
FrankenPandas output contract; this change does not alter production explode
code.

## Contract

- Worker: `vmi1264463`
- 1M command:

  ```text
  RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category dataframe_ops --sizes 1M --dtypes float64 --workloads df_explode,df_explode_string_python,df_explode_string_arrow --output artifacts/bench/proud_lane_m_explode_backends_1m_20260727.json --json-stdout
  ```

- 10M command:

  ```text
  RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category dataframe_ops --sizes 10M --dtypes float64 --workloads df_explode_string_python --output artifacts/bench/proud_lane_m_explode_string_python_10m_20260727.json --json-stdout
  ```

- Shared invocation within every measured row:
  `vs-pandas-20260728T042541.777759Z-pid1487265` at 1M and
  `vs-pandas-20260728T044019.007957Z-pid1509753` at 10M.
- Harness source SHA-256 in both invocations:
  `b58aabad2ada84132e40f8f504f9f7cca21a9482dcaae736a9b9017f07c3dd6a`
  (58,168 bytes).
- FrankenPandas executing ELFs, self-reported by the processes:
  `1004f1a94d113ce7491d598b96948c19bcfeb7a54d6b3c050456f2e37b4d60d6`
  (70,293,280 bytes) at 1M and
  `d8ed875bd0d9b1e057bb81dc09fc532d1a9cd91efefd218ce5b9f5952c0ed59d`
  (70,293,304 bytes) at 10M.
- Live incumbent: pandas 2.2.3.
- pandas installed-distribution content-tree SHA-256:
  `051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb`
  (70,709,779 bytes, 2,922 files).
- Arrow backend: pyarrow 24.0.0, installed-distribution content-tree
  SHA-256
  `2e701e78b2e69a481b6e901b584db29c4151221f59568dcb7cde7f036bca5f17`
  (157,865,460 bytes, 861 files).
- Python executable SHA-256:
  `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`.
- Every engine/workload/size row ran 25 alternating A/A pairs in the same
  invocation as its A/B comparison.
- Decision gate: twice the combined A/A bootstrap-median 95% CI log
  half-width. CV is provenance only and had no vote.
- Raw JSON SHA-256:
  `3ebd597232ddfb653b8898f263d4500d96ac9e57a256dad6c3135d13fb3ba735`
  (1M) and
  `edc3865261bfb684711f5db52cf8efc2ec421ae01e98c870821cfa8aed3c190f`
  (10M).

## Results

| pandas storage | size | FP p50 | pandas p50 | ratio | FP A/A median CI | pandas A/A median CI | claim log effect | required log effect | verdict |
|---|---:|---:|---:|---:|---|---|---:|---:|---|
| `object` | 1M | 134.263 ms | 1,329.562 ms | **9.903x** | [0.963659, 1.089874] | [0.919331, 1.081146] | 2.29280112 | 0.17212484 | **FASTER** |
| `string[python]` | 1M | 129.557 ms | 1,251.063 ms | **9.656x** | [0.955614, 0.999670] | [0.953308, 1.035103] | 2.26762913 | 0.09563501 | **FASTER** |
| `string[pyarrow]` | 1M | 134.662 ms | 1,316.825 ms | **9.779x** | [0.929549, 1.058178] | [0.900276, 0.980604] | 2.28020920 | 0.21010809 | **FASTER** |
| `string[python]` | 10M | 1,600.738 ms | 15,889.945 ms | **9.927x** | [0.930059, 1.123736] | [0.876962, 1.162040] | 2.29522199 | 0.30035482 | **FASTER** |

`string[python]` is the strongest incumbent at 1M: its pandas p50 is 5.0%
below Arrow and 5.9% below object. Against that arm, the admitted ratio rises
from 9.656x at 1M to 9.927x at 10M (+2.8%). The absolute median-time advantage
grows from 1.122 seconds to 14.289 seconds. Pandas time scales 12.70x across
the 10x input increase while FrankenPandas scales 12.36x.

The 10M FP CV is 60.74%, but the paired median-CI decision remains decisive:
effect 2.29522199 versus required threshold 0.30035482. This supports a
median incumbent claim only; it does not support a p95 or p99 claim.

## Retry predicates

- The median incumbent rows stand until the explode implementation, timed
  boundary, harness source, pandas/pyarrow artifact, worker ISA, or allocator
  changes. Any replacement claim must repeat the live-incumbent,
  same-invocation ELF/A/A/median-CI contract.
- Re-screen the pandas storage winner when pandas or pyarrow changes version
  or adds a materially different string backend. Carry only the fastest
  semantically identical arm to 10M.
- Re-run 10M only to claim tail latency, using fresh child processes and
  recording peak RSS plus major faults while preserving the same identities.
- Do not reopen an fp-side explode lever from this measurement alone. Require
  a current profile naming a target frame with more than 5% self-time and a
  computed Amdahl ceiling above 5%; the ledger's prior contiguous-Utf8 and
  typed-index work remains closed otherwise.

Raw evidence:
`artifacts/bench/proud_lane_m_explode_backends_1m_20260727.json` and
`artifacts/bench/proud_lane_m_explode_string_python_10m_20260727.json`.
