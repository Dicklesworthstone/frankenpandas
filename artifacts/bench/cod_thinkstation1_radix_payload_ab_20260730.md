# `radix_argsort_u64` co-permuted key payload — 2.16–2.39x maintenance self-speedup; 5.110x live-pandas win

Follow-on to `cod_thinkstation1_sort_serial_residual_profile_20260729.md`, which
profiled the serial residual and named this exact lever. That profile found the
residual is 87.3% of wall, `radix_argsort_u64` is 94.8% of the residual, its
scatter loop is 94.33% of the kernel, and **70.32% of the whole function is the
skid of one dependent random load** — `keys[i]`, read *through* the permutation
over an 80 MB working set, with the bucket, the `count[bucket]` address and the
store address all dependent on it.

**Campaign classification:** the 2.16–2.39x FP-vs-FP result is a
`SELF-speedup`, so it is **MAINTENANCE**, not a campaign win. The competitive
claim comes only from the later same-invocation live-pandas rows: **5.110x
FASTER**, replicated at **5.900x FASTER**.

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

## Maintenance self-speedup provenance

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

## Maintenance self-speedup result — `dataframe_ops/sort_values_single`, 10M rows x 10 Float64 columns

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

## Live-pandas conversion — incumbent win

**Campaign result class:** `incumbent-win`. The chooser-facing subtype is
`whole-operation-win`: each timed sample is the complete 10M-row,
ten-Float64-column `df.sort_values("col_0")` operation. Fixture population is
outside timing; the sort result is fully constructed inside timing. Pandas
2.2.3 and the exact FrankenPandas ELF ran side by side in the **same
invocation** with their own alternating A/A controls.

**Legacy incumbent arm (same invocation):**
name=pandas version=2.2.3
artifact_sha256=c10b13e6b6bec9a38bef8a24062c35f84c343a67973eec708b0c523302a5845f
invocation_id=vs-pandas-20260730T062823.212746Z-pid1749690
measured_ratio=5.110x

### Strict build and artifact provenance

The measured FrankenPandas ELF was built only through the required strict
remote route, from exact base `9d21f64314042f31d4e09327390e403f87e068d1`:

```text
RCH_REQUIRE_REMOTE=1 RCH_NO_SELF_HEALING=1 \
rch exec --no-self-healing \
  --base 9d21f64314042f31d4e09327390e403f87e068d1 \
  --clean-overlay --no-overlay -- \
  cargo build -p fp-bench --profile release-perf
```

Build worker `vmi1153651` produced the exact measured artifact.

**Executing ELF SHA-256 (self-reported by process):**
`bench_elf_sha256=088ce5728aedce97a589f927032dfb16468a43797d65d7e6b71f8367df1a6ecc
(73771288 bytes)
/data/tmp/cod-cyanlynx-frankenpandas-radix-vs-pandas-20260730/target/release-perf/fp-bench`.

The copied execution artifact matched that in-process identity byte-for-byte.
No locally built ELF contributes to the live-pandas rows.

Both admitted invocations executed on `vmi1149989`, AMD EPYC Processor (with
IBPB), **10 physical / 10 logical cores**, SMT inactive, CPUs `0-9`,
63,196,901,376 bytes RAM, one NUMA node, kernel `6.17.0-40-generic`. Host ISA:
SSE2, AVX, AVX2, FMA, BMI1, BMI2, AES; VAES and AVX-512F absent. FrankenPandas
reported scalar, SSE2, AVX2, FMA, BMI2. The requested affinity cap was ten, but
the evidence records **actual observed operation threads**: FrankenPandas
`10`, pandas `1`, in both invocations.

The live incumbent was pandas 2.2.3 distribution SHA-256
`c10b13e6b6bec9a38bef8a24062c35f84c343a67973eec708b0c523302a5845f`
(70,681,559 bytes, 2,922 files). Its Python 3.13 ELF was
`efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e`
(6,894,448 bytes). Harness source SHA-256 was
`eea8716f3b0a3815ed6feddb58e2a1af395c40ea783341534edfaebe0a4589cf`;
the optional pyarrow distribution was
`cc070ad58b3c3e9e5e2a79b07883ddc705a74d11e30883688ee78425a33f3114`.

### Same-invocation result

Each engine contributed 50 whole-operation samples from 25 alternating A/A
pairs. The effect CI below is an independent 20,000-resample bootstrap of
`median(pandas) / median(FrankenPandas)`, seed `0xF2A_2026_0725`. The 2x-null
column is the upper ratio implied by twice the larger engine null log-CI
half-width. The effect CI clears that complete interval in both rows.

| row | invocation | FP p50 / p95 / p99 | pandas p50 / p95 / p99 | pandas / FP | effect CI95 | 2x-null upper | verdict |
|---|---|---:|---:|---:|---|---:|---|
| primary | `vs-pandas-20260730T062823.212746Z-pid1749690` | 547.572 / 1,160.802 / 1,745.289 ms | 2,798.268 / 3,238.291 / 3,440.199 ms | **5.110x** | **[4.543, 5.737]** | 1.454x | **FASTER** |
| replication | `vs-pandas-20260730T063725.213861Z-pid1759655` | 493.561 / 1,067.227 / 1,352.823 ms | 2,911.805 / 3,345.684 / 3,650.424 ms | **5.900x** | **[5.408, 6.309]** | 1.976x | **FASTER** |

