# thinkstation1 10M sort serial-residual profile — 2026-07-29 (cod-pandas)

Ledger-bound entry. `docs/NEGATIVE_EVIDENCE.md` was reserved by ProudChapel at
commit time, so this banks the finding in an artifact the reservation does not
cover; fold it into the ledger when that reservation releases.


### 2026-07-29 cod-pandas — 10M sort serial-residual profile: the residual is 87% of wall, `radix_argsort_u64`'s scatter is 94% of it, and the cost is a dependent RANDOM KEY LOAD, not missing threads

The trj sweep's routing item 4 asked for a profile of the exact 10M FP sort
path before any further lever. This is that profile. It answers, per entry,
whether the cost is *inherently sequential* or *merely not-yet-parallelized* —
the distinction that decides algorithm-change versus more threads. **No timing
claim and no pandas ratio is made here; see the blocked-measurement note.**

**Hardware and artifact provenance.** Host `thinkstation1`, AMD Ryzen
Threadripper PRO 5975WX, 32 physical cores, 64 logical threads, two threads per
core, 231,691,894,784 bytes RAM, one NUMA node, kernel `6.17.0-35-generic`,
`powersave` governor. Profiled ELF SHA-256
`68eff32169e8ab0d0a1ddff50a488c09194a6b630ffa321d58ea903030d90c1a`
(73,637,864 bytes), self-reported by the process and matched against the
external hash on its builder. **Builder identity: rch worker `hz1`**
(Hetzner, EPYC-Milan), project hash `c6b8122cb471d29d`, built from base commit
`6774e9a37` via `rch exec --base --clean-overlay --no-overlay`, then retrieved
by `scp` (Route 1) since `rch exec` has no artifact-retrieval mechanism. This
repo's `.cargo/config.toml` is intentionally empty and the tree sets no
`target-cpu`/`target-feature`, so the worker-built binary is a portable-baseline
artifact whose ISA dispatch resolves against the executing host; the process
reported `scalar,sse2,avx2,fma,bmi2,vaes` on this host. Workload
`dataframe_ops/sort_values_single`, 10M rows × 10 Float64 columns,
`operation_threads_used=10` against `runtime_available_parallelism=64`,
checksum `4957dea0fe3e2ed1`, 50 timed samples, p50 1450.2 ms.
`perf record -F 999 --all-user`: 178,538 samples, **0 lost**.

**Serial-vs-parallel split in WALL terms.** Aggregate CPU time understates the
serial share, so samples were attributed per thread. The main thread (tid
3969403) holds 268,502,778,630 of 657,971,003,553 cycles; the ~280 short-lived
scope workers hold the remaining 389,468,224,923. The main thread runs alone
(`par_map_columns_min` spawns and joins; it does no work itself), and the
gather's 10 workers run concurrently, so wall ∝ 268.50 G serial + 389.47/10 =
38.95 G parallel:

| region | share of aggregate CPU | share of WALL |
|---|---:|---:|
| serial residual (main thread) | 40.8% | **87.3%** |
| 10-column parallel gather (workers) | 59.2% | 12.7% |

That is the Amdahl statement the sweep was missing: making the ten-column
gather *infinitely* fast buys at most 12.7%. The residual is the whole problem.

**Top-10 self-time WITHIN the serial residual** (cycle-weighted, main thread
only):

| # | symbol | self | inherently sequential, or merely not-yet-parallelized? |
|---:|---|---:|---|
| 1 | `fp_columnar::radix_argsort_u64` | 94.826% | **BOTH — see split below** |
| 2 | `fp_frame::typed_dense_sort_order` | 2.433% | not-yet-parallelized; in fact ELIMINABLE |
| 3 | `<fp_index::Index>::take` | 0.720% | merely not-yet-parallelized |
| 4 | `[unknown]` (kernel) | 0.609% | not-yet-parallelized (fault-in of the large buffers) |
| 5 | `__memset_avx2_unaligned_erms` | 0.487% | not-yet-parallelized (`vec![0; n]` scratch zeroing) |
| 6 | `__memmove_avx_unaligned_erms` | 0.485% | not-yet-parallelized |
| 7 | `fp_bench::build_frame` | 0.272% | setup, OUTSIDE the timed op |
| 8 | `<fp_columnar::Column>::from_f64_values` | 0.074% | setup, OUTSIDE the timed op |
| 9 | `sha2::sha256::x86_sha::compress` | 0.052% | provenance checksum, not the op |
| 10 | `drop_in_place::<fp_columnar::ScalarValues>` | 0.025% | inherently sequential (single owner drop) |

**Entry 1 decomposed, because the whole decision turns on it.** Instruction-level
annotation of `radix_argsort_u64` splits it as **scatter loop 94.33%**,
**histogram loop 5.18%**, prefix-sum plus prologue 0.45%. So:

