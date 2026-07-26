# Ledger Resurrection Audit — frankenpandas

**Campaign:** FrankenSuite Performance Domination, 2026-07-25 · Meta-Lever #1
**Lanes:** cc / STRUCTURAL (QuietHarbor) + cod / HARNESS+FRONTIER (CrimsonGate)
**Beads:** br-frankenpandas-uza04 + br-frankenpandas-elptw
**Source ledger audited snapshot:** `docs/NEGATIVE_EVIDENCE.md` (19,011 lines, 843 `###` entries);
the live shared ledger reached 19,177 lines / 846 entries while the corrected re-runs executed.

**Audit tool:** read-only classifier, scratchpad-only (no repo file processed or rewritten)

---

## 1. Headline

| Metric | Count |
|---|---:|
| REJECT-bearing entries audited | 256 |
| Flagged VOID by automated criteria | 31 |
| **Hand-confirmed genuine VOID lever-rejects** | **16** |
| Classifier false positives (WIN / meta / audit rows quoting "REJECT"/"REVERTED") | 15 |
| WEAK (provenance gap only — decidable number, missing sha and/or null control) | 177 |
| STANDS (correctly decided) | 48 |
| Already resurrected in-repo, yield banked | **2 lanes** |
| Corrected top-five outcomes this round | **2 KEEP / 1 REJECT / 1 NULL_UNDECIDABLE / 1 MOOT** |
| Harness contract adopted | **ELF SHA-256 + same-invocation A/A + median-CI gate** |

**The single most important finding is not a number in the table above.**
frankenpandas **independently invented and executed the §1 method on 2026-07-10**, two weeks before
the campaign named it — and it produced this repo's largest win lane. See §3.

---

## 2. Method

Each `###` entry containing a REJECT/REVERT/NOSHIP/zero-gain marker was classified against the
campaign's VOID criteria:

| ID | Criterion |
|---|---|
| V1 | Claimed ratio lies **inside** the A/A null floor **and no null floor was ever measured** |
| V2 | No null control recorded at all |
| V3 | Target frame self-time ~0% in the profile the bench actually exercised |
| V4 | `cv < 5%` was the **deciding** gate on a shared, unpinnable rch worker |
| V5 | No binary sha256 recorded while concurrent agents were editing the crate |

**A refinement this repo's ledger forced.** The first classifier pass flagged 41 VOID and was *wrong*.
It marked entries VOID merely for landing inside the null band — including the 2026-07-11 rolling-median
row, which is a **model** reject: it ran two Fenwick A/A controls (spanning 1.0816%), proved 1M-row
bit-identical output *before* timing, found the candidate 0.2576% from the control midpoint and
*between the two controls*, and recorded the worker id. That entry is correctly decided.

> **A ratio inside the null floor is VOID only when the floor was never measured.
> When a real A/A control was run and the candidate fell inside it, the reject is VALID.**

V1 and V4 were re-gated on `not has_null_control`. VOID requires at least one *decisive* reason
(V1/V3/V4); V2/V5 alone are provenance gaps on an otherwise decidable number and yield WEAK.

**Honest limitation.** Automated classification of prose has a ~48% false-positive rate at the VOID
tier here (15 of 31), because celebratory WIN entries and meta-audit entries quote the word REJECT
heavily. Every VOID row in §4 was therefore hand-read before being counted. The 177 WEAK rows were
**not** individually hand-read — that count is an upper bound on provenance debt, not a claim that
177 levers are recoverable.

---

## 3. Prior in-repo resurrections (yield already banked)

This is the campaign's method, run here before the campaign existed, twice — both times it paid.

### 3.1 The four transpose/to_dict rejects → the lazy-transpose-view lane

`docs/NEGATIVE_EVIDENCE.md:11538` (2026-07-10, cc_fp) — *"LEDGER-INTEGRITY AUDIT of the four
transpose/to_dict REJECT rows — 2 INVALID (REOPENED), 1 INVALID-AS-MEASURED but MOOT, 1 VALID"* —
and its strict follow-up at `:11636` (cod_fp), which hardened the verdict to **4/4 INVALID as reject
evidence** on the grounds that *no candidate profile carried non-zero SELF time*. That is criterion
V3, discovered independently.

