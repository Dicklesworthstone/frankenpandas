# FrankenPandas Perf — Negative-Evidence Ledger & Head-to-Head vs pandas

Measured head-to-head: each lever's realistic-workload bench run **locally** from the
release binary (built remotely via `rch exec -- cargo build --release -p fp-frame
--examples`, artifacts transferred to `/data/projects/.rch-targets/frankenpandas-cc`),
compared against **pandas 2.2.3** (`python3`, `time.perf_counter`, best-of-N).

Method: `examples/bench_*.rs` self-time `best=<ns>` over N iters; pandas scripts
(`perf/pandas_baseline/`) time the equivalent op best-of-N on a matched workload.
`ratio = pandas_ns / fp_ns` (>1 ⇒ fp faster). Same machine, single-thread unless noted.

Rule: record EVERY result (win/loss/neutral). Revert any lever that regressed or showed
~0 gain. Never retry a recorded dead end.

## Results

| Lever (bead) | Workload | pandas | fp | ratio | verdict |
|---|---|---|---:|---:|---|
| value_counts FxHash (g1de8) | 500k rows, 5k distinct Utf8 | 22.90 ms | 8.85 ms | **2.59× faster** | ✅ KEEP — beat khash (was 0.62× pre-lever) |
| sort_values gather/reorder (7ufhq+take_positions) | 1M shuffled int64 | 57.18 ms | 47.68 ms | **1.20× faster** | ✅ KEEP — was 0.91× pre-levers |
| head/tail zero-copy slice (6wx84) | 2M int64, k=5 | 5.91 µs | 0.35 µs | **~17× faster** | ✅ KEEP — Index::slice+Column::slice |
| loc_bool filter (t0y8n) | 2M, 50% mask | 10.89 ms | 8.42 ms | **1.29× faster** | ✅ KEEP — collect-positions + take_positions |
| drop_duplicates FxHashSet (6vep3) | 1M, card 1000 | 5.99 ms | 2.95 ms | **2.03× faster** | ✅ KEEP — beat khash dedup |
| sum typed Int64 (bwgyc) | 2M int64 | 216 µs | 171 µs | **1.27× faster** | ✅ KEEP |
| std / var typed (0xdfx Welford) | 2M int64 | 19.5 ms | 1.72 ms | **11.3× faster** | ✅ KEEP — Welford crushes pandas std |
| max typed Int64 (4qs3h) | 2M int64 | 219 µs | 1.15 ms | **0.19× (5.2× SLOWER)** | ⚠️ KEEP (beats old fp Scalar path) but LOSES to numpy SIMD — gap |
| min typed Int64 (4qs3h) | 2M int64 | 230 µs | 1.17 ms | **0.20× (5.1× SLOWER)** | ⚠️ KEEP but LOSES to numpy SIMD — gap |

### Gap: max/min lose to numpy SIMD (~5×)
fp `Series.max/min` use `iter().max()` (Option/Ord comparison loop, doesn't auto-vectorize)
vs numpy's SIMD max. NOT a regression (the typed lever beats old fp's Scalar iteration), so
not reverted — but a real vs-pandas gap. FIX ATTEMPT (REVERTED): branchless
`fold(i64::MIN, i64::max)` — built+measured, **max stayed 1.15 ms (~0 gain)**; LLVM did not
auto-vectorize the i64 min/max reduction here. Reverted to the simpler `iter().max()`
(working-tree only, never committed). CONCLUSION: beating numpy SIMD max/min needs explicit
SIMD (portable_simd / chunked manual vectorization) — out of safe-Rust auto-vec reach; a
candidate radical lever for the kernel layer, NOT a code-first fp-frame change. Dead end
recorded — do not retry the branchless-fold approach.

## pandas 2.2.3 baselines (best-of-N, for pending comparisons)

| op | workload | pandas best |
|---|---|---:|
| sort_values | 1M shuffled int64 | 57.2 ms |
| head(5) | 2M int64 | 5.9 µs |
| tail(5) | 2M int64 | 5.9 µs |
| s[mask] (filter) | 2M, 50% mask | 10.9 ms |
| drop_duplicates | 1M, card 1000 | 5.99 ms |
| sum | 2M int64 | 190 µs |
| max | 2M int64 | 200 µs |
| std | 2M int64 | 5.44 ms |

(fp numbers for these pending the all-examples release build; rows added as measured.)

## Reverts

_None yet — value_counts confirmed a win, kept._

## Notes / gotchas

- rch executes **remotely** (ovh-b); `cargo run` does NOT relay the program's stdout, so
  build via rch (artifacts transfer back) then run the binary **locally** for timing.
- Release build of fp-frame is ~5 min remote; subsequent example builds reuse the cached lib.
