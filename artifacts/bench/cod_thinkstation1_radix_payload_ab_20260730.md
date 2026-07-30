# thinkstation1 — `radix_argsort_u64` co-permuted key payload: 2.16–2.39x on the 10M sort, bit-identical

Follow-on to `cod_thinkstation1_sort_serial_residual_profile_20260729.md`, which
profiled the serial residual and named this exact lever. That profile found the
residual is 87.3% of wall, `radix_argsort_u64` is 94.8% of the residual, its
scatter loop is 94.33% of the kernel, and **70.32% of the whole function is the
skid of one dependent random load** — `keys[i]`, read *through* the permutation
over an 80 MB working set, with the bucket, the `count[bucket]` address and the
store address all dependent on it.

## The change

Carry each key next to its index and permute the pair, so pass *k+1* reads the
keys pass *k* already placed — sequentially — instead of chasing them randomly
through the permutation. One 16-byte tuple, so the scatter still touches a
single destination cache line per element rather than two 8-byte streams.

`crates/fp-columnar/src/lib.rs`, `radix_argsort_u64`: `idx: Vec<usize>` plus
random `keys[i]` becomes `cur: Vec<(u64, usize)>` permuted as a unit.

**This is a work deletion, not a parallelization.** The profile is why: the
histogram loop that threads *could* have attacked is only 5.18% of the kernel,
and `#![forbid(unsafe_code)]` in fp-columnar rules out the conventional
disjoint-index parallel scatter anyway (the safe formulations cost either 2x
write traffic or C-fold read amplification). The 8 LSD passes remain
**inherently sequential** with respect to each other; that dependence is
untouched and unbreakable without changing algorithm.

## Bit-identity

- Visit order unchanged (`cur` is in `idx` order), each element's bucket comes
  from the same key, stable counting scatter unchanged → identical permutation.
- Pass-skip test unchanged: `cur` always holds a permutation of the same key
  multiset as `keys`, and a histogram is invariant under permutation, so `count`
  and every prefix offset match what the old code computed off `keys` directly.
- **Measured:** the harness output checksum was `4957dea0fe3e2ed1` on *every*
  arm of *every* round of both runs — 18 invocations, both ELFs, identical.
- `cargo test -p fp-columnar --profile release-perf`: **592 passed, 0 failed**,
  57 ignored.
- `cargo test -p fp-frame --profile release-perf`: **3207 passed, 0 failed**,
  26 ignored — the consumer crate that owns `sort_values`, `nlargest`/`nsmallest`,
  `rank` and every other `radix_argsort_u64` caller. **3799 tests green in total.**

## Provenance

Host `thinkstation1`, AMD Ryzen Threadripper PRO 5975WX, 1 socket, **32 physical
cores / 64 logical threads**, 2 threads per core, 231,691,894,784 bytes RAM, 1
NUMA node, kernel `6.17.0-35-generic`, governor `powersave`, SMT and boost on.
Runtime-detected ISA `sse2,avx,avx2,fma,bmi1,bmi2,aes,vaes`; `avx512f` absent.

Both arms were built **locally on this host from the same pinned toolchain**
(`nightly-2026-04-22`) so the ONLY difference between them is this patch — no
cross-builder confound. Route 2 was used under PART B guardrails (`/data` at
458G, well over the 150G floor; one reused repo target dir; final measurement
binary only; `force_local` never set). PART A's
`--base 6774e9a37 --clean-overlay --overlay-path crates/fp-columnar/src/lib.rs`
was attempted first and refused: `no admissible workers
(critical_pressure=2,insufficient_slots=2,hard_preflight=7)`.

| arm | ELF SHA-256 | bytes |
|---|---|---:|
| baseline (`6774e9a37` source) | `13f630df76b91078a6dafb66476d2e2e9b55e2edf5213206c02bad077c9e950a` | 73,731,872 |
| candidate | `505520c4903f68ae6f8d695d82d197f090a51a76afb846f38484e480362c5021` | 73,738,256 |

Each invocation self-reported its own SHA-256 from inside the process and the
harness aborts on any mismatch against the externally computed hash.
**Observed** operation threads were `10` in every invocation of both runs
(against `runtime_available_parallelism=64`) — the ncols plateau of `6774e9a37`,
unchanged by this patch, as expected: this lever does not add threads.

## Result — `dataframe_ops/sort_values_single`, 10M rows x 10 Float64 columns

Interleaved baseline/candidate, alternating within every round. 50 timed samples
per invocation, pooled per arm. Bootstrap 20,000 resamples, seed `0xC0DFEED`.

| run | affinity | n/arm | baseline p50 | candidate p50 | ratio | CI95 | verdict |
|---|---|---:|---:|---:|---:|---|---|
| 1 | `0-63`, unpinned, all 64 online | 300 | 1378.446 ms | 576.504 ms | **2.3910x** | [2.3581, 2.4059] | FASTER |
| 2 (replication) | `taskset -c 5,6,7,24,25,26,27,28,29,31,33,34,36,56,57,58` (16 quietest, max 2.0% busy) | 150 | 1374.638 ms | 636.969 ms | **2.1581x** | [2.1049, 2.2131] | FASTER |

