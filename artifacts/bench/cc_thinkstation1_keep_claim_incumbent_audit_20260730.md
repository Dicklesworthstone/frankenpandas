# KEEP-claim incumbent-coverage audit — FrankenPandas

- Agent: BlackThrush (cc pane) · Host: `thinkstation1` · Date: 2026-07-30
- Base commit: `fbc5b3f60` · Method: parsed `docs/NEGATIVE_EVIDENCE.md`, `artifacts/perf/SCORECARD.md`, `README.md`, and all 143 parseable `artifacts/bench/*.json`
- Nobody asked for this specific number; it follows the fleet policy that a perf KEEP requires a vs-incumbent ratio.

## The one line

**113 KEEP claims. 113 carry a vs-pandas ratio. 0 were measured with pandas live in the same invocation. 113 do not meet that bar.**

That is the uncomfortable version and it is the one the policy actually asks for.
The flattering version — "how many carry *any* vs-incumbent ratio", which is the
metric frankenfs reported — is **113 / 113, zero unsupported.** Both are true.
They differ because the binding constraint here is not *whether* an incumbent was
measured but *how*.

## Why the two numbers diverge

Every ledger KEEP row lives in a table headed:

```
| Lever (bead) | Workload | pandas | fp | ratio | verdict |
```

so a pandas number is structurally mandatory — you cannot land a row without one.
That is a genuinely good property and it is why FrankenPandas does not have
frankenfs's 67/186 problem.

But essentially every one of those pandas numbers was taken in a **separate
process, and usually on a separate machine**. The recurring shape is:

> FP measured via `rch exec -- cargo run --example bench_survey2 --release` on
> RCH worker `vmi1149989`; **pandas 2.2.3 local best-of-6**.

FP on a Contabo/Hetzner build worker, pandas on the workstation. Different CPU,
different memory system, different load. The ledger's own methodology row (line
118) already says this out loud:

> 🔬 **BLOCKER** — any A/B whose two binaries were built on DIFFERENT workers is
> invalid; only LARGE deltas (≳2×) survive. Marginal wins/losses … are NOT
> trustworthy as currently measured.

So the ledger knows. What the audit adds is the count: **0 of 113** KEEP rows
reference `benches/vs_pandas_harness.py`, the only instrument in this repo that
runs both engines in one invocation with an interleaved A/A null control.

## Public exposure — what a user could actually act on

This is the ranking axis that matters, and here the picture is much better.

| surface | claims | same-invocation | notes |
|---|---|---|---|
| `README.md` | **1** numeric | 0 | see below — the single load-bearing gap |
| `artifacts/perf/SCORECARD.md` (linked from README) | 7 dated sections | **4 / 7** | cites A/A null, median-CI, ELF SHA-256, schema v4 |
| `docs/NEGATIVE_EVIDENCE.md` | 113 KEEP | **0 / 113** | internal ledger, not user-facing |
| `FEATURE_PARITY.md`, `CHANGELOG.md`, `PARITY-COVERAGE.md`, `COVERAGE_MATRIX.md` | 0 | — | carry no speed claims at all |

The README is almost entirely clean: it makes the "exceeds pandas" claim
*conditional on* a category geomean > 1.0 from the harness, and defers the actual
numbers to the scorecard. That is the right structure.

### Ranked conversion queue

Ordered by how load-bearing the claim is where a user will meet it.

| # | claim | where | why it ranks here | convertible? |
|---|---|---|---|---|
| **1** | **"Round 5 · `has_duplicates` OnceLock memoization · 87% faster on groupby benchmark"** | `README.md` L917 table | The **only concrete number in the README**, and it is an **FP-side self-speedup with no incumbent arm at all** — not a weak pandas comparison, *no* pandas comparison. A reader takes "87% faster" as "faster than pandas." | ✅ yes — groupby has full harness coverage |
| 2 | 3 non-same-invocation scorecard sections: RangeIndex Set Ops (2026-06-20), RangeIndex Values (2026-06-22), `RangeIndex::asof` (2026-06-19) | `artifacts/perf/SCORECARD.md` | Public, linked from README, but each already carries a pandas ratio — the gap is provenance, not existence | ⚠️ **partly** — see "cannot convert" below |
| 3 | 113 ledger KEEP rows | `docs/NEGATIVE_EVIDENCE.md` | Internal. A user never sees these; they steer *our* work, and steering on a cross-host ratio is how we waste turns | ✅ mostly — most map to an existing harness category |
| 4 | README "Recent complexity sweep (2026-05)" — ~20 O(n²)→O(n) reductions | `README.md` | Complexity claims, not speed ratios. `O(n²)→O(n)` is a provable structural statement | n/a — not a perf ratio |

