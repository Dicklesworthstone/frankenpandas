# REJECTED — contiguous-Utf8 fast path for Column::unique (str dedup)

## Lever attempted (one)
Give `Column::unique` an all-valid contiguous-Utf8 fast path that dedups directly
over `&[u8]` byte spans (`FxHashSet<&[u8]>`) and emits a contiguous-Utf8 result,
instead of the general path that iterates `self.values` (materializing a
`Scalar::Utf8(String)` per row) and boxes the distinct values through
`Vec<Scalar>` + `Self::new`. Hypothesis: avoiding ~200k per-row `String`
allocations on a 200k-row / 1000-distinct column would be a large win.

Bit-identical: golden FNV over the output strings unchanged (`005f43c2b7326d69`,
out_len=1000) before and after — the lever is correct, just not fast enough.

## Why REJECTED (Score < 2.0)
`ScalarValues::LazyContiguousUtf8` memoizes its materialized `Vec<Scalar>` in a
`OnceLock`. The 200k `String` allocations therefore happen **once** (first
`.values` touch) and are reused by every later op on that column. So:

- **Warm (column reused — the common pipeline case):** before 2.18 ms → after
  1.99 ms = **1.07×**. The 200k allocations are already amortized; steady state
  is hash-bound (200k `FxHashSet` probes of the same ~10-byte keys either way),
  which the lever does not reduce.
- **Cold (fresh column, one-shot unique):** unmeasurable cleanly — the per-call
  `build()` cost (~12.9 ms for 200k formatted strings) dominates and its variance
  swamps the unique delta (subtracted `unique_only` swings 1.9–2.4 ms).

No demonstrable ≥2× on either path. Reverted; the reusable bench
(`bench_str_unique.rs`) is kept.

## Root-cause note (prevents re-attempts)
The `OnceLock<Vec<Scalar>>` materialization cache on lazy columns means
"avoid Scalar materialization in op X" levers only help the FIRST op on a fresh
column — every later op on the same column reuses the cache. To beat steady
state you must reduce the actual hashing/compute, not the materialization. (The
same caveat applies to factorize / value_counts / drop_duplicates on
contiguous-Utf8 columns.)

## Session finding & next swings (per no-ceiling reporting rule)
A full profile sweep (200k) + ownership pass shows the ownable kernel surface is
already optimized to memory/compute bounds:
- fp-columnar: arithmetic (same/cross-index fused), comparison (memory-bound),
  rank (radix + typed i64 output, dwmu9), reductions (typed two-pass), cumulative
  (typed), unique/factorize (dense-i64 + FxHash), radix multi-sort (skips
  constant byte-passes).
- fp-frame row gather (`reorder_rows_by_positions`) is already typed
  (`take_positions`) AND column-parallel; `get_indexer` is sorted-merge/bsearch.
- fp-join is FxHash build/probe; fanout-bound.

The remaining real gaps are NOT ownable micro-levers:
1. **datetime parse-once (0ezw7), target 5–10× dt accessors + parity fix** —
   `to_datetime`/CSV produce `Scalar::Utf8` (re-parsed per dt access) where pandas
   returns datetime64. The fix (return `Scalar::Datetime64`) is *more* faithful
   but breaks 30+ tests that assert the current Utf8 output → genuinely
   multi-session, in fp-frame.
2. **corr/cov/spearman/kendall Gram (~900ms), target 2–4×** — FMA-gated, blocked
   on jawxr orchestrator sign-off (golden regen under `f64::mul_add`).
3. **groupby orchestration (transform/rank/quantile 6–16ms)** — grouping + frame
   assembly in fp-frame dominate; the per-group kernels are not the bottleneck.

These are the next swings, all requiring fp-frame coordination (OrangePeak's
active crate) or FMA sign-off rather than an isolated columnar lever.
