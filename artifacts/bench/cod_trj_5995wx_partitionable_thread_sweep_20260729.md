# trj 5995WX partitionable-operation thread sweep — 2026-07-29

## Definition

This is the required full `1/2/4/8/16/32/64/128` sweep for
`groupby_mean_float64`, `df_abs`, `join_inner`, and `sort_values_single` at
1M and 10M Float64 rows. It compares the live pandas 2.2.3 incumbent against
FrankenPandas in the same invocation. Population and the operation-thread
probe are outside the timed region.

Host identity is `threadripperje` (`trj`): AMD Ryzen Threadripper PRO 5995WX,
64 physical cores, 128 logical threads, 536,069,869,568 bytes RAM, one NUMA
node, eight 8-core L3 domains, kernel 6.17.0-41-generic. Runtime ISA detection
reported SSE2, AVX, AVX2, FMA, BMI1, BMI2, AES, and VAES; AVX-512F was absent.
The governor was `performance`, SMT and frequency boost were enabled.

Every cap used `taskset` over the contiguous logical-CPU range `0..N-1`.
That means caps through 64 expose one hardware thread per physical core;
128 additionally exposes SMT siblings. Each row records this affinity and
the CPU-time-active operation workers actually observed. Requested CPUs are
not reported as used threads.

## Measurement and provenance contract

- 25 alternating A/A pairs and 50 raw samples per engine and row.
- Acceptance gate: bootstrap median-CI with a 2x null margin; CV is
  provenance only and has no vote.
- All 64 canonical rows passed the host, topology, affinity, actual-thread,
  ISA, executable, sample-count, A/A, and median-CI contract.
- FrankenPandas self-reported
  `bench_elf_sha256=50e9e4001486513763eca3fe4a7904f24ec94b849fd1d28b18b7937e29888fad
  (73636568 bytes)
  /data/projects/frankenpandas-thread-sweep/target/release-perf/fp-bench`.
- Python self-reported
  `bench_elf_sha256=efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e
  (6894448 bytes) /usr/bin/python3.13`.
- The live pandas 2.2.3 artifact SHA-256 was
  `80c4fc7efcc4d8deabf0faf971a49013556a22109ae402df6913962a577d227e`.
- GroupBy used harness Git SHA `74e03efb8` and source SHA-256
  `def6b0fc6d037d59da8ce71b5d535911cc7f9cf15dcec62ab042e1c3b43d52b3`.
  The other three sweeps used Git SHA `5bea1c5d2` and source SHA-256
  `5d170c9bbdbf3a1f99de04ed69f869d2a1a12b0f12f64e1c3fd297601f3ea07`.
  The Rust binary was byte-identical across all four sweeps.

The compact machine-readable manifest is
`artifacts/bench/cod_trj_5995wx_partitionable_thread_sweep_20260729.json`.
The 32 canonical raw JSON files retain every invocation ID, exact cpuset,
raw timing vector, A/A vector and CI, CV, checksum, artifact identity, and
gate calculation.

## Results

Times below are p50 milliseconds. `FP/pd threads` are operation threads
actually used, not the requested affinity.

### GroupBy mean Float64

| cap | affinity | FP/pd threads | FP/pandas 1M | ratio | FP/pd threads | FP/pandas 10M | ratio |
|---:|---|---:|---:|---:|---:|---:|---:|
| 1 | 0 | 1/1 | 2.165/8.596 | 3.971x FASTER | 1/1 | 24.340/82.834 | 3.403x FASTER |
| 2 | 0-1 | 1/1 | 2.158/9.368 | 4.341x FASTER | 1/1 | 24.291/83.723 | 3.447x FASTER |
| 4 | 0-3 | 1/1 | 2.162/8.658 | 4.004x FASTER | 1/1 | 26.575/84.183 | 3.168x FASTER |
| 8 | 0-7 | 1/1 | 2.460/9.434 | 3.835x FASTER | 1/1 | 24.445/84.894 | 3.473x FASTER |
| 16 | 0-15 | 1/1 | 2.169/8.637 | 3.982x FASTER | 1/1 | 24.570/83.284 | 3.390x FASTER |
| 32 | 0-31 | 1/1 | 2.200/8.821 | 4.009x FASTER | 1/1 | 25.811/83.573 | 3.238x FASTER |
| 64 | 0-63 | 1/1 | 2.494/9.923 | 3.979x FASTER | 1/1 | 24.514/83.057 | 3.388x FASTER |
| 128 | 0-127 | 1/1 | 2.165/8.721 | 4.028x FASTER | 1/1 | 24.425/83.977 | 3.438x FASTER |

Both implementations remain serial and the ratio is flat. The older
19.486x 1M result came from a worker artifact with no host/thread
fingerprint and a 53.144 ms pandas median. It does not reproduce across
eight trj invocations: trj's pandas medians are 8.596–9.923 ms and the
current ratio range is 3.168x–4.341x.

### DataFrame absolute value