The same pass found (`:11834`) that the `df_transpose` bench **never crossed the materialization
boundary** — the benchmark did not execute the code under test — and added `df_transpose_materialize`
before permitting any lever. That is the frankenmermaid 0.000%-self-time failure mode, found here
independently.

**Yield:** the reopened lane became the feature-gated `DataFrameTransposeView`, then the default
(`538c03d6f`), and shipped, per bead `br-frankenpandas-l4vzc`:

| Arm | Ratio vs pandas 2.2.3 |
|---|---:|
| Float64 / Int64 / Bool homogeneous | 76.7–96.9× |
| Datetime64 / Timedelta64 | 130.4–154.7× |
| Mixed-numeric (PromotedFloat64) | 39.1–46.3× @10k/100k |
| Contiguous-Utf8 | ~43–69.8× |
| Nullable Int64 / canonical nullable Float64 | 38.2–43.7× |

### 3.2 `MultiIndex::to_flat_index` — PARKED-unmeasured → landed

`:11362` recorded a candidate with **parity already proven and perf UNMEASURED**, patch saved, working
tree pristine, blocked purely by a disk emergency that banned local builds. Not a lever failure — an
instrument failure, held honestly. `:11472` records it **UNPARKED + LANDED: 1.437× fp-side on the
shipped parallel path (2.945× serial)**.

**Lesson for the fleet:** a "PARKED, perf UNMEASURED" row is a *pre-identified* VOID with the design
work already paid for. Grep for those first — they are the cheapest resurrections available.

---

## 4. Hand-confirmed VOID entries

Ranked by the self-time / measured cost of the target frame where that is recorded. "Null floor at the
time" is *none* wherever the entry ran no A/A control — which is the modal case.

| # | Entry (line) | Target | Ratio claimed | Null floor at the time | Self-time of target frame | Binary sha? | Verdict |
|---|---|---|---:|---|---|---|---|
| 1 | `:9510` var/std/sem(axis=1) tiled two-pass moment kernel | `reduce_rows_func_f64` | 0.975× FP-side | **none** | not captured; sibling `median_axis1` measured ~107 ms @1M×10 | no | VOID → **NULL_UNDECIDABLE** (§5) |
| 2 | `:11362` `MultiIndex::to_flat_index` core::fmt kill | `to_flat_index` | **unmeasured** | none | `core::fmt` in contiguous build | no | VOID → **RESURRECTED** (§3.2) |
| 3 | `:10452` df.transpose all-valid owned row columns | `df_transpose` | 0.997× FP-side | none | 58.877 ms median routing | no | VOID → **MOOT** (eager path bypassed by §3.1) |
| 4 | `:12050` sorted-insert materializing rerun | to_dict/transpose | **no verdict** | none | CV gate aborted the run | no | VOID → **REJECT** (§5) |
| 5 | `:12364` `to_dict(index)` row-shards rerun | `df_to_dict_index` | INVALID | none | 2.620066× routing evidence only | no | VOID → **KEEP confirmed**, superseded by later 2.80× |
| 6 | `:2798` resample value-aggs | `aggregate_scalar` | std 0.92× / median 1.04× | none | ~2 ms of 25.6 ms @1M | no | **VOID — but blocked** (bit-identity risk on `nanstd`/`nanvar`, golden regen) |
| 7 | `:4471` Series Int64 Bool-mask direct-compress | `filter_int64_by_bool_mask` | 1.01–1.03× | none | — | no | VOID → **MOOT** (broader typed-gather path landed and covers f64 too) |
| 8 | `:813` max/min branchless fold | i64 max fold | ~0 gain | none | 1.15 ms | no | VOID → **MOOT** (chunked 8-lane accumulator shipped instead) |
| 9 | `:2365`, `:3161` unstack typed-output + alloc fix | unstack | ~0-gain ×2 | none | root-caused: INPUT string-parse dominates | no | VOID — attribution given, target is the parser |
| 10 | `:6703` Datetime64 sort_values radix path | sort_values | 1.24× fp-side / 0.85× vs pandas | none | gather-bound | no | VOID — mixed evidence, gather is the named wall |
| 11 | `:7714` DataFrame dedup, Scalar-backed Utf8 subset | dedup | 0.60× loss, levers ~0-gain | none | — | no | VOID — loss decidable, *levers* undecidable |
| 12 | `:9147` cut/qcut Int64 typed-input arm | `cut` | zero-gain | none | 497→500 ms, output-bound | no | STANDS in substance — deeply root-caused (output-locked contract) |
| 13 | `:4537` Series abs 0.36× / cumsum 0.93× | arc-copy-on-produce | 0.36× / 0.93× | none | — | no | Split: 0.36× decidable; **0.93× cumsum VOID** |
| 14 | `:5139` cumulative-f64 arc-copy | cumulative f64 | ~0.5× | none | — | no | Loss decidable; arc-copy attribution unproven |
| 15 | `:4655` composite OUTER merge borrowed sort keys | merge OUTER | 0.60→0.85× | none | — | no | Partial keep; residual undecidable |
| 16 | `:3290` multi-string-key dense routing correction | `build_groups` | 1.07× | none | central dispatch | no | VOID as *measurement*; superseded by the shipped mixed-radix path |

