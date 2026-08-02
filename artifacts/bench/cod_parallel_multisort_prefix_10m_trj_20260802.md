# Parallel primary-prefix multi-sort: 10M rows on trj

Decision: **KEEP**

Commit `47ad0a4d7832c1a8b38df2661a9581c833ff55e4` replaces the serial
multi-key radix ordering residual with a stable high-16-bit primary-key
partition followed by shared-nothing completion of disjoint prefix buckets.
Workers own both permutation and scratch slices, so the parallel section uses
no locks, atomics, or merge pass.

## Whole-job result

Workload: `dataframe_ops/sort_values_multi/10M/float64`, ascending stable sort
on `col_0`, `col_1`, and `col_2` over ten all-valid Float64 columns.

| Arm | p50 | p95 | p99 | CV | Operation threads |
|---|---:|---:|---:|---:|---:|
| exact parallel-prefix candidate | 341.859 ms | 355.690 ms | 359.147 ms | 2.07% | 64 |
| immutable serial FrankenPandas | 3,083.411 ms | 3,198.124 ms | 3,223.696 ms | 1.60% | 1 |
| live pandas 2.2.3 | 18,578.385 ms | 18,749.214 ms | 18,778.317 ms | 0.47% | 1 |

- Reference / candidate: **9.019531x**, bootstrap median-ratio 95% CI
  **[8.918147, 9.098719]**.
- pandas / candidate: **54.345x**, bootstrap median-ratio 95% CI
  **[53.749941, 54.838639]**.
- Candidate, reference, and pandas paired-null medians were respectively
  `0.990625`, `1.000829`, and `0.998510`; every corrected gate clause passed.
- Candidate and reference liveness checksums match: `4957dea0fe3e2ed1`.

The canonical all-online-CPU quiescence preflight failed closed on both
`thinkstation1` and `threadripperje` because unrelated long-running jobs were
active. This is therefore a **directional same-driver result**, not a canonical
quiet-host claim. Candidate, reference, and live pandas still ran in one
invocation on the same `0-63` physical-core cpuset with paired A/A controls.

## Profile-guided lever and residuals

The exact pre-change 1M profile ranked `radix_argsort_multi_u64` at 95.12% of
self-time with one operation thread. Its byte-pass dependencies are ordered,
but the completed primary prefix establishes final independent ranges, so the
largest residual was not inherently sequential; this change parallelizes it.

The candidate profile moved 88.86% of samples into the scoped bucket workers.
Within a bucket the LSD digit passes remain inherently ordered, while buckets
are now parallel. `typed_radix_keys` (4.38%) is shared key normalization,
serial prefix histogram/scatter (2.73%) is the largest not-yet-parallel
candidate-only residual, `memset` (1.59%) is buffer initialization, and index
gather (0.75%) is a materialization cost both implementations pay.

## Identity and validation

- Candidate ELF: `f07988412d2e9b90cc853eee19441978d4b4fa5f691cddec3be7810351b1b735`
  (76,001,744 bytes), strict-remote build on `vmi1227854`.
- Reference ELF: `e196423454b9f6cf0a4d284668833b092fc3275277e2794f762d2080eeabf484`
  (75,912,896 bytes), strict-remote build on `vmi1227854`.
- Python ELF: `34f3f446f5e1dac82d603c7b4519823eb098d3361ed517ebe9fcad1e68d06bbb`;
  pandas distribution SHA-256 `bc82fc75b88d683dac9346e5067f35d72fce509564447d76d6f867d5e1825bbe`.
- Harness SHA-256: `c98d2d3af9daa6b086dde6682e365d42fc0068c45623f5cd3c3ab54c5ff6b4a6`.
- Host: `threadripperje`, AMD Ryzen Threadripper PRO 5995WX, 64 physical / 128
  logical CPUs, affinity `0-63`; runtime ISA includes AVX2, FMA, BMI2, and VAES.
- Stable-reference differential test passed strict-remote; exact isolated
  workspace `cargo check --workspace --all-targets` passed.
- Workspace Clippy reached the pre-existing `fp-columnar` backlog; its one new
  diagnostic was fixed before commit. The bounded UBS scan reproduced the
  tracked broad `br-frankenpandas-yavyk` inventory.

Machine-readable summary:
`artifacts/bench/cod_parallel_multisort_prefix_10m_trj_20260802.json`.
