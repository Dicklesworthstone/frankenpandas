# Incumbent-ratio coverage — FrankenPandas @ `e7eb7b5a0`

- Agent: BlackThrush · Host: `thinkstation1` · Date: 2026-07-31
- Method: reproducible parser, not a hand count —
  `artifacts/bench/cc_blackthrush_recount_keep_claims.py` (rules stated inline,
  re-runnable, disputable; `python3 artifacts/bench/cc_blackthrush_recount_keep_claims.py`)
- Supersedes the hand audit in `cc_thinkstation1_keep_claim_incumbent_audit_20260730.md`
  on the ledger count; that audit's artifact census is reproduced exactly.

## The three numbers

| | |
|---|---:|
| **A. total KEEP perf claims held** | **104** |
| **B. carrying a vs-incumbent ratio measured with pandas LIVE in the same invocation** | **0** |
| **C. not measured that way** | **104** |

Breakdown of C:

| bucket | rows | meaning |
|---|---:|---|
| `CROSS_PROCESS_VS_PANDAS` | 97 | a real pandas number exists, taken in a **separate process and usually a separate machine** |
| `FP_SIDE_NULL_GATED` | 7 | corrected A/A null gate applied properly — but the comparator is **FrankenPandas itself**, not pandas |
| `NO_INCUMBENT` | 0 | — |

Excluded from A: 32 commit-replay rows (`| # | commit | … | KEEP |`), where KEEP
means "this commit replays clean", not "this lever beats pandas".

### On frankenfs's metric

frankenfs reported 67 of 186 claims with **no ratio at all**. On that metric
FrankenPandas is **104 / 104 with zero unsupported** — every ledger KEEP row sits
in a table headed `| Lever | Workload | pandas | fp | ratio | verdict |`, so a
pandas number is structurally mandatory. Both numbers are honest. They diverge on
*how* the incumbent was measured, not *whether*.

The uncomfortable number is B = 0, and B is the one the policy asks for.

## Root cause of B = 0 — found this session, and it is not negligence

**No worker in the rch fleet had pandas installed. Not one.** Probed all 12 on
2026-07-31:

```
hz1 hz2 ovh-a ovh-b vmi1149989 vmi1152480 vmi1153651 vmi1156319
vmi1167313 vmi1227854 vmi1264463 w10   →   pandas=NONE  (12/12)
```

FrankenPandas builds and benchmarks on those workers. pandas only ever existed on
the workstation. So a same-invocation A/B was **physically impossible anywhere the
FP arm ran**, and on the one host where pandas did exist, the fail-closed
host-exclusivity gate (every online CPU ≤ 20% busy) is unsatisfiable under swarm
load. B = 0 was a structural property of the fleet, not an oversight in any row.

Fixed today: pandas 2.2.3 + pyarrow installed to `/root/fpbench-libs` on
`vmi1149989` (pip `--target`, no venv, no system packages touched), plus the
pinned `nightly-2026-04-22` toolchain synced there. It is now a viable
head-to-head host: quiet (load 0.44), 10 physical cores, no SMT, avx2+bmi2+fma.

## Public exposure — what a user could actually act on

| surface | claims | same-invocation |
|---|---:|---:|
| `README.md` | **1** numeric | 0 — and it has **no incumbent arm at all** |
| `artifacts/perf/SCORECARD.md` (linked from README) | 10 dated sections | **4** |
| `docs/NEGATIVE_EVIDENCE.md` | 104 KEEP | **0** |
| `FEATURE_PARITY.md`, `CHANGELOG.md`, `PARITY-COVERAGE.md`, `COVERAGE_MATRIX.md` | 0 | — |

The README is otherwise well built: it makes the "exceeds pandas" claim
*conditional on* a harness category geomean and defers numbers to the scorecard.

## CANNOT convert vs NOT YET measured — different problems

**Cannot convert (structural — no incumbent arm exists, or the ask is a category error):**

1. **`RangeIndex` set-ops / values / `asof`.** pandas has `RangeIndex`, so an
   incumbent arm is *constructible*, but `vs_pandas_harness.py` has no `indexing`
   FP arm wired for these — `fp-bench` reports INCOMPLETE. This is unwritten
   harness code, not a re-run.
2. **The ~20 O(n²)→O(n) complexity claims** (README "Recent complexity sweep").
   A complexity reduction is proved by construction. Demanding a pandas ratio for
   it is a category error, and it should stay unconverted.
3. **Ledger rows measured only against an FP-ORIG baseline.** These are
   maintenance records, not campaign wins. The honest fix is to relabel them, not
   to retrofit an incumbent.
4. **The 283 `DROPPED_HIGH_CV` artifact rows.** 0 of 283 carry a `null_control`
   block or raw `times_us` — only summary statistics. The corrected gate needs the
   interleaved A/A ratios to bootstrap a median CI; summary stats cannot
   reconstruct them. **The bytes needed to re-decide them were never written.**
   Re-measurement, not re-adjudication.

**Not yet measured (a re-run away, now that a pandas-equipped quiet host exists):**

- The 97 `CROSS_PROCESS_VS_PANDAS` ledger rows. Each maps to an existing harness
  category; each needs one same-invocation re-run.
- The README "87% faster" claim — no incumbent arm, but groupby has full harness
  coverage. Converted in this session; see the companion artifact.

## Artifact census (unchanged from the prior audit; reproduced exactly)

143 files, 562 comparison rows, 82 carrying a `null_control` block.

| verdict | rows |
|---|---:|
| DROPPED_HIGH_CV | 283 |
| FASTER | 202 |
| SLOWER | 52 |
| INCOMPLETE | 13 |
| NULL_UNDECIDABLE | 6 |
| PARITY | 4 |
| KEEP_FAST_VS_PANDAS | 2 |

## Why this count differs from yesterday's 113

Yesterday's hand audit said 113; this parser says 104 for the ledger. Two parse
bugs in the hand pass, both found by writing the count as code:

1. Verdict cells are emoji/bold decorated (`✅ KEEP — …`), so a bare
   `startswith("KEEP")` drops the majority of rows.
2. Long ledger tables are interrupted by blank lines. Treating "first row of a
   `|` run" as the header makes continuation blocks' **data** rows look like
   headers, silently reclassifying whole tables.

The headline is unchanged either way: **0 same-invocation**.
