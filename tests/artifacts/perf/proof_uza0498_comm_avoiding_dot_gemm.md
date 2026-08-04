# uza04.98 — communication-avoiding df.dot GEMM — KEPT (2.54x)

BlackThrush, 2026-06-13. perf_event_paranoid=4 (perf blocked), so attribution
came from env-gated `Instant` phase timers in `dot()` (reverted before commit).

## Target

`df_dot` 100000 (GEMM C = A(m=100000 × k=256) · B(256 × 256)): baseline
**572.056 ms/iter**, min-of-5 (rch worker, release-perf). 6.5e9 MACs ⇒ ~11.6
GMAC/s aggregate on 64 cores — ~22× below the kernel's FMA-free ceiling.

## Root cause (phase timers)

```
FP_DOT_PROF m=100000 k=256 n=256 extract=~300ms  compute=~150ms  assemble=~22ms
```

The 300ms `extract` phase was building the row-major `a` buffer via
`a[i*k+l] = self[i][l]` — a scattered column→row TRANSPOSE: 25.6M writes with
stride k=256 (2048 B), i.e. one cache line per write. The prior seti2 work
optimized the 150ms compute kernel — the wrong phase. (A secondary issue: the
column-partitioned kernel made each of 64 workers re-stream all of A from DRAM.)

## Lever (one, bit-identical)

Rewrote `DataFrame::dot`'s GEMM section:
1. **Eliminate the A transpose** — keep A column-major: borrow each all-valid
   Float64 column as its contiguous `&[f64]` (`Cow::Borrowed`, free) and let the
   kernel read `a_cols[l][row]` directly. The 4-row `di` tile walks 4 CONSECUTIVE
   rows of column `l` (contiguous), so column-major is cache-friendly here.
2. **Communication-avoiding row-banding** — each worker owns a disjoint band of
   output ROWS and computes all n columns, reading only its A-band (whole A from
   DRAM once total) with the 4×4 A-tile in L1 and B packed once (≈512KB) in L2.

Isomorphism: every C[i][j] is the same 4×4-tiled, l=0..k ascending left-fold with
the same f64 products (`acc += a*b`, separate `*` then `+=`, no FMA);
`a_cols[l][i] == old a[i*k+l] == self[i][l]`. Only thread assignment and loop
nesting change.

## Proof

- Golden sha256 BYTE-IDENTICAL before/after, df_dot at n=2000 and n=5000
  (ddbde1c3…b535, 04af7c2b…db3f) — exercises the parallel row-band path.
- `cargo test -p fp-frame dot`: 12 passed, 0 failed.
- Phase timers after: extract **0.36ms** (was ~300ms), compute ~165ms,
  assemble ~22ms.
- Timing min-of-6 (rch worker): **572.056 → 224.882 ms/iter = 2.54x**.

Score 2.54 (>= 2.0) → kept. fp-frame check + dot tests green; timers removed;
diff localized to `dot()`.

## Next swing

`df_dot` is now compute-bound at ~150ms (~43 GMAC/s aggregate). The residual is
the FMA-free 4×4 microkernel ceiling (same wall as the corr/cov Gram) plus the
~22ms column assembly. Further GEMM gains need either an FMA build decision
(parity/golden-regen, jawxr — rejected) or a wider explicitly-SIMD microkernel
(prior MR4×NR8 widen was neutral). Profile-gated; needs paranoid<=1.
