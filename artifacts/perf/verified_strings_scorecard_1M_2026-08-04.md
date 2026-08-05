# Verified strings scorecard @ 1M — shipping FP vs live pandas 2.2.3

Agent: MossyMeadow (claude-code / opus-5). Date: 2026-08-04.
Host `frankenlibc-test` (vmi1152480, drained from rch), AMD EPYC, 10 physical /
10 logical, `cpu_governors: []`. Shipping FP ELF `a41869d5814b5334…` (HEAD
`a4a08fbd5` behaviour). Live pandas 2.2.3 + pyarrow 24.0.0 **in the same
invocation** as each FP arm. ONE workload per invocation — a single gate failure
aborts a whole invocation, so batching makes acceptance impossible.

**Why this exists.** The campaign's standing loss figures are unreliable: the
"str.startswith 0.42x LOSS" was refuted by a decidable row
([[br-frankenpandas-csapr_string_kernel_spawn_variance]]). This re-derives real
verdicts so effort goes at losses that actually exist.

## Rows

| workload | verdict | ratio | FP p50 | pandas p50 | FP CV | FP null | pandas null | FP threads |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| `str_groupby_sum_arrow` | **FASTER** | **2.827x** | 7708.2 us | 21793.2 us | 11.4% | 1.0124 OK | 0.9961 OK | **1** |
| `str_contains_arrow` | undecidable | 8.393x | 2520.1 us | 21149.8 us | 26.8% | **0.9665 FAIL** | 0.9995 OK | 8 |
| `str_len` | undecidable | 2.989x | 3552.6 us | 10618.4 us | 19.3% | **0.9763 FAIL** | 0.9890 OK | 8 |
| `str_value_counts_arrow` | undecidable | 2.035x | 8213.8 us | 16718.5 us | 12.3% | 0.9938 OK | **0.9472 FAIL** | 1 |
| `str_sort_arrow` | undecidable | 1.498x | 37435.9 us | 56092.5 us | 43.5% | **0.9310 FAIL** | **1.0323 FAIL** | 10 |
| `str_upper` | no row | — | — | — | — | — | — | — |

`str_upper` exhausted 14 attempts without clearing the exclusivity gate.

Every row cleared `effect_ci_excludes_unity` AND `effect_exceeds_two_x_null_margin`.
The **only** clause that ever failed is `null_medians_within_2pct_unity`.

## Findings

**1. ZERO measured losses.** All five ratios are above 1.0, from 1.498x to
8.393x, and every effect CI excludes unity. There is no verified string-op loss
at 1M to attack. The campaign's "attack your worst measured LOSSES first"
directive currently has no verified target in this category — the standing loss
figures should be re-derived before anyone spends effort on them.

**2. One decidable win: `str_groupby_sum_arrow` at 2.827x FASTER** (CI95
[2.620, 2.975]), both nulls passing. This is the second validated row of the
campaign after csapr's 1.320x.

**3. FP thread count predicts FP null failure — 5/5.**

| FP threads | FP null verdict |
|---|---|
| 1 (`groupby_sum`, `value_counts`) | **PASS** (1.0124, 0.9938) |
| 8-10 (`contains`, `len`, `sort`) | **FAIL** (0.9665, 0.9763, 0.9310) |

Independent confirmation, across five workloads, of the csapr result that the
per-call `thread::scope` spawn — not the exclusivity gate, not fleet disk
pressure — is what makes FP rows undecidable. It also sharpens the payoff of
[[br-frankenpandas-vrjrf]] (pool, needs no unsafe): it would move roughly
*three of five* string rows from undecidable to decidable, not just one.

**4. The noise is not exclusively FP-side.** `str_value_counts_arrow` is
undecidable despite FP's null passing at 0.9938 — it was **pandas'** null that
failed (0.9472). `str_sort_arrow` failed on both. So single-threaded pandas is
not immune, and some residual host jitter is common-mode. A pool would fix FP's
contribution but would not by itself make every row decidable.

## What is NOT claimed

The four undecidable rows are **not** wins. Their ratios (1.498x-8.393x) and
CIs are suggestive and every one excludes unity, but a failed A/A null
invalidates the row per the campaign contract, and quoting those numbers would
be proof-class inflation. They are recorded as *unresolved*, not as results.

The gate was not modified.

## Provenance

Raw accepted rows: `artifacts/bench/sc_<workload>_1M.json`, each carrying both
executables' SHA-256 self-reported from inside the measuring process, the
host-wide exclusivity observations, and the 25-round A/A null series.
Acceptance attempts: `groupby_sum` 1, `value_counts` 3, `len` 5, `sort` 5,
`contains` 9, `upper` exhausted at 14.