## The bigger finding: 283 measurements dropped on a gate we have since disowned

Auditing the harness artifacts surfaced something larger than the ledger gap.
Across 143 parseable artifacts, 562 same-invocation comparison rows exist:

| verdict | rows |
|---|---:|
| **DROPPED_HIGH_CV** | **283** |
| FASTER | 202 |
| SLOWER | 52 |
| INCOMPLETE | 13 |
| NULL_UNDECIDABLE | 6 |
| PARITY | 4 |
| KEEP_FAST_VS_PANDAS | 2 |

**Over half of all properly-measured rows were discarded on coefficient of
variation** — a criterion the project has since explicitly abandoned. The current
harness docstring reads "Gates claims on the null-median bootstrap 95% CI, **never
on cv**", and the scorecard's Lane M re-adjudication states "**CV had no vote**".

When those 16 Lane M rows were actually re-adjudicated under the corrected gate:
**16/16 re-adjudicated, 0 rejected on CV, 14 came back FASTER**, geomean 5.73×.

So ~**267 same-invocation measurement rows, spread over 82 artifact files, are
sitting unadjudicated under a superseded gate**, and the one sample that was
re-run converted at 14/16.

### ⚠️ Correction — these need RE-MEASUREMENT, not re-adjudication

An earlier revision of this audit called this "the cheapest conversion path in
the repo … they need re-adjudication, not re-measurement." **That was wrong, and
checking it is what disproved it.**

Every CV-dropped row was inspected. **0 of 283 carry a `null_control` block or
raw `times_us`** — they store only summary statistics:

```
frankenpandas / pandas keys:
  cv_pct, iterations, mean_us, p50_us, p95_us, p99_us, stddev_us,
  throughput_rows_sec, valid          # no null_control, no times_us
```

The corrected gate needs the interleaved A/A null-control ratios to bootstrap a
median CI and apply the decidability margin. Summary statistics cannot
reconstruct them. These artifacts predate the schema that records them.

This also re-reads the Lane M precedent correctly: its heading says the 16 rows
**"ran on strict-remote worker `vmi1149989` under schema v4"** — they were
*re-run*, not recomputed. The evidence was there; the earlier inference was not.

Consequence for the queue: these 267 rows move from *cheapest* to **gated behind
host quiescence**, which is currently the most expensive constraint in the repo
(a 60-second probe found 0/60 clear samples; an 8-minute watch never caught a
15-second lull). The 14/16 conversion rate from Lane M still makes them the
highest-*expected-value* target once a quiet host exists — but the cost estimate
was wrong by the full price of re-running 283 same-invocation benchmarks.

## What cannot be converted, and why that is a different problem

Being explicit, because "nobody got round to it" and "there is nothing to compare
against" are different failures:

- **`RangeIndex` set-ops / values / `asof` (queue #2).** pandas has `RangeIndex`,
  so an incumbent arm *is* constructible — but `benches/vs_pandas_harness.py` has
  **no `indexing`-category FP arm wired for these**; `fp-bench` reports them
  INCOMPLETE. This is "no arm exists yet", i.e. real work, not a re-run.
- **The four O(n²)→O(n) complexity claims.** Not convertible *and should not be* —
  a complexity reduction is proved by construction, and demanding a pandas ratio
  for it would be a category error.
- **Anything in the ledger measured against an FP-ORIG baseline only** (the
  "FP-side Nx vs ORIG" phrasing). These are maintenance records, not campaign
  wins, and the honest fix is to relabel them as such rather than to manufacture
  an incumbent number retroactively.
- **The 283 CV-dropped rows** cannot be converted from stored data at all — see
  the correction above. Not "nobody got round to it": the bytes needed to
  re-decide them were never written.

## Not done in this turn

Per instruction, **nothing was deleted, weakened, or relabelled** — this is
inventory only. The README's "87% faster" line is still there and still
unsupported; it is queue item #1 precisely so that the fix is a decision rather
than a silent edit.
