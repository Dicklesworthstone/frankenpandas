# Round 3 Recommendation Contract

Change:

- Add guarded identity-alignment fast path in `fp-groupby::groupby_sum` to bypass `align_union + reindex` when both inputs already share identical, duplicate-free indexes.

Hotspot evidence:

- pre-change benchmark mean: `0.899 s` (`artifacts/perf/round3_groupby_hyperfine_before.json`)
- post-change benchmark mean: `0.291 s` (`artifacts/perf/round3_groupby_hyperfine_after.json`)
- mean speedup: `67.6%`
- before-flamegraph hotspots included alignment/reindex/hash-heavy index-map work.

Mapped graveyard sections:

- `alien_cs_graveyard.md`: `§0.1` profile-first loop, `§0.2` opportunity gate, `§0.3` isomorphism proof, `§0.7` artifact contract.
- `high_level_summary_of_frankensuite_planned_and_implemented_features_and_concepts.md`: cross-project optimization-gated scoreboard and decision-contract discipline.

EV score (Impact * Confidence * Reuse / Effort * Friction):

- `(5 * 5 * 4) / (1 * 1) = 100.0`

Priority tier (S/A/B/C):

- `S`

Adoption wedge (boundary/compatibility/rollout):

- Boundary: internal `fp-groupby` kernel logic only.
- Compatibility: no API change, strict/hardened behavior unchanged.
- Rollout: default-on with duplicate-index guard preserving legacy path.

Budgeted mode (default budget + on-exhaustion behavior):

- No new runtime budget parameters required for this lever.
- Exhaustion/failure behavior: immediate fallback to existing path conditions (non-identical or duplicate indexes).

Expected-loss model (states/actions/loss):

- states: `{identity_alignment_common, duplicate_or_nonidentity_alignment, semantic_regression}`
- actions: `{ship_guarded_fast_path, keep_old_unconditional_alignment}`
- illustrative losses:
  - `L(ship, identity_alignment_common)=1`
  - `L(keep_old, identity_alignment_common)=40`
  - `L(ship, semantic_regression)=100`
  - `L(keep_old, semantic_regression)=0`
- selected action: `ship_guarded_fast_path` because regression risk is controlled by explicit guard + conformance checks.

Calibration + fallback trigger:

- trigger fallback/revert if:
  - packet parity gate fails, or
  - benchmark mean regresses >5% on two consecutive runs.

Isomorphism proof plan:

- preserve duplicate/non-identity behavior by routing through legacy path.
- verify ordering/tie-break semantics via unit and conformance tests.
- verify golden outputs via checksum bundle.

p50/p95/p99 before/after target:

- before: `p50=0.896 s`, `p95=0.933 s`, `p99=0.938 s`
- after: `p50=0.290 s`, `p95=0.297 s`, `p99=0.297 s`
- target: reduce all tail metrics by at least 30% with zero parity drift (achieved).

Primary failure risk + countermeasure:

- risk: accidental behavior drift in duplicate-index cases.
- countermeasure: explicit `has_duplicates` guard + dedicated duplicate-index regression test + conformance gates.

Repro artifact pack (env/manifest/repro.lock/legal/provenance):

- `artifacts/perf/ROUND3_BASELINE.md`
- `artifacts/perf/ROUND3_OPPORTUNITY_MATRIX.md`
- `artifacts/perf/ROUND3_ISOMORPHISM_PROOF.md`
- `artifacts/perf/round3_groupby_hyperfine_before.json`
- `artifacts/perf/round3_groupby_hyperfine_after.json`
- `artifacts/perf/round3_groupby_flamegraph_before.svg`
- `artifacts/perf/round3_groupby_flamegraph_after.svg`
- `artifacts/perf/round3_groupby_strace_before.txt`
- `artifacts/perf/round3_groupby_strace_after.txt`

Primary paper status (hypothesis/read/reproduced + checklist state):

- not paper-specific; this lever is profile-guided identity-case elimination under graveyard process discipline.

Interference test status (required when composing controllers):

- N/A (no adaptive controller composition introduced).

Demo linkage (`demo_id` + `claim_id`, if production-facing):

- N/A for this internal kernel optimization.

Rollback:

- revert the fast-path block in `crates/fp-groupby/src/lib.rs`.

Baseline comparator (what are we beating?):

- prior unconditional alignment/reindex path in `fp-groupby::groupby_sum`.

