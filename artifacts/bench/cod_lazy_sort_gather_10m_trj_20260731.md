# Deferred Float64 sort gather: 10M rows on trj

Decision: **KEEP**

The 10M-row, 10-column Float64 `sort_values_single` return boundary is now
**9.708x faster than live pandas 2.2.3** and **1.767x faster than the immutable
pre-change FrankenPandas binary**.

The canonical host-wide preflight was unavailable, so this is a directional
same-driver result rather than a host-admitted headline. Candidate, reference,
and live pandas nevertheless ran in one invocation on the same `0-63` cpuset,
with their own interleaved A/A controls. Both median-CI decisions pass all three
corrected clauses.

## Result

Workload: `dataframe_ops/sort_values_single/10M/float64`.

| Arm | p50 | p95 | p99 | CV | Observed operation threads |
|---|---:|---:|---:|---:|---:|
| deferred-gather candidate | 138.753 ms | 143.494 ms | 144.680 ms | 1.81% | 16 |
| immutable pre-change FP | 245.234 ms | 252.937 ms | 255.865 ms | 1.66% | 16 |
| pandas 2.2.3 | 1,346.970 ms | 1,364.834 ms | 1,369.425 ms | 0.73% | 1 |

- Reference / candidate: **1.767409x**, bootstrap median-ratio 95% CI
  **[1.754732, 1.780431]**.
- pandas / candidate: **9.708x**, bootstrap median-ratio 95% CI
  **[9.643494, 9.771549]**.
- Candidate, reference, and pandas A/A medians were respectively `1.008517`,
  `0.995208`, and `1.001195`.
- Candidate and reference liveness checksums match:
  `4957dea0fe3e2ed1`.
- The process had one hardware thread on each of all 64 physical cores in its
  affinity. The radix operation activated 16 workers for this key distribution;
  pandas activated one.

## Residual classification and lever

The post-radix profile made parallel `Column::take_positions` the aggregate
leader at **84.92%** while the main-thread share had fallen to **5.56%**. That
gather is not inherently sequential: every destination row and every column is
independent. Adding another nested thread layer would still move 800 MB of
payload, so this lever removes the work from the sort return boundary instead.

| Profile entry | Classification | Disposition |
|---|---|---|
| `Column::take_positions` (84.92% aggregate) | Parallelizable, not inherently sequential | Largest removable residual; replaced by shared deferred gathers. |
| `radix_argsort_u64` | Mixed | Radix digits are ordered; work across high-prefix partitions is independent and already parallel. |
| `typed_dense_sort_order` assembly | Not yet parallel / eliminable | Element-independent, but smaller than payload gather. |
| `Index::take` | Parallelizable | Remains eager; profile contribution was small. |
| faults, memset, and copies | Parallelizable memory service | Reduced by avoiding ten eager payload buffers. |
| final single-owner drop | Inherently sequential per allocation | Negligible. |

For eligible frames (at least 262,144 rows, no row MultiIndex, and exclusively
all-valid Arc-backed Float64 columns), the owned permutation is moved into one
`Arc<Vec<usize>>`. Each output column stores its immutable source plus that
shared permutation and materializes exactly once through `OnceLock` only when a
consumer asks for dense values. The index still reorders eagerly. Nullable,
mixed-dtype, MultiIndex, and small frames retain the existing eager path.

This is an AACE lazy-materialization win: the timed API boundary returns a
semantically complete sorted frame without eagerly copying payload that may be
filtered, aggregated, projected away, or never observed. A consumer that
immediately reads every sorted value pays the deferred gather once; the focused
tests force that materialization and compare it bit-for-bit with eager output.

## Identity and host

- Source commit: `d5391268230c20371ed035d90143b018e5a161a4`.
- Candidate ELF: `df6eabdf7de0ac848953a007a9be3c76ddf395e1491ae13b31bd5571fe111f85`
  (`75,293,824` bytes), strict-remote build worker `vmi1264463`.
- Reference ELF: `727ff81d81aa5e396101de1479669bbe0d94966a80e0019a37b9ca0a4125026a`
  (`75,121,752` bytes).
- Python ELF: `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`
  (`6,894,448` bytes); pandas `2.2.3`.
- Harness: `b9affde33c6e14a19ad118d6d3f0252c72fd38ee4cc9983517f5d244ebf859d4`
  (`141,699` bytes).
- Host: `threadripperje`, AMD Ryzen Threadripper PRO 5995WX, 64 physical /
  128 logical CPUs, 536,069,869,568 bytes RAM, performance governor, kernel
  `6.17.0-41-generic`.
- Affinity: logical CPUs `0-63`, exactly one thread per physical core.
- Runtime ISA: SSE2, AVX, AVX2, FMA, BMI1/BMI2, AES, and VAES; no AVX-512.

Machine-readable summary:
`artifacts/bench/cod_lazy_sort_gather_10m_trj_20260731.json`.

## Semantic and build checks

- `deferred_sort_gather_column_matches_eager_bits`: passed remotely.
- `deferred_sort_gather_frame_matches_eager_order`: passed remotely at the
  262,144-row activation threshold.
- Strict-remote `cargo check --workspace --all-targets`: passed; only existing
  workspace warnings were emitted.
- Strict-remote workspace Clippy reached the tracked `fp-columnar` backlog (25
  library / 66 test-target errors under `-D warnings`); none points into the
  deferred-gather implementation.
- `fp-columnar` bounded UBS reproduced the tracked broad inventory and found no
  unsafe code in the lever; the bounded `fp-frame` scan reproduced its
  documented 180-second timeout.
- Owned hunks pass rustfmt and `git diff --check`; workspace rustfmt remains
  blocked by unrelated pre-existing formatting drift.