The conservative primary ratio is the headline. Point log-effect / required
2x-null log-effect was `1.63126103 / 0.37418856` in the primary and
`1.77488231 / 0.68106454` in the replication. The direction and verdict
replicate; the ratio is not presented as more precise than those two rows.

**A/A null control (same invocation):** 25 alternating pairs per engine and
row. Primary FrankenPandas/pandas bootstrap-median 95% CIs were
`[0.829366,1.186423]`/`[0.968692,1.038915]`; replication CIs were
`[0.711392,1.218157]`/`[0.981589,1.028369]`.

**Median-CI decision:** the primary median log effect 1.63126103 cleared the
required two-times-null log-effect threshold 0.37418856, and its independent
effect CI was `[4.543053,5.737131]`; the replication median log effect
1.77488231 cleared 0.68106454, with effect CI
`[5.407723,6.309032]`. Both decisions are `FASTER`.

**CV role:** provenance only; CV had no vote. FP/pandas CV was 49.37%/8.33%
in the primary and 39.94%/8.65% in the replication. The high FP dispersion
widens the bootstrap intervals and the A/A margin; both effects still clear
by multiple factors.

All six all-CPU exclusivity observations cleared in each admitted invocation.
Maximum observed busy fraction was 16.67% in the primary and 19.00% in the
replication, below the unchanged 20% ceiling. Twenty-four earlier local or
remote invocations failed closed at a pre/post checkpoint and wrote no result;
none contributes a sample or ratio.

Raw artifacts:

- `artifacts/bench/cod_vmi1149989_radix_payload_vs_pandas_10m_20260730.json`
  — SHA-256
  `58831b9ed14a34a3502e56b834c612c0bb4ee41bf5442dcb2e1947821808ea77`.
- `artifacts/bench/cod_vmi1149989_radix_payload_vs_pandas_10m_20260730_replication.json`
  — SHA-256
  `ef1ff9728a75778cdfa390029c62043a70648c2ecd4fec512a86a4ec59ea87cb`.

### CI-straddle defect audit — reported, no gate change

**Finding: neither gate exhibits the reported precision defect, and neither
gate was changed.**

- The maintenance A/B harness has no null confidence-interval straddle veto.
  It gates the effect bootstrap CI against twice a conservative
  `max|A/A - 1|` raw-null bound.
- The canonical incumbent harness reports each null bootstrap CI, but
  `compute_comparison` has no `nulls_hold` and never requires a null CI to
  include 1.0. It compares the claim log-effect with twice the larger null
  log-CI half-width; CV is explicitly provenance only.
- Two fully admitted same-ELF invocations retained the same `FASTER` verdict.
  The verdict therefore did not move randomly with null-CI precision, which is
  the transferred defect's required symptom.
- The missing effect-CI clause was evaluated independently rather than added to
  the harness: [4.543, 5.737] and [5.408, 6.309] both exclude 1.0 and both lie
  wholly above their 2x-null intervals.

For completeness, applying the note's alternative three-clause rule as a
**diagnostic** would mark both incumbent rows undecidable on its extra
null-median-within-2% clause: FP null medians were 1.028251 and 1.033124
(2.825% and 3.312% from unity); pandas null medians were 1.005420 and 1.008509.
The effect-CI and 2x-margin clauses pass in both rows. Because the actual gates
contain no CI-straddle defect, the verdict is stable, and the instruction was
to report rather than change the gate, that stricter clause is disclosed but
not retrofitted as a new veto. These rows must not be reused as proof for a
future campaign that explicitly adopts the alternative three-clause rule.

**Decision: KEEP** the source optimization as maintenance and keep the
competitive 5.110x whole-operation claim for this exact 10M×10 Float64 sort
shape on the recorded host.

## Remaining frontier

1. The 8 LSD passes stay inherently sequential; the scatter is now
   sequential-read plus random-write, i.e. shifted from latency-bound toward
   bandwidth-bound. A parallel chunked stable scatter becomes the natural next
   lever *at that point*, but it needs a safe-Rust formulation that does not pay
   2x write traffic, and the df_abs row-partition reject is the warning that
   added bandwidth pressure on this box can lose.
2. `typed_dense_sort_order`'s `Scalar` round trip, still un-removed and still
   under-reported ~28x by this benchmark because the `OnceLock<Vec<Scalar>>`
   materializes once while the bench sorts the same frame 28 times.
   `Column::as_f64_slice()` would delete it outright for a single-call
   `df.sort_values()`.

**Concrete retry predicate:** do not rerun this exact shape to seek a larger
ratio. Reopen only for a materially different chooser question—another dtype,
null/NaN semantics, a different column width, thread-normalized execution, or
another hardware class—or after the incumbent/harness artifact changes. Any
new claim must again use a strict RCH-built exact ELF, live pandas in the same
invocation, complete host/actual-thread/artifact provenance, an effect
bootstrap CI that clears the 2x null margin, and CV as provenance only.
