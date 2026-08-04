# Lane M 1M/10M numeric merge incumbent resurrection

Date: 2026-07-27 America/New_York (2026-07-28 UTC)

Bead: `br-frankenpandas-v3y7n`

Result class: `incumbent-win` for all six rows.

This was a measurement-only re-adjudication of shipping merge code. It was not
an fp-before/fp-after optimization and is not a maintenance self-speedup.

## Workload

Both engines merge two frames with identical schemas and values:

- left key: unique Int64 `0..n`
- right key: unique even Int64 `0,2,..,2(n-1)`
- left payload: Float64 `0..n`
- right payload: Float64 `10 * (0..n)`
- merge variants: `inner`, `left`, and `outer`
- sizes: `n=1,000,000` and `n=10,000,000`

Setup is outside the timed region. The timed boundary is the public merge call
that constructs the result.

## Contract

- Worker: `ovh-a`
- Command:

  ```text
  RCH_WORKER=ovh-a RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category joins --sizes 1M,10M --dtypes float64 --workloads join_inner,join_left,join_outer --output artifacts/bench/proud_lane_m_merge_1m_10m_20260727.json --json-stdout
  ```

- Shared invocation:
  `vs-pandas-20260728T035631.671745Z-pid2122218`
- FrankenPandas executing ELF, self-reported by the process:
  `885f386e10f4440e961b2672543c8fe735eadccf055748fc2e63dd48da979349`
  (70,208,952 bytes)
- Live incumbent: pandas 2.2.3
- pandas installed-distribution content-tree SHA-256:
  `fb69f90acac18b871bb69f5eab56bea198b17692c5045de29eed608132a959c9`
  (70,709,779 bytes, 2,922 files)
- Python host executable SHA-256:
  `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`
- Every engine/workload/size row ran 25 alternating A/A pairs in the same
  invocation.
- Decision gate: twice the combined A/A bootstrap-median 95% CI log
  half-width. CV is provenance only and had no vote.
- Raw JSON SHA-256:
  `cac48c70982fa9c16a6a04e7645abd1753e8a7ef3a82dfaf1325a665385243d3`

## Results

| workload | size | FP p50 | pandas p50 | ratio | FP A/A median CI | pandas A/A median CI | claim log effect | required log effect | verdict |
|---|---:|---:|---:|---:|---|---|---:|---:|---|
| `join_inner` | 1M | 2.618 ms | 19.895 ms | **7.599x** | [0.991618, 1.012245] | [0.999618, 1.002138] | 2.02803246 | 0.02434037 | **FASTER** |
| `join_inner` | 10M | 37.232 ms | 133.635 ms | **3.589x** | [0.989816, 1.001965] | [0.992164, 1.001922] | 1.27794804 | 0.02047241 | **FASTER** |
| `join_left` | 1M | 5.338 ms | 12.708 ms | **2.381x** | [0.994356, 1.000509] | [0.991565, 1.026109] | 0.86746426 | 0.05154784 | **FASTER** |
| `join_left` | 10M | 70.317 ms | 108.995 ms | **1.550x** | [0.993308, 0.999978] | [0.987770, 1.012954] | 0.43828016 | 0.02574092 | **FASTER** |
| `join_outer` | 1M | 9.485 ms | 39.273 ms | **4.141x** | [0.963643, 1.013766] | [0.979382, 1.026793] | 1.42085968 | 0.07406973 | **FASTER** |
| `join_outer` | 10M | 111.388 ms | 696.546 ms | **6.253x** | [0.974273, 1.013474] | [0.909849, 1.070320] | 1.83311235 | 0.18895384 | **FASTER** |

The six-row geomean is **3.710x**. Inner and left narrow from 1M to 10M,
while outer widens. This is a strong current incumbent result, but the mixed
scaling does not establish one family-wide interpreted-overhead mechanism.

The 10M rows have broad raw tails (CV 151.76%, 152.23%, and 98.73% on FP;
61.77% on pandas outer). CV is recorded only as provenance. The paired A/A
median intervals remain bounded, and every claim effect exceeds its
predeclared median-CI threshold by a wide margin; the narrowest relative
clearance is still 9.7x (`join_outer@10M`).

## Retry predicates

- `join_inner`: reopen the competitive number only if source, result
  materialization boundary, pandas artifact, or worker ISA changes. Preserve
  the same exact unique-key values and full invocation contract.
- `join_left`: the smallest admitted row is 1.550x at 10M. Do not pursue a new
  kernel lever unless a current profile attributes more than 5% self-time to a
  named frame and its computed Amdahl ceiling exceeds 5%.
- `join_outer`: the 10M median claim is decisive, but p95/p99 attribution is
  not. Re-run only to make a tail-latency claim, using fresh child processes
  plus peak RSS and major-fault counters while preserving the same worker and
  binary identities.

Raw evidence:
`artifacts/bench/proud_lane_m_merge_1m_10m_20260727.json`.
