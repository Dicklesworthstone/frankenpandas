# uza04.99 — parallelize + lazy-index the rank-corr prep — KEPT (kendall 5.3x, spearman 5.0x)

BlackThrush, 2026-06-13. perf_event_paranoid=4 (perf blocked); attribution via
env-gated `Instant` phase timers in `pairwise_rank_corr` (reverted before commit).

## Target (phase timers, 100000 rows)

The 9 rejected kendall beads (uza04.87–95) all attacked the per-pair inversion
COUNTER — i.e. the `matrix` phase, which was already parallel. The timers show
that was the wrong phase:

```
kendall  n=32  series=31ms   prep=117ms (SERIAL)   matrix=30ms
spearman n=64  series=65ms   prep=10ms             matrix=5ms
```

- Kendall **prep = 117ms (65%)**: per-column `kendall_no_tie_order` is an
  O(n log n) comparison argsort + rank inversion, run in a SERIAL `.iter().map()`
  over the 32 columns.
- Spearman **series = 65ms (81%)**: `series_list` rebuilt a fresh
  `Vec<IndexLabel>` of `len` Int64 labels for EVERY column (n·len ≈ 6.4M label
  allocations) even though all columns share the identical 0..len RangeIndex.

## Lever (one coherent prep rewrite, bit-identical)

1. **Lazy shared row index**: build each series' index via
   `Index::new_known_unique_int64_unit_range(0, len)` (no `Vec<IndexLabel>`
   materialization) instead of `Index::new((0..len).map(...).collect())`.
   Correlation operates on column values by position; index labels never enter a
   cell → identical output.
2. **Parallel per-column kendall prep**: the (values, order, rank) chain is
   independent per column; fan it across workers (work-stealing `thread::scope`),
   placing each column's result back at its index. Per-column results are
   computed exactly as the serial chain → identical.

## Proof

- Golden sha256 **BYTE-IDENTICAL** before/after for df_spearman, df_kendall,
  df_corr, df_cov at n=2000 and n=5000 (8/8). Kendall goldens still
  acf366c2…/031978ba…; spearman 36c26986…/dc37f75e…
- `cargo test -p fp-frame corr`: 52 passed, 0 failed.
- Phase timers after: kendall series 9µs / prep **6.2ms** / matrix 25ms;
  spearman series 14µs / prep 9ms / matrix 4ms.
- Timing min-of-6 (rch worker, timer-free binary):
  - **df_kendall 210.353 → 39.809 ms = 5.28x**
  - **df_spearman 110.701 → 22.026 ms = 5.03x**

Score ≫ 2.0 → kept; timers removed; diff localized to `pairwise_rank_corr`.

## Next swing

Kendall is now matrix-bound again (~25ms, the per-pair Fenwick inversion count,
cache-miss bound — the uza04.87–95 territory). Spearman is now matrix+prep bound
at ~20ms. The next df-wide target: `str_outer_join` (~29ms, output
materialization) and the residual `df_dot`/Gram FMA-free microkernel ceiling.
Methodology that keeps working: phase-time the op first — the parallelized inner
kernel is rarely the real cost; serial prep/extraction/transpose usually is.