**0 of 256 entries in the audited snapshot carried a binary sha256** — matching frankenlibc's
0-of-93. The ledger did record worker ids and pinned target dirs on later rows, which was partial
mitigation. This campaign closes that instrumentation gap: `fp-bench` now self-reports its running
ELF SHA-256 on line 1, and every corrected re-run below reports the test ELF SHA-256 from inside the
same process that timed the arms.

---

## 5. Corrected top-five re-runs

The reverted candidates were reconstructed only inside ignored benchmark tests; no historical
production hunk was restored merely to obtain a number. Each measured row used one release-perf test
ELF, 25 order-alternating `ORIG / identical-ORIG-null / CANDIDATE` triplets, exact observable parity,
and a deterministic 10,000-resample bootstrap 95% CI for the median A/A ratio. A result is decidable
only when its paired median effect clears twice the null-CI log half-width. CV is reported but never
votes.

| rank | resurrected entry | worker / running ELF SHA-256 | paired ORIG÷candidate | A/A median CI | attributed self | corrected verdict |
|---:|---|---|---:|---:|---:|---|
| 1 | `:9510` axis=1 tiled two-pass variance @1M×10 | `vmi1227854` / `26a8b6eac09669fbc30d3e7fcb4d4c6ed2a26c8e8075a5fcf2372b1de9ceafac` | **0.998099×** | `[0.843831, 1.006400]` | candidate **43.26%** | **NULL_UNDECIDABLE** |
| 2a | `:11362` `to_flat_index`, serial 49k | `vmi1227854` / `ea3eb63687ad151744a3f563a9cd1edc35e362e11e143ed564badbd6955873eb` | **3.097997×** | `[0.974225, 1.048136]` | exact parity | **KEEP confirmed** |
| 2b | same ELF, parallel 1M | same | **1.741439×** | `[0.763136, 1.057655]` | exact parity | **KEEP confirmed** |
| 3 | `:10452` eager owned transpose rows | `vmi1264463` / `714a503ccc907f761cc3d214b140efd21a51f25ca503d56ccc78500562180fa2` | current route A/A only | `[0.963400, 1.010678]` | old eager target does not route | **MOOT** |
| 4 | `:12050` sorted sequential insert | `vmi1227854` / `9e583d86a2225e9c297647fa6858eaa3c6657f07b19f1c4d050f43ff0df9c7c1` | **0.593669×** | `[0.944538, 1.021580]` | wrapper 0.01%; `BTreeMap::insert` 5.98% | **REJECT** |
| 5 | `:12364` `to_dict(index)` row shards | `vmi1227854` / `e4306216bb6293f83148e6bf75534fe20834bf19ba34aad542e187bd2dcdba2c` | **1.862427×** | `[0.939331, 1.039169]` | candidate **2.59%** | **KEEP confirmed; superseded** |

Rank 1's first bounded attempt expired before the timed path on an overloaded worker and is not a
result. The pinned retry above completed all 25 triplets and proved the old `0.975×` rejection had no
authority: the corrected point estimate is essentially one, but the same-worker null CI is too wide
to decide either direction. Its profile is nevertheless admissible attribution for the next frontier
cycle (§7).

