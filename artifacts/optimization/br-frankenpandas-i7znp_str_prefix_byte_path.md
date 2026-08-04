# br-frankenpandas-i7znp — str.startswith/endswith byte-predicate path

**Status: code landed (`5538b49cb`). PERF CLAIM NOT MADE — row is `NULL_UNDECIDABLE`.**

Agent: MossyMeadow (claude-code / opus-5). Date: 2026-08-04.

## The lever

`StringAccessor::apply_str_bool`'s contiguous (rung-3) path called
`std::str::from_utf8` on EVERY row before handing a `&str` to the predicate. For
a prefix/suffix test that is a full UTF-8 validation pass over the whole byte
buffer performed only to compare a few leading/trailing bytes — on the 1M-row
`item_%010d` benchmark, 16 MB validated to serve 4-byte compares, work
pandas/Arrow never pays.

`apply_str_bytes_bool` keeps the identical chunking and parallel-entry
thresholds but hands the predicate `&[u8]`, so no `from_utf8` runs.
`startswith`/`endswith` route through it; non-contiguous backings delegate to
`apply_str_bool`.

Bit-transparency: `as_utf8_contiguous` gates on `validity.all()` (no nulls to
reinterpret) and the buffer is valid UTF-8 by construction — the skipped
`from_utf8` is exactly the check the old code `.expect()`s can never fail. For
valid-UTF-8 haystack and needle, `starts_with`/`ends_with` ARE byte compares:
UTF-8 is prefix-free and self-synchronizing, so a byte match cannot land
mid-code-point.

## Measurement: ACCEPTED ROW, BUT UNDECIDABLE

Arms built from trees differing ONLY in `crates/fp-frame/src/lib.rs`:

| Arm | ELF SHA-256 | `apply_str_bytes_bool` symbol hits |
|---|---|---|
| reference (HEAD) | `8f50de2e…a844988b` | 0 |
| candidate (+lever) | `9a44e005…be141433` | 21 |

Distinct SHA-256 **and** distinct symbol content, retiring the
`rch-ab-elf-retrieval-trap` false-REJECT class. `pandas_dependency_probe=ready
version=2.2.3 pyarrow_version=24.0.0`; `corrected_null_gate_self_test=pass`.

Host `frankenlibc-test` (vmi1152480), AMD EPYC (with IBPB), 10 physical / 10
logical, `cpu_governors: []` (virtualized, no frequency control).
`str_startswith_arrow @ 1M/float64`, candidate vs **live pandas in the same
invocation**.

| | FP candidate | pandas 2.2.3 (arrow) |
|---|---:|---:|
| p50 | 2773.66 us | 4698.97 us |
| p95 | 7116.37 us | 5555.25 us |
| p99 | 10343.49 us | 5717.21 us |
| CV | **61.28%** | 12.16% |
| threads actually used | **5** | **1** |

Raw ratio 1.694, effect CI95 [1.483, 1.998].

**VERDICT: `NULL_UNDECIDABLE`, `claim_validated: false`. The 1.694 is NOT a
result and must not be quoted as one.**

Gate clauses:

| clause | value |
|---|---|
| `effect_ci_excludes_unity` | true |
| `effect_exceeds_two_x_null_margin` | **false** (0.527 log-effect vs 0.662 required) |
| `null_medians_within_2pct_unity` | **false** |

## Root cause: the FP arm's noise floor is wider than the effect

The A/A null control times the SAME binary against itself; it must land at 1.0.

| arm | null median | within +/-2%? |
|---|---:|---|
| frankenpandas | **0.970** | **NO** |
| pandas | 1.019 | yes |

Over 25 rounds the FP null ratios ranged **0.475 to 1.709** with
`median_ci_95 = [0.718, 1.156]`, giving
`two_x_decidable_interval = [0.516, 1.940]`. The observed 1.694 falls INSIDE
that band — on this hardware no effect of this magnitude is decidable in either
direction, whichever arm is faster.

So the blocker is NOT the exclusivity gate and NOT the fleet's disk pressure
(both were chased and eliminated first — see below). It is that FP's parallel
string kernel spawns 5 threads for a 2.77 ms operation on a virtualized host
with no governor control, and the resulting scheduling jitter swamps the signal.

This corroborates [[parallel-per-call-overhead-ledger]]: `thread::scope` spawn
measured at ~397 us for 8 threads. Against a 2.77 ms median that is a
double-digit-percent tax with very high variance.

**Implication for the next lever, not claimed, to be measured:** the promising
move on this workload may be *avoiding the spawn* — a reusable pool, or raising
`PARALLEL_MIN_BYTES` so 16 MB inputs stay serial — rather than shaving the
`from_utf8` pass. pandas wins its stability here by being single-threaded.

## Premise check