- The **8 LSD passes are INHERENTLY SEQUENTIAL** with respect to each other:
  pass *k+1* permutes pass *k*'s output. That dependence cannot be threaded
  away; only a different algorithm (MSD/bucket-partition) removes it.
- **Within** a pass, both the histogram and the scatter are **merely
  not-yet-parallelized** — a chunked stable counting sort (per-chunk
  histograms → exclusive prefix over (bucket, chunk) → disjoint per-chunk
  scatter) reproduces the sequential permutation exactly.
- **But threading is the wrong lever anyway, and the profile says why.**
  70.32% of the entire function lands on the single instruction immediately
  following `mov 0x0(%r13,%rdi,8),%rdx` — the skid of the **dependent random
  load `keys[i]`**. The scatter re-reads each key *through* the permutation, a
  random 8-byte load over an 80 MB working set no prefetcher can predict, and
  the bucket, the `count[bucket]` address, and the store address all depend on
  it. The cost is memory LATENCY on a serialized dependence, not absent
  parallelism. Parallelizing the histogram would have chased 5.18%; a parallel
  scatter would have added memory-level parallelism to a stall that can instead
  be **deleted**.

Note also that `#![forbid(unsafe_code)]` in `fp-columnar` rules out the
conventional disjoint-index parallel scatter; the safe formulations cost either
2x write traffic or C-fold read amplification. That reinforces choosing the
layout fix over the thread fix.

**Entry 2 is under-reported by this benchmark, by roughly 28x.**
`typed_dense_sort_order` takes `&[Scalar]`, so reaching it calls
`Column::values()`, which forces the lazy `LazyAllValidFloat64Vec` backing to
materialize one `Scalar` per row into its `OnceLock<Vec<Scalar>>` and then
matches every enum straight back to the `f64` the column already stores
contiguously. The bench builds the frame once and calls `sort_values` 28 times
(3 warmup + 25 timed), so that `OnceLock` materializes on the FIRST call and is
free for the other 27 — the timed window sees ~1/28 of a cost that a real
single-call `df.sort_values()` pays in full. `Column::as_f64_slice()` already
exposes the contiguous `&[f64]` and would delete the round trip outright. This
is a BENCHMARK-INTEGRITY caveat on entry 2's 2.433%, not a claim that the
single-call cost was measured.

**Result class:** profile/attribution only. The named frontier is now specific:
remove the dependent random key load from the scatter by co-permuting each key
with its index (one 16-byte pair, one destination cache line per element),
which converts pass *k+1*'s key read from random to sequential and is
bit-identical — same visit order, same buckets, same stable counting scatter,
and the pass-skip histogram is invariant under permutation of the key multiset.
A candidate implementing exactly that is written and held OUT of tree at
`scratchpad/radix-payload-candidate.patch`; it is **unbuilt and unmeasured**,
so nothing about its effect is claimed or banked here.

**Blocked measurement, disclosed rather than worked around.** The A/B against
the live pandas arm could not be taken this turn, for two independent
infrastructure reasons, neither of which a valid number can be produced around:
1. **No admissible rch worker for this repo.** `rch exec` refused with
   `critical_pressure=2,insufficient_slots=2,hard_preflight=7`. The tree pins
   `nightly-2026-04-22`; only `hz1` and `hz2` carry it and both are
   disk-critical (hz1 at 11 GB free of 225 GB, from usage unrelated to rch —
   all of `/data/tmp/rch` there is 7.7 GB). The seven disk-rich workers fail
   hard preflight for want of that toolchain. Installing it on `vmi1227854`
   (232 GB free) succeeded and was verified (`rustc 1.97.0-nightly`, `rust-src`
   present), but rch's cached admission did not pick it up, and forcing a
   re-poll means `rch daemon restart`, which would disrupt seven in-flight
   builds belonging to five other repos. Local builds are frozen by policy
   (`/data` at 113 GB, under the 150 GB floor), and `RCH_REQUIRE_REMOTE=1`
   correctly refused every local fallback.
2. **The host cannot pass its own exclusivity gate.** `MAX_HOST_WIDE_BUSY_
   FRACTION` is 0.20 across every online CPU; five to ten of 64 CPUs sat above
   it on every sample, with at least one pinned at 1.00, and load average was
   16.79/45.27/85.46 during the profile itself.

The attribution above is reported anyway because self-time *fractions* are far
more robust to background load than absolute times are, and because the
decisive evidence — a 70.32% single-instruction skid on a named dependent load,
and a 94.33%/5.18% scatter-versus-histogram split — is structural. The exact
percentages should not be quoted as a quiescent-host measurement.