Rank 3 is not being relabeled by fiat. The current `df_transpose_materialize` route takes the direct
lazy per-output-row slot and completes in roughly 3–5 µs at 100k×10; the reverted eager
`from_f64_values_all_valid_owned_unchecked` constructor is absent from the executed graph. Reintroducing
it would benchmark dead code, so MOOT is the only contract-valid outcome.

Rank 5 re-confirms the historical direction, but its old private `BTreeMap` shards must not replace
the newer public typed-cell parallel pair-build, which already landed at about 2.80×. This is a
resurrection of evidence, not a regression to an older implementation.

---

## 6. Retry predicates

Per §4 of the campaign, concrete and falsifiable:

- **`:9510` axis=1 two-pass moments.** The target is now proven hot (43.26% self), so retry only with
  an allocator-stable/batched invocation whose A/A bootstrap CI has log half-width below 0.02. Do not
  retry the same two-output-sized-buffer shape; the next lever must remove that extra full-size
  buffer while retaining exact float order.
- **`:11362` `to_flat_index`.** No retry while the formatter-free path remains in production and its
  exact-parity tests pass. Re-open only if a future profile again places `core::fmt` above 5% self in
  the contiguous tuple-label build.
- **`:10452` eager transpose row constructor.** Re-open only if the default public transpose route
  again eagerly constructs every output row and a profile attributes at least 5% self to the row
  constructor. It has no authority over the current lazy per-slot route.
- **`:12050` sorted insert.** Re-open only after a standard-library or representation change removes
  incremental `BTreeMap::insert` from the candidate (or profiles it below 1% self). Repeating
  sequential insertion against lexical bulk `collect()` is closed by the 0.593669× result.
- **`:12364` `to_dict(index)` row shards.** Do not reintroduce this older implementation. Re-open only
  if the current typed-cell parallel pair-build regresses by more than 5% outside its A/A median CI;
  compare any successor against that current public path, not the historical serial baseline.
- **`:2798` resample std.** Retry only if a typed `aggregate_scalar_f64` path can be shown to preserve
  `fp_types::nanstd`/`nanvar` float-op order **exactly** (no golden regen). If golden regen is
  required, this is a maintainer decision, not a perf lever — keep it closed.
- **All WEAK rows.** Do not re-run in bulk. Promote a WEAK row to a retry candidate only when its
  target frame independently appears above 5% self-time in a current profile.
- **Blanket predicate for this repo.** No REJECT may be written from this point without (a) an A/A
  null control in the same invocation, (b) a self-reported binary identity, and (c) a median-CI
  decision. CV-only rejects are WEAK by construction and carry no authority to close a vein.

---

## 7. Harness adoption and frontier continuation

### 7.1 Meta-Lever 2 parts 1–3

- `fp-bench` self-hashes the running executable and emits its SHA-256, byte length, and path on
  stdout line 1.
- Every Rust and pandas workload now runs 25 same-invocation, order-alternating A/A pairs.
- `vs_pandas_harness.py` bootstraps each engine's median A/A ratio (10,000 resamples, 95% CI) and
  requires a 2× log-CI margin before emitting `FASTER` or `SLOWER`.
- `cv_pct` remains in schema v4 as provenance only. `scripts/perf_ratchet.py` quarantines
  `NULL_UNDECIDABLE` / invalid measurements, never a high-CV-but-decidable row.
- Cache-populating `df_to_numpy` / `df_values` arms construct a fresh frame outside each timed
  region, so arm B cannot inherit arm A's materialization.

Strict-remote execution of the resulting `fp-bench` reported ELF
`714a503ccc907f761cc3d214b140efd21a51f25ca503d56ccc78500562180fa2` and all 25 A/A pairs. A synthetic
high-CV case (20.20%) correctly remained `FASTER` when its median-CI effect was decisive, while a wide
null case returned `NULL_UNDECIDABLE`.

### 7.2 Profile-attributed frontier KEEP

The rank-1 profile showed the resurrected two-pass candidate at 43.26% self plus 8.50% in page
clearing, but its full-size means buffer erased the streaming benefit. The next primitive changed
that shape: complete mean and M2 for each 4096-row tile before advancing. The means scratch falls
from 1,000,000 f64s to 4,096 while every row still sees the identical left-to-right mean and M2
operation order.

