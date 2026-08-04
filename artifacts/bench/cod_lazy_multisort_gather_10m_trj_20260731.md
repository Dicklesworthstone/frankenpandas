# Deferred Float64 multi-key sort gather: 10M rows on trj

Decision: **KEEP**

The 10M-row, 10-column Float64 `sort_values_multi` return boundary is now
**2.956x faster than live pandas 2.2.3** and **1.355x faster than the immutable
pre-change FrankenPandas binary**.

The canonical host-wide preflight did not reach a clear readiness window, so
this is a directional same-driver result. Candidate, reference, and live pandas
still ran in one invocation on the same `0-63` cpuset with their own interleaved
A/A controls. Both corrected median-CI decisions pass all three clauses.

## Whole-job result

Workload: `dataframe_ops/sort_values_multi/10M/float64`, stable ascending sort
on `col_0`, `col_1`, and `col_2` over ten all-valid Float64 columns.

| Arm | p50 | p95 | p99 | CV | Observed operation threads |
|---|---:|---:|---:|---:|---:|
| deferred multi-gather candidate | 5,602.070 ms | 5,719.556 ms | 24,994.687 ms | 80.56% | 1 |
| immutable pre-change FP | 7,591.408 ms | 7,773.873 ms | 7,873.146 ms | 6.31% | 10 |
| pandas 2.2.3 | 16,558.532 ms | 17,229.122 ms | 17,283.449 ms | 2.38% | 1 |

- Reference / candidate: **1.355108x**, bootstrap median-ratio 95% CI
  **[1.341178, 1.365453]**.
- pandas / candidate: **2.956x**, bootstrap median-ratio 95% CI
  **[2.919162, 2.994033]**.
- Candidate, reference, and pandas A/A medians were respectively `1.003174`,
  `1.009579`, and `0.995943`; all remain within 2% of unity.
- Candidate and reference liveness checksums match: `4957dea0fe3e2ed1`.
- The candidate's first timed sample was a cold 41.884 s outlier, which inflates
  p99 and CV. CV is provenance rather than the decision gate; the 5.602 s
  median, effect CI, and A/A median remain decisive.

## Lever and residual classification

The multi-key radix and typed-comparison paths computed an owned stable row
permutation, borrowed it into the eager reorder helper, and immediately copied
all ten payload columns. The single-key path already moved the same ownership
into the existing shared deferred-gather representation. This change extends
that representation to both multi-key paths: the permutation is moved once,
shared by all eligible Float64 columns, and each column materializes at most
once through its existing `OnceLock` only when dense values are requested.

| Residual | Classification | Disposition |
|---|---|---|
| ten payload gathers after lexsort | Independent and parallelizable, but eliminable | Removed from the sort return boundary by sharing the owned permutation. |
| multi-key radix ordering | Partition-parallel with ordered digit dependencies | Unchanged; this lever preserves its exact stable order. |
| comparison fallback ordering | Not-yet-parallel | Unchanged algorithm; its owned result now takes the same lazy handoff. |
| eager index reorder | Parallelizable | Retained; small relative to ten 80 MB payload columns. |
| final payload observation | Parallelizable and deferrable | Paid once only by consumers that actually demand each dense column. |

The thread probe observed one operation thread for this candidate run, so this
artifact makes **no new parallel-core claim**. The speedup is work elimination:
roughly 800 MB of immediate payload gathering disappears from the API boundary.

## Semantic witness and risk note

- The stable lexicographic permutation is computed by the exact same radix or
  comparison code; only its ownership handoff changes.
- `deferred_multi_sort_gather_matches_stable_lexicographic_order` forces full
  materialization of both differently-directed keys and a payload column at
  262,144 rows, then compares every Float64 bit pattern with an independent
  stable lexicographic reference.
- Eligible frames are the existing deferred-gather domain: large, non-row-
  MultiIndex frames whose columns are all-valid supported Float64 storage.
  Nullable, mixed-dtype, row-MultiIndex, and small-frame paths remain eager.
- The index is still reordered eagerly, preserving labels and output ordering.
- A consumer that immediately reads every sorted payload pays each gather once;
  projection, filtering, aggregation, or abandoned results avoid unused work.

## Identity and validation

- Source base: `f37890e6a96e3de5d092d86cbe3c35f00afa52ff` plus the owned
  `crates/fp-frame/src/lib.rs` overlay.
- Candidate ELF: `40853cd87541b699e58a382498732a3871377408ea7dbb6377082d51b8a8be2f`
  (75,300,144 bytes), strict-remote build worker `vmi1264463`.
- Reference ELF: `df6eabdf7de0ac848953a007a9be3c76ddf395e1491ae13b31bd5571fe111f85`
  (75,293,824 bytes), the exact live incumbent from the prior kept sort lever.
- Python ELF: `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`;
  pandas `2.2.3`.
- Harness: `b9affde33c6e14a19ad118d6d3f0252c72fd38ee4cc9983517f5d244ebf859d4`.
- Host: `threadripperje`, AMD Ryzen Threadripper PRO 5995WX, 64 physical /
  128 logical CPUs, performance governor, kernel `6.17.0-41-generic`; affinity
  `0-63`, one logical CPU from each physical core.
- Focused forced-materialization parity test: passed strict-remote.
- Strict-remote `cargo check --workspace --all-targets`: passed with only
  pre-existing warnings outside this lever.
- Strict-remote workspace Clippy stopped at the existing `fp-columnar` backlog
  (25 library and 66 test-target errors under `-D warnings`); no diagnostic
  points into `fp-frame` or the owned multi-sort change.
- Owned diff passes `git diff --check`; crate fmt check reaches only unrelated
  pre-existing example formatting drift.

Machine-readable summary:
`artifacts/bench/cod_lazy_multisort_gather_10m_trj_20260731.json`.
