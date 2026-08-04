# Lane M large-N rolling/expanding incumbent resurrection

Date: 2026-07-27 America/New_York (2026-07-28 UTC)

Bead: `br-frankenpandas-v5h3g`

Result class: `incumbent-win` for seven rows; `NULL_UNDECIDABLE` for
`ewm_mean@10M`.

This was a measurement-only resurrection of existing shipping code. It was not
an fp-before/fp-after optimization and therefore is not a maintenance
self-speedup.

## Contract

- Worker: `ovh-a`
- Command:

  ```text
  RCH_WORKER=ovh-a RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo run --locked --profile release-perf -p fp-bench -- --remote-python-harness --category rolling --sizes 1M,10M --dtypes float64 --workloads rolling_mean_w10,rolling_std_w50,expanding_sum,ewm_mean --output artifacts/bench/proud_lane_m_rolling_expanding_1m_10m_20260727.json --json-stdout
  ```

- Shared invocation:
  `vs-pandas-20260728T034723.542175Z-pid2087469`
- FrankenPandas executing ELF, self-reported by the process:
  `dfe3fd9cb3badf5e3889d33d9c587c68570798869fe71d0bcbf40e4b6877c34f`
  (70,209,304 bytes)
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
  `391365d0a53224479a70dcb8dfa687ff447ef2bae3219dd0aece19060f9fb0be`

## Results

| workload | size | FP p50 | pandas p50 | ratio | FP A/A median CI | pandas A/A median CI | claim log effect | required log effect | verdict |
|---|---:|---:|---:|---:|---|---|---:|---:|---|
| `rolling_mean_w10` | 1M | 6.641 ms | 10.791 ms | **1.625x** | [0.999367, 1.001059] | [0.993865, 1.005599] | 0.48549084 | 0.01230734 | **FASTER** |
| `rolling_mean_w10` | 10M | 67.482 ms | 111.184 ms | **1.648x** | [0.999936, 1.000401] | [0.988181, 1.007534] | 0.49932384 | 0.02377846 | **FASTER** |
| `rolling_std_w50` | 1M | 13.223 ms | 18.097 ms | **1.369x** | [0.997875, 1.002812] | [0.993457, 1.030774] | 0.31377669 | 0.06062042 | **FASTER** |
| `rolling_std_w50` | 10M | 132.314 ms | 176.701 ms | **1.335x** | [0.999622, 1.000371] | [0.987496, 1.007966] | 0.28927991 | 0.02516584 | **FASTER** |
| `expanding_sum` | 1M | 3.036 ms | 9.422 ms | **3.103x** | [0.996737, 1.002322] | [0.990040, 1.009378] | 1.13244814 | 0.02001893 | **FASTER** |
| `expanding_sum` | 10M | 30.731 ms | 77.891 ms | **2.535x** | [0.998532, 1.002498] | [0.997150, 1.004560] | 0.93002714 | 0.00909850 | **FASTER** |
| `ewm_mean` | 1M | 6.657 ms | 7.947 ms | **1.194x** | [0.999943, 1.002920] | [0.983939, 1.015565] | 0.17706398 | 0.03238203 | **FASTER** |
| `ewm_mean` | 10M | 66.811 ms | 67.128 ms | 1.005x | [0.998581, 1.000726] | [0.987656, 1.003537] | 0.00472877 | 0.02484177 | `NULL_UNDECIDABLE` |

The seven decidable rows have a 1.728x geomean. Rolling mean and rolling std
hold nearly constant ratios from 1M to 10M. Expanding sum remains a larger win,
but its ratio narrows at 10M. EWM converges to parity at 10M. This family
therefore establishes current large-N incumbent wins, but it does not reproduce
GroupBy's gap-growing interpreted-overhead signature.

## Retry predicates

- The seven admitted rows stand until the corresponding kernel, benchmark
  boundary, pandas artifact, or worker ISA changes. Any replacement claim must
  again run the live incumbent side-by-side in one invocation and retain the
  in-process ELF SHA, pandas artifact SHA, A/A controls, and median-CI gate.
- Do not quote `ewm_mean@10M` as faster. Reopen that row only if either:
  (a) fresh-child or counter-based isolation reduces the combined required log
  effect to at most 0.0040 with the same identities, or (b) a current profile
  names an EWM frame with more than 5% self-time and a counted mechanism can
  remove at least 2% of whole-workload cycles. The rejected branch/loop-shape
  micro-optimization remains closed.

Raw evidence:
`artifacts/bench/proud_lane_m_rolling_expanding_1m_10m_20260727.json`.