The final production-path strict-remote ELF
(`d6488b2980459e896153f0bf3a1f35711371213664ee5c83a75546d3b960a59f`,
`vmi1227854`) measured both comparisons:

| comparison @1M×10 Float64 | paired ratio | A/A median CI | candidate self | verdict |
|---|---:|---:|---:|---|
| resurrected two-full-buffer → tile-local means | **1.645242×** | `[0.990203, 1.218106]` | 34.30% | **KEEP** |
| explicit legacy public row-gather → shipped public tile-local route | **1.584547×** | `[0.852740, 1.044968]` | 34.30% frontier frame | **KEEP** |

All 1,000,000 variance outputs matched both prior arms bit-for-bit before timing. The production
fast path applies the same primitive to all-valid Float64 `var(axis=1)`, `std(axis=1)`, and
`sem(axis=1)`; nullable, Int64, mixed, and generic cases retain their prior fallbacks. Retry only if a
future profile puts this moment kernel above 20% self and a different primitive removes a measured
component without changing float order.

### 7.3 Fleet recommendations

1. **Grep `PARKED / perf UNMEASURED` first.** frankenpandas's one parked row landed at
   1.437×/2.945× when the instrument unblocked.
2. **State candidate availability.** A reverted candidate is a reconstruction, not a mere re-run;
   keep it benchmark-only until corrected evidence says it should ship.
3. **A NULL result can still route the next lever.** Rank 1 could not decide the old shape, but its
   valid 43.26%-self profile named the buffer/page-fault component that produced the 1.584547×
   production-path keep.

### 7.4 Verification

- Strict-remote `cargo check --workspace --all-targets`: **PASS**.
- Strict-remote `cargo test -p fp-frame --lib`: **3,186 passed, 0 failed, 23 ignored**.
- Strict-remote `cargo test -p fp-conformance --lib`: **1,596 passed, 0 failed**. The remote worker did
  not carry the optional legacy pandas checkout, so the live-oracle smoke check reported its documented
  skip; the committed differential corpus ran green.
- Strict-remote `cargo test -p fp-bench`: **2 passed, 0 failed**.
- Strict-remote touched-package clippy (`fp-frame`, `fp-index`, `fp-bench`, `--no-deps`) with only the
  three inherited fp-frame lint classes explicitly allowed: **PASS**. The required full-workspace
  `-D warnings` attempt reached unrelated pre-existing `fp-columnar` warnings and failed there before
  this lane; no warning originated in the campaign hunks.
- `cargo fmt --check` was offered to strict RCH and rejected as a non-compilation command
  (`RCH-E301`); no local Cargo fallback was used. Direct `rustfmt --check` passes `fp-bench`; the two
  large touched library files reproduce inherited whole-file drift, while the campaign hunks match
  rustfmt output.
- A synthetic ratchet check proved a decisive row with 99% CV still votes, an arbitrarily large
  `NULL_UNDECIDABLE` delta only quarantines and cannot block/update, and a median-CI-decided 20%
  regression blocks.
- Bounded `timeout 180s ubs crates/fp-frame/src/lib.rs` reproduces the known scanner stall recorded in
  `artifacts/audits/fp_frame_ubs_inventory_2026-06-17.md`. Focused scans of the other changed source
  files found no new issue on a campaign hunk; reported criticals were pre-existing test panics, a
  deterministic low-bit boolean generator misclassified as a secret comparison, and the existing
  shell-free `fp-bench` subprocess (now resolved, project-root-confined, and executable-name checked).

---

*Generated 2026-07-25 by QuietHarbor (cc / STRUCTURAL) and completed by CrimsonGate
(cod / HARNESS+FRONTIER). No ledger row was deleted. Classifier: `audit_ledger.py`, retained in the
session scratchpad; corrected re-runs live as ignored tests in their owning crates.*

---

## 8. Re-classification under the fleet-standard taxonomy (frankenfs, adopted 2026-07-25)

The orchestrator broadcast adopted frankenfs's six-class taxonomy fleet-wide, superseding the
V1–V5 criteria used in §2. §2–§4 are **retained unchanged** as the record of what was decided
and why; this section re-scores the same 261 REJECT-bearing entries under the standard classes.
Nothing above is withdrawn — §4's hand-adjudicated head is unaffected, because every row in it was
read in full.