Per-round ratios, run 2: 1.985, 2.172, 2.320. CV is **provenance only and never
gates**: run 1 baseline 0.0612, candidate 0.0509; run 2 baseline 0.1373, candidate 0.0739.

The two runs differ by ~10% in magnitude, which is expected and not a stability
problem — run 2 deliberately changed the affinity from 64 to 16 CPUs, which
changes the ten-worker gather's placement, not the kernel under test. The
**verdict is identical** under both conditions.

## Gate audit (fleet primitive transfer, frankenlibc A/A straddle defect)

**Audited: this gate does not exhibit the defect, and was left alone.**

- It computes **no confidence interval on the null** and has **no "null CI must
  include 1.0" straddle clause**. The failure mode described — a tighter null
  vetoing its own row, precision coupled to verdict — cannot arise here.
- Its margin is `max|x - 1.0|` over raw A/A ratios, a worst-case bound. That is
  strictly conservative: it *widens* the reject band, so it can only ever
  suppress a real effect, never manufacture one.
- **Verdict stability, per the required evidence standard:** the same two ELFs
  were re-run on a different, quieter core subset. Verdict stayed FASTER; the
  effect reproduced. A reproducible effect with a *stable* verdict is the
  negative control for this defect.
- **Corrected three-clause rule evaluated independently, run 2:** effect CI
  excludes 1.0 (true); effect deviation 1.1581 exceeds 2x null half-width
  0.0305 (true, by ~38x); **null median 0.999542, deviation 0.000458 — well
  inside the 2% bound** (true). `decidable = True`,
  **`agrees_with_local_gate = True`.**
- **Integrity check:** applying the corrected rule changed **nothing** — it
  agreed with the local gate in both runs. No loss became a win, because no
  verdict moved at all. Had correcting the gate flipped anything here, this
  result would be reported as suspect.
- Honest note on the *other* direction: this gate is over-conservative relative
  to the corrected rule (run 2 band `[0.130, 1.870]` from the max-deviation
  margin, versus `2 x half-width = 0.0305`). A small effect could be vetoed here
  that the corrected rule would decide. That errs toward suppressing findings,
  which is the safe direction, and the present effect clears both by a wide
  margin — so per instruction the gate is not being changed on the strength of
  this row.

Null telemetry, run 2 (reported, never a veto): n=150, median 0.999542,
CI95 [0.988560, 1.019072], half-width 0.015256, max abs deviation 0.434685.

## What is NOT claimed: the live pandas arm

**No pandas ratio is claimed.** The above is an FP-vs-FP self-relative speedup
of the sort operation, not a head-to-head. The gated pandas arm was attempted
and **refused, fail-closed**, by the project's own adjudicator:

```
ERROR: host-wide benchmark exclusivity requires every online CPU to remain at
or below 20.0% busy; phase=invocation_preflight missing=[]
busy=[9,14,16,19,22,23,33,40,41,47,49,51,53,54,55]
```

Fifteen of 64 CPUs were over the ceiling, from peer agents on this shared host
that are not mine to evict. The refusal is the correct behavior and was not
worked around. The harness did emit its full identity first, recorded here so
the pending run is reproducible: pandas artifact
`c10b13e6b6bec9a38bef8a24062c35f84c343a67973eec708b0c523302a5845f`
(70,681,559 bytes, 2,922 files), pyarrow
`cc070ad58b3c3e9e5e2a79b07883ddc705a74d11e30883688ee78425a33f3114`, Python ELF
`efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`, harness
source `eea8716f3b0a3815ed6feddb58e2a1af395c40ea783341534edfaebe0a4589cf`,
fingerprint git SHA `0b7d4e691`.

For reference only, and explicitly NOT a claim: the banked trj sweep put 10M
`sort_values_single` at 0.769x–0.921x **SLOWER** than pandas. A 2.16–2.39x
self-speedup of that operation would be expected to invert that sign. That
expectation is **unverified** — it mixes hosts and invocations, which this
ledger forbids, and it must be confirmed by a single gated invocation on an
exclusive host before any competitive claim is made.

## Remaining frontier

1. **The pandas arm**, on an exclusive host. This is the only missing piece.
2. The 8 LSD passes stay inherently sequential; the scatter is now
   sequential-read plus random-write, i.e. shifted from latency-bound toward
   bandwidth-bound. A parallel chunked stable scatter becomes the natural next
   lever *at that point*, but it needs a safe-Rust formulation that does not pay
   2x write traffic, and the df_abs row-partition reject is the warning that
   added bandwidth pressure on this box can lose.
3. `typed_dense_sort_order`'s `Scalar` round trip, still un-removed and still
   under-reported ~28x by this benchmark because the `OnceLock<Vec<Scalar>>`
   materializes once while the bench sorts the same frame 28 times.
   `Column::as_f64_slice()` would delete it outright for a single-call
   `df.sort_values()`.