| cap | affinity | FP/pd threads | FP/pandas 1M | ratio | FP/pd threads | FP/pandas 10M | ratio |
|---:|---|---:|---:|---:|---:|---:|---:|
| 1 | 0 | 1/1 | 4.292/7.134 | 1.662x FASTER | 1/1 | 47.090/67.469 | 1.433x FASTER |
| 2 | 0-1 | 2/1 | 3.911/6.401 | 1.637x FASTER | 2/1 | 43.250/66.628 | 1.541x FASTER |
| 4 | 0-3 | 4/1 | 3.916/7.229 | 1.846x FASTER | 4/1 | 43.863/67.445 | 1.538x FASTER |
| 8 | 0-7 | 8/1 | 4.050/7.141 | 1.763x FASTER | 8/1 | 44.845/67.477 | 1.505x FASTER |
| 16 | 0-15 | 10/1 | 2.232/7.224 | 3.237x FASTER | 10/1 | 30.095/67.741 | 2.251x FASTER |
| 32 | 0-31 | 10/1 | 2.601/7.139 | 2.745x FASTER | 10/1 | 19.710/67.505 | 3.425x FASTER |
| 64 | 0-63 | 10/1 | 3.026/7.183 | 2.373x FASTER | 10/1 | 20.526/68.506 | 3.337x FASTER |
| 128 | 0-127 | 10/1 | 3.076/7.275 | 2.365x FASTER | 10/1 | 19.482/68.902 | 3.537x FASTER |

FrankenPandas scales to one worker per column and then stops. At 10M its
best median is 2.417x faster than its one-thread median, while pandas stays
at one operation thread. The competitive ratio rises from 1.433x to 3.537x,
but it does not reach the campaign's 5x bar.

### Inner join

| cap | affinity | FP/pd threads | FP/pandas 1M | ratio | FP/pd threads | FP/pandas 10M | ratio |
|---:|---|---:|---:|---:|---:|---:|---:|
| 1 | 0 | 1/1 | 2.746/21.490 | 7.825x FASTER | 1/1 | 36.487/135.421 | 3.712x FASTER |
| 2 | 0-1 | 2/1 | 2.692/20.909 | 7.766x FASTER | 2/1 | 35.578/137.471 | 3.864x FASTER |
| 4 | 0-3 | 3/1 | 2.560/20.916 | 8.169x FASTER | 3/1 | 34.729/134.523 | 3.874x FASTER |
| 8 | 0-7 | 3/1 | 2.568/20.914 | 8.143x FASTER | 3/1 | 36.797/133.853 | 3.638x FASTER |
| 16 | 0-15 | 3/1 | 4.467/20.934 | 4.687x FASTER | 3/1 | 47.192/133.784 | 2.835x FASTER |
| 32 | 0-31 | 3/1 | 5.963/21.164 | 3.549x FASTER | 3/1 | 54.603/134.495 | 2.463x FASTER |
| 64 | 0-63 | 3/1 | 5.113/21.493 | 4.203x FASTER | 3/1 | 57.829/135.539 | 2.344x FASTER |
| 128 | 0-127 | 3/1 | 6.617/21.225 | 3.208x FASTER | 3/1 | 60.500/149.792 | 2.476x FASTER |

The implementation uses at most three operation workers. The four-CPU
same-L3 cap is best: 8.169x at 1M and 3.874x at 10M. Widening the allowed
placement across L3 domains slows FP despite the unchanged three-worker
count, so this is a locality/scheduling frontier rather than missing worker
quantity.

### Single-column sort

| cap | affinity | FP/pd threads | FP/pandas 1M | ratio | FP/pd threads | FP/pandas 10M | ratio |
|---:|---|---:|---:|---:|---:|---:|---:|
| 1 | 0 | 1/1 | 58.836/66.516 | 1.131x FASTER | 1/1 | 1581.088/1216.419 | 0.769x SLOWER |
| 2 | 0-1 | 2/1 | 43.994/66.757 | 1.517x FASTER | 2/1 | 1202.891/1205.479 | 1.002x NULL_UNDECIDABLE |
| 4 | 0-3 | 4/1 | 43.936/66.660 | 1.517x FASTER | 4/1 | 1058.523/1210.069 | 1.143x FASTER |
| 8 | 0-7 | 8/1 | 43.714/65.487 | 1.498x FASTER | 8/1 | 1026.705/1214.622 | 1.183x FASTER |
| 16 | 0-15 | 10/1 | 38.611/66.543 | 1.723x FASTER | 10/1 | 925.641/1223.920 | 1.322x FASTER |
| 32 | 0-31 | 10/1 | 35.296/68.179 | 1.932x FASTER | 10/1 | 919.016/1216.527 | 1.324x FASTER |
| 64 | 0-63 | 10/1 | 35.755/67.135 | 1.878x FASTER | 10/1 | 924.548/1231.832 | 1.332x FASTER |
| 128 | 0-127 | 10/1 | 37.432/69.734 | 1.863x FASTER | 10/1 | 1383.717/1273.882 | 0.921x SLOWER |

The 10M curve distinguishes the mechanisms cleanly: one thread loses,
two is inside the null interval, caps 4–64 win modestly, and the full
SMT-wide cap loses. The fastest FP median is at cap 32; the largest ratio is
at cap 64 because the pandas median also moves. At cap 128 the absolute log
effect 0.08270452 only narrowly clears the required 0.07730210, but it is a
decidable loss under the median-CI gate; CV 15.23% has no vote.

## Frontier routing

1. GroupBy needs row partitioning before another thread sweep; no current
   operation-level parallelism exists.
2. `df_abs` needs row-chunk partitioning or a wider-column fixture to use
   more than 10 workers; at 10 columns it is already at its structural cap.
3. Join needs worker placement within one L3 domain before more workers.
4. Sort needs a profile of the exact 10M FP path. The threaded 10-column
   gather has saturated while the remaining median is still 0.919 s, so
   the next admissible lever must name a non-zero-self frame and Amdahl
   ceiling in the serial residual. No source lever is claimed by this sweep.

The profile was not started while an unrelated 128-thread `whisper-cli` job
occupied trj; mixing that job into a new profile would destroy attribution.