| Class | Count | Share |
|---|---:|---:|
| `VOID-NONULL` | 92 | 35.2% |
| `DECIDABLE-LARGE` (ratio far outside any plausible null; not a fleet class — see below) | 88 | 33.7% |
| `VALID-AB` | 39 | 14.9% |
| `UNCLASSIFIED` (screen could not assign; hand-read required) | 28 | 10.7% |
| `VOID-ZEROSELF` | 7 | 2.7% |
| `VALID-MECHANISM` | 3 | 1.1% |
| `VOID-CV` | 3 | 1.1% |
| `VALID-PROFILE` | 1 | 0.4% |
| **VOID total** | **102** | **39.1%** |
| Rows carrying a binary sha256 | 33 | **12.6%** |

### 8.1 This independently reproduces frankenfs's correction

The broadcast's key finding — *the CV gate is **not** the dominant void class; `VOID-NONULL` is* —
reproduces here without tuning:

| | frankenpandas | frankenfs |
|---|---:|---:|
| `VOID-NONULL` share of VOID | 92 / 102 = **90.2%** | 214 / 219 = **97.7%** |
| `VOID-CV` | **3** | **4** |
| rows with a binary sha256 | **12.6%** | 10.9% |

Two repos, independent ledgers, same shape. The CV gate is a real defect (it cost this repo three
rows, and cost `.218` three of four mutually-agreeing runs — §5) but it is a *rounding error* next
to the epidemic: **an A/B ran, the row was killed on a near-1.0 wall ratio, and neither an A/A null
nor a counted mechanism was written down.**

### 8.2 Two honesty notes on applying the taxonomy

**`VALID-MECHANISM` must not be inflated.** My first screen scored **102** rows `VALID-MECHANISM`
because the regex matched *"bit-identical"* / *"byte-identical"* / *"0 diffs"*. Those are **parity
proofs** (the output is unchanged), **not** mechanism refutations (the *work* is unchanged). This
ledger says "bit-identical" in almost every row, because parity is a landing requirement here.
Tightening the detector to require a **counted quantity shown unchanged** (instructions, cycles,
syscalls, allocations, page/branch/cache misses, `perf stat`) dropped it from 102 to **3**. The
broadcast warns this class "cuts BOTH ways"; conflating parity with mechanism silently rescues ~100
rows that were never refuted on a count. **Anyone porting the screen should check this first.**

**`DECIDABLE-LARGE` is a screening bucket, not a verdict.** 88 rows recorded a ratio far outside any
plausible null floor (e.g. 0.17×, 0.36×, 3.5× worse) with no null control. A null control cannot
rescue a 3.5× regression, so these are *not* `VOID-NONULL` — but they are not `VALID-AB` either.
They need hand-adjudication into `VALID-MECHANISM` (if the mechanism was counted) or `VOID-NONULL`
(if the large ratio was cross-worker — see §8.3, which makes several of them suspect).

### 8.3 ⚠️ The build-variance blocker that makes marginal rows undecidable is now REMOVED

`NEGATIVE_EVIDENCE.md:92` is a standing methodology BLOCKER, and it is the single most important
context for this whole audit:

> *"`rch` distributes `cargo build` across heterogeneous workers (ovh-a/hz2/…) with differing
> `target-cpu`, so … autovectorization-sensitive kernels (the where/mask f64 branchless bit-select)
> swing ~2.6× in wall-time on **byte-identical source**. CONSEQUENCE: any A/B whose two binaries
> were built on DIFFERENT workers is invalid; only LARGE deltas (≳2× and structural) survive."*

Per the orchestrator survey (2026-07-25), **`ovh-b` (Xeon E3-1245 v2, Ivy Bridge 2012) was the only
worker lacking avx2+fma and has been removed from the rust tag**; 73 rust slots remain across 11
AVX2+FMA workers. The heterogeneity that produced the 2.6× same-source swing — and the 1-ULP acosh
golden flips — is therefore **substantially reduced**. That blocker's own retry predicate ("pin
`-Ctarget-cpu=x86-64-v3` … then re-A/B the suspect items") is now satisfiable by default.

