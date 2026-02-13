# Round 2 Recommendation Contract

Change:

- Replace owned-key first-position maps in `fp-index::align_union` with borrowed-key maps to reduce clone/allocation pressure in alignment-heavy paths.

Hotspot evidence:

- Pre-change benchmark (12 runs) mean `1.400 s`, stddev `0.051 s`.
- Post-change benchmark (12 runs) mean `1.138 s`, p50 `1.129 s`, p95 `1.191 s`, p99 `1.206 s`.
- Net mean improvement: `18.7%`.

Mapped graveyard sections:

- `alien_cs_graveyard.md`: `§0.1` (profile-first loop), `§0.2` (opportunity gate), `§0.3` (isomorphism), `§0.7` (artifact contract), `§0.13` (runtime decision contract), `§0.15` (tail decomposition).
- `high_level_summary_of_frankensuite_planned_and_implemented_features_and_concepts.md`: `§0` framework, cross-project opportunity scoreboard, project-level decision contracts, and evidence-ledger discipline.

EV score (Impact * Confidence * Reuse / Effort * Friction):

- `(4 * 4 * 4) / (1 * 2) = 32.0` (implement threshold exceeded).

Priority tier (S/A/B/C):

- `A` (high ROI, low integration risk, immediate measurable win).

Adoption wedge (boundary/compatibility/rollout):

- Boundary: `fp-index` internal map construction only.
- Compatibility: no API changes, output-isomorphic semantics.
- Rollout: immediate default in strict + hardened modes.

Budgeted mode (default budget + on-exhaustion behavior):

- Budget: no new runtime budget knobs introduced; this is a deterministic structural optimization.
- Exhaustion behavior: N/A for controller budget; fallback is code rollback if conformance/perf guardrails fail.

Expected-loss model (states/actions/loss):

- States: `{semantic_regression, perf_regression, isomorphic_speedup}`.
- Actions: `{keep_old_owned_map, ship_borrowed_map}`.
- Loss matrix (illustrative):
  - semantic_regression + ship = 100
  - perf_regression + ship = 30
  - isomorphic_speedup + keep_old = 15
  - isomorphic_speedup + ship = 1
- Selected action: `ship_borrowed_map` because conformance + checksums remove semantic-regression evidence and measured speedup is strong.

Calibration + fallback trigger:

- Trigger fallback if either condition holds:
  - any packet parity gate fails, or
  - benchmark mean regresses >5% relative to current baseline for two consecutive runs.
- Fallback action: revert optimization and reopen opportunity matrix.

Isomorphism proof plan:

- Validate ordering/tie-break behavior in index alignment tests.
- Re-run conformance packet suite with gate enforcement.
- Verify golden checksum set unchanged.

p50/p95/p99 before/after target:

- Before: pre-change run captured mean/range only (`1.400 s`, `1.335..1.500 s`).
- After: `p50=1.129 s`, `p95=1.191 s`, `p99=1.206 s`.
- Target achieved: mean runtime reduced by at least 10% with no conformance drift.

Primary failure risk + countermeasure:

- Risk: lifetime/key-equality misuse causing incorrect map lookups.
- Countermeasure: keep key type as `&IndexLabel`, preserve `Eq/Hash` semantics, and gate with parity + checksum tests.

Repro artifact pack (env/manifest/repro.lock/legal/provenance):

- `artifacts/perf/ROUND2_BASELINE.md`
- `artifacts/perf/ROUND2_OPPORTUNITY_MATRIX.md`
- `artifacts/perf/ROUND2_ISOMORPHISM_PROOF.md`
- `artifacts/perf/round2_groupby_hyperfine_after.json`
- `artifacts/perf/round2_groupby_strace_after.txt`

Primary paper status (hypothesis/read/reproduced + checklist state):

- Not paper-driven for this lever; this is a direct data-structure application under graveyard process discipline.

Interference test status (required when composing controllers):

- Not applicable; no adaptive controllers composed in this lever.

Demo linkage (`demo_id` + `claim_id`, if production-facing):

- Not production-facing demo scope in this round.

Rollback:

- Revert `crates/fp-index/src/lib.rs` borrowed-map change.

Baseline comparator (what are we beating?):

- Prior owned-key alignment-map implementation in `fp-index::align_union`.

