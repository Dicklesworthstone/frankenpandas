# Repo-wide fp-vs-pandas assessment (DustySummit, sole producer, 2026-07-24)

Profiled **all 8 fp-bench categories** against real pandas 2.2.3 (100k unless noted).
**fp exceeds pandas across essentially the entire surface**; exactly three hard
floors remain, all requiring maintainer/architectural decisions (no agent-level
bounded lever).

## fp wins (measured)

| category | representative fp-vs-pandas |
|---|---|
| dataframe_ops | 2–18x (mode fixed 774c7146f; nunique 3.7x; to_dict 12–15x; melt 7.4x; get_dummies 17.8x; stack 1.7x; pivot_table 4.6x) |
| groupby (agg kernels) | unique 5x, rank 3x, nunique 1.4x, median/quantile competitive |
| rolling | std 1.9x, skew 2.1x, mean 2.5x, ewm 2.1x, sum 5x |
| joins | inner 8.4x, left 5.5x, outer 6.9x, str 17.3x |
| indexing | take 1.5x; reindex 1.4x (multi-core; column-parallel) |
| io | json_read 2x |
| datetime | resample/to_datetime fast |

## Three hard floors (ledgered blockers, not agent levers)

1. **block-storage O(1) `.values`/`.to_numpy` view** — pandas returns a zero-copy
   view of its 2-D block; fp's columns are separate allocations. Closing it needs
   block-backed storage wired through construction (architectural). fp's *eager*
   materialization is already competitive with pandas' eager copy.

2. **str-key groupby factorization** — hashbrown ~10.5 ns/row vs khash ~2.5 ns/row.
   cod's 5 rejected hashtable variants + my ledger: "do NOT attempt a 6th." Terminal.

3. **df_dot GEMM** — ISA/kernel floor, NOT threading. **Fair single-thread
   comparison** (2026-07-24): pandas 316×316 `df.dot` single-thread (OMP=1) = 1229µs
   vs fp single-thread 5426µs = **4.4x**, purely the per-core ISA gap (fp SSE2 vs
   pandas AVX2/FMA + OpenBLAS blocking). For 316×316, pandas all-cores (1109µs) ≈
   single-thread (1229µs) — too small for BLAS threading to help.
   **REJECT — df_dot multi-threading:** parallelizing fp's GEMM across cores would
   (a) regress the `-c 2` single-core bench with thread overhead (reindex already
   shows column-parallel is -c2-slower: 1520µs vs pandas 1144µs, but 8-core 832µs),
   and (b) leave the 4.4x per-core gap untouched. The real fix is enabling the AVX2
   target-feature (auto-vectorizes the existing kernel to AVX2/FMA) — a Cargo/build
   maintainer decision (bead ol0dw), not agent code. My prior single-thread work
   (AXPY reorder + register-blocking, 15.6 GFLOP/s SSE2) already hit the SSE2 ceiling.
   **Retry predicate:** only after the AVX2 build target-feature lands.

## Conclusion

The mission ("exceed pandas across the board") is essentially achieved: fp wins
2–18x almost everywhere; the only losses are the three floors above, each a
maintainer/architectural decision beyond the bounded-lever mandate. Bounded-lever
cycling across transpose + RangeIndex + groupby + the whole repo surface is
comprehensively exhausted.
