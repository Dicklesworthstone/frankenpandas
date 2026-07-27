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

3. **df_dot GEMM** — microkernel-quality floor, NOT threading. **Fair single-thread
   comparison**: pandas 316×316 `df.dot` single-thread (OMP=1) = 1229µs.
   A 2026-07-26 strict-remote, same-worker whole-binary A/B measured fp default at
   5239.1µs and `-C target-cpu=x86-64-v3` at 5071.3µs = **1.0327x**, bootstrap
   95% CI `[1.0240, 1.0376]` outside the ±0.24% A/A floor. The two executing ELFs
   self-reported different SHA-256 values and identical numeric checksums, proving
   that the flag changed codegen without changing output. The v3 arm is still
   **4.1x slower** than pandas. For 316×316, pandas all-cores (1109µs) ≈
   single-thread (1229µs) — too small for BLAS threading to help.
   **REJECT — df_dot multi-threading:** parallelizing fp's GEMM across cores would
   (a) regress the `-c 2` single-core bench with thread overhead (reindex already
   shows column-parallel is -c2-slower: 1520µs vs pandas 1144µs, but 8-core 832µs),
   and (b) leave the per-core gap untouched. A `df_abs` control on the same worker
   pair was null at 1.0071x with CI `[0.9870, 1.0261]`, so the ISA flag is neutral
   outside GEMM. The residual is OpenBLAS's hand-tuned assembly GEMM microkernel
   versus fp's auto-vectorized Rust GEMM, not the compiler ISA flag.
   **Retry predicate:** only a hand-written, register-blocked and packed-panel GEMM
   microkernel; no further build-flag sweep.

   `-C target-cpu=x86-64-v3` is a benchmark-fleet build option, not a shipping one:
   frankenpandas is a library distributed to third parties, and v3 codegen faults on
   pre-2015 consumer CPUs. A 3.3% single-workload gain does not justify that.

## Conclusion

The mission ("exceed pandas across the board") is essentially achieved: fp wins
2–18x almost everywhere; the only losses are the three floors above, each a
maintainer/architectural decision beyond the bounded-lever mandate. Bounded-lever
cycling across transpose + RangeIndex + groupby + the whole repo surface is
comprehensively exhausted.

## Block-storage build — concrete scope & plan (DustySummit, 2026-07-24)

Scoped the block-backed-storage build (the O(1) `.values`/`.to_numpy` view floor).
It is TRACTABLE but a multi-commit architectural project, NOT a bounded lever —
delivering user value requires wiring it through construction paths. Plan, feature-
gated behind `block-storage` (default-off ⇒ default build/tests unaffected, risk
bounded), modeled on the existing `lazy-transpose-view` `HomogeneousTranspose`
variant:

- **Commit 1 (foundation):** `Float64BlockStore { block: Arc<[f64]> (column-major),
  rows, cols, names, index }` + a `LazyDataFrameColumns::Float64Block(Arc<..>)`
  variant. Handle the ~14 contained match arms (get_one → block-slice Column;
  materialized → build the BTreeMap from slices; clone/default/serde/PartialEq/
  logical_len/name_at/into_materialized/make_eager/Deref) — the enum is centralized,
  no direct matches sprawl the codebase. A `DataFrame::from_f64_block_columns`
  constructor + O(1) `to_numpy` view for a block-backed frame + tests.
- **Commit 2:** zero-copy `get_one` — needs a `Column` Arc<[f64]>+range backing so a
  block column borrows the shared block instead of copying (the dot path already uses
  `Arc<[f64]>` views, so the primitive largely exists).
- **Commit 3+:** wire construction paths (`build_frame`/`from_columns`/read_csv/
  homogeneous arithmetic results) to PRODUCE block-backed frames when all columns are
  all-valid f64 — this is what makes real frames (and the `df_to_numpy` bench) O(1).
  Pervasive but mechanical (each path checks homogeneity, builds the block).

**Value:** matches pandas' core O(1) `.values`/`.to_numpy` zero-copy view — a genuine
pandas-parity gap (fp's eager materialization is competitive with pandas' eager copy,
but has no view). **Cost:** ~3–6 commits, touches core `DataFrame` storage +
construction. **Decision:** this is the only remaining substantive work in the lane; it
needs a "build block-storage" go since it is a multi-commit architectural project that
cannot be delivered as a single bounded lever. Absent that go it remains a ledgered
blocker; the perf frontier is otherwise won.