## 9. ISA-shaped rejections — VOID candidates queued for Lane M

Per the orchestrator: the premise *"the residual is ISA-bound (SSE2 vs AVX2) and therefore not an
agent lever"* is now **false**. Every row resting on it is a VOID candidate. **frankenpandas is Lane
B — none of these were re-run. They are queued for our Lane M rotation.**

| # | Row | Rejected because | Why the premise is now void | Class |
|---|---|---|---|---|
| 1 | `:90` Float64 comparison `+sse4.1,+avx` build-policy probe | **BLOCKED** — "no admissible workers: `critical_pressure=1,insufficient_slots=11`"; every flagged build fell open locally, so no remote same-worker A/B was obtainable | Pure **worker-admission** failure, not a lever failure. Local signal was **1.16×–2.38× vs pandas** against remote main's 0.71×–1.10×, checksum unchanged (`49999550`). 73 AVX2 slots now available. | **VOID — highest ISA rank** |
| 2 | `:183` `Series.round()` `round_ties_even` intrinsic | 3.40 → 11.9 ms, **3.5× WORSE** | Explicitly ISA-caused: *"on the BASELINE x86-64 target (SSE2, no `+sse4.1`) the intrinsic lowers to a libm `roundeven` CALL per element (**no `roundpd`**), un-vectorizable."* `roundpd` is SSE4.1; every remaining worker has it. The measurement is sound *for SSE2* and says nothing about the shipped ISA. | **VOID-ISA** |
| 3 | `:222` max/min portable-SIMD `i64x4` (uza04.207) | 0.17× / 0.16× — "AVX2-width `std::simd` variant was 3.1× slower than the manual accumulator" | An **AVX2-width** vector type compiled for a **baseline** target lowers to scalar/SSE2 emulation — the reject measured the emulation, not the lever. | **VOID-ISA** |
| 4 | `:788` explicit AVX2 via `#[target_feature(enable="avx2")]` + runtime dispatch | Reverted — requires `unsafe`, and the crate is `#![forbid(unsafe_code)]` (cf. `:571`) | The *unsafe* objection stands and is a real repo constraint. But the **safe** alternative it was weighed against — a global target-feature build — was rejected on **portability**, which is the premise now removed. Re-decide the safe build-flag path, **not** the unsafe intrinsic. | **VOID (partial)** |
| 5 | `:5235` `.cargo/config.toml` "intentionally empty — a `+fma,+avx2` … deliberately-reverted build decision" | Deliberate portability choice | Same premise. The fleet no longer contains a non-AVX2 rust worker. | **VOID (policy)** |
| 6 | `docs/repo_vs_pandas_assessment_dustysummit.md:42` `df_dot` **AVX2 REJECT #2** | Measured AVX2 = 3850 µs vs SSE2 5426 µs = **1.4×**, "still 3.1× slower than pandas single-thread… **not worth it at a portability cost**" | ⚠️ **Only PARTLY void — state this honestly.** Unlike rows 1–5 this one *did* build with `+avx2,+fma` and measure. **The 1.4× measurement stands.** What is void is only the *cost/benefit* half of the conclusion ("at a portability cost"), since there is no longer a portability cost. It does **not** become a 2–4× win. | **VALID measurement / VOID rationale** |

**Ranking for the Lane M rotation** (campaign rule: by target-frame self-time, tie-broken toward
levers whose design work is already done): **1 → 2 → 3 → 6 → 4 → 5.** Row 1 outranks the rest
because it is the only one that was never measured *at all* and it already has a positive local
signal; rows 2–3 are cheap re-runs of existing reverted patches; row 6 needs only a re-decision, not
a re-measurement.

**Gate when Lane M opens:** same worker for both arms, ELF sha recorded per arm (same source +
different sha ⇒ codegen changed, per campaign §2.6), A/A null in the same invocation, median-CI gate,
and **wall/cycles — never instruction count** (an ISA change retires more work per instruction, so
fewer instructions is the mechanism, not a neutral proxy).

**Not queued:** the str-groupby factorization floor. It is hash-table-bound, not ISA-bound, and the
standing "no 6th hash-table attempt" prohibition is unaffected by the ISA change.