The task framed `str.startswith` as a 0.42x LOSS. That did not reproduce on
either arm: the candidate measured 1.694 and the reference 1.040 against live
pandas, i.e. both at or above parity, not 0.42x. Both rows are undecidable, so
**neither the 0.42x loss nor any improvement is established** — but nothing
observed here supports a 0.42x loss either, and the reference arm carries the
UNMODIFIED code, so that figure cannot be blamed on this lever's absence. The
prior 0.42x should be treated as unverified until reproduced with a passing
null; it may have come from a different host, size, dtype, or pandas backend.

## Gate integrity

The exclusivity gate was NOT modified. Loosening it would have manufactured a
passing row for this agent's own claim (the named `gate self-weakening`
pattern), and the prescribed price for a legitimate gate fix — demonstrate a
moved verdict, then publish the WIN/LOSE split of every row the fix admits —
cannot be met by the change's beneficiary.

Eliminated in order, each with evidence:
1. *Arrow workloads systematically vetoed* — WRONG; the pandas arm cleared its
   post-gate on several attempts.
2. *FP's own worker threads winding down* — WRONG; on the pinned host the trip
   landed on CPU 8 while both arms were confined to CPUs 0-3.
3. *Transient kernel reclaim* (`kswapd0`/`kcompactd0`, every worker at ~96%
   disk) — real, and why 3-arm invocations needing six no-retry
   `require_quiet` samples never cleared: 14/14 exhausted unpinned, 14/14
   exhausted pinned (`taskset -c 0-3 --thread-count 4`). Splitting to two
   invocations (four gates each) cleared on attempt 6.

## Reference arm (added after the entry above was first written)

The reference arm also produced an ACCEPTED row, on attempt 8, and it is
**also `NULL_UNDECIDABLE`**.

| | ELF | FP p50 | FP CV | pandas p50 | raw ratio | FP A/A null |
|---|---|---:|---:|---:|---:|---:|
| candidate | `9a44e005…` | 2773.7 us | 61.3% | 4699.0 us | 1.694 | 0.970 FAIL |
| reference | `8f50de2e…` | 5548.4 us | 43.1% | 5771.6 us | 1.040 | 0.946 FAIL |

The FP A/A null failed on BOTH arms, so neither row is valid and the pair
establishes nothing.

**The tempting number, and why it is not reported as a result.** Candidate p50
2773.7 us against reference p50 5548.4 us looks like a ~2x self-speedup from the
lever. It is not claimable, for three independent reasons, any one of which is
disqualifying:

1. Both rows are `NULL_UNDECIDABLE` — the A/A control failed on each, so neither
   measurement is valid on its own terms.
2. The INCUMBENT moved between the two invocations: pandas' own p50 went
   4699.0 -> 5771.6 us, a 23% drift on the identical workload and binary. The
   two arms were therefore not measured under comparable host conditions. This
   is exactly the weakness the split-invocation design trades away, and it is
   why the three-arm same-invocation form is the preferred shape.
3. Even a clean candidate-vs-reference delta is a **self-speedup**, which this
   campaign classifies as MAINTENANCE, not a win. A win requires a decidable
   vs-incumbent ratio.

Read together the two rows are *consistent with* the lever helping, and equally
consistent with host drift: the reference arm simply ran during a slower period,
which its own inflated pandas baseline shows. Distinguishing those requires a
host where the null lands within +/-2%. Until then, nothing is established.

## Retry predicate

Re-run when EITHER holds:
- a host with **stable CPU frequency and dedicated cores** is available (not a
  shared VPS with `cpu_governors: []`), such that the FP A/A null lands within
  +/-2% of unity — that is the gating condition, not the exclusivity sampling; or
- FP-side variance on this workload is reduced at the source (pool instead of
  per-call `thread::scope`, or a higher parallel threshold), which is itself the
  next lever to try.

Working route for whoever picks this up: build both arms directly over SSH on an
idle worker — rch admission starved ~90 min without ever queuing — at ~7m18s per
release `fp-bench`. Both ELFs and pinned pandas 2.2.3 / pyarrow 24.0.0 are
staged at `/opt/fpbench/` on `vmi1152480` and `vmi1153651`
(`PYTHONPATH=/opt/fpbench/pylibs`).

## What IS established

Correctness only, not speed:
- `str_prefix_suffix_byte_path_matches_str_oracle_on_multibyte` — seeded LCG over
  a 2/3/4-byte code-point alphabet, 14 needles incl. empty and over-long,
  asserted row-for-row against the std `&str` oracle through BOTH the contiguous
  and Scalar backings.
- `str_prefix_suffix_parallel_path_matches_oracle` — 600k rows / 9.6 MiB crosses
  `PARALLEL_MIN_BYTES`, first coverage of the threaded arm and its part-merge.
- `str_prefix_suffix_propagates_nulls_through_byte_path` — a missing row must
  propagate NaN, not read as an empty byte slice.

3/3 green at `origin/main` after subsequent peer commits; `clippy -D warnings`
0 findings in fp-frame; `ubs` exit 0. The change strictly removes work and is
bit-transparent, so it is safe to carry on correctness grounds alone — but it
carries **no measured speedup**.
