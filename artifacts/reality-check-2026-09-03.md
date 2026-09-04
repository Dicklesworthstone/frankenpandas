# FrankenPandas Reality Check — 2026-09-03

> Generated via `/reality-check-for-project` (skill: `je_private_skills_repo/.claude/skills/reality-check-for-project`), run end to end by zai/glm-5.3-flash (omp) in `/data/projects/frankenpandas` at HEAD `04004e6b4`. Inputs: full read of `AGENTS.md` (project + suite-wide), `README.md` (2,961 lines), all 16 `docs/planning/*.md`, the live work graph (4,017 beads: 16 open / 56 in_progress / 13 blocked / 1 review), four parallel read-only audit scouts (metrics, coverage, conformance, perf), and an end-to-end run of the software. **Beads + this artifact are the deliverable.**

---

## TL;DR

**The library is real, large, tested, and — on certified lanes — roughly 4x faster than pandas. But every enforcement gate around it is currently advisory: CI has produced zero green runs in 1,200 attempts and has run no jobs at all since 2026-08-16, and the ready queue is gated on four representation decisions no agent may make. The code outran its verification infrastructure.**

The May 2026 reality check's three CRITICAL gaps (SeriesGroupBy 20%, MultiIndex 19%, Index variants 0%) are **CLOSED in code**: 69/69 SeriesGroupBy pandas methods, full MultiIndex surface (131 pub fns), all five typed Index variants present (111–170 pub fns each). The gap moved from *capability* to *assurance*: the differential/live-oracle/perf-certification machinery exists, is well designed, and is mostly not currently running anywhere that enforces anything.

| Vision goal (AGENTS.md / README) | Status | Evidence |
|---|---|---|
| Absolute pandas API parity | **LARGELY MET** (Rust side) | 2026-05-03 gaps closed; drift report shows 99–100% on 4 of 5 surfaces; 22 active parity beads target the residue (tz carrier, DISC-006, resample, argsort, Bool**Int64, object dtype…) |
| True drop-in replacement | **NOT MET (Python)** — honest | `fp-python` ~18% of surface, no wheel CI/PyPI/stubs/pytest harness (bead `6tglr`); README says so |
| Differential conformance for the FULL surface | **INFRASTRUCTURE BUILT, ENFORCEMENT DOWN** | 1,341 packets all with provenance; local `.venv-oracle` runs live; **CI conformance job hasn't executed since 2026-08-16** (`ey5sl`, P0); 5 groupby-resample live tests `#[ignore]`d; per-packet parity reports stale-once-written |
| Performance EXCEEDS pandas across the board | **MET ON CERTIFIED LANES / UNPROVEN ON 60% OF LANES** | Certified census 2026-09-03: 143/359 lanes certified, geomean **3.967x**, every category >1.0x, 10 certified losses (4 flagged stale-ELF); 216 lanes undecidable/uncertified |
| Zero unsafe | **MET** | `#![forbid(unsafe_code)]` verified 15/15 crate roots |
| Clean-room | **MET** (no contrary evidence) | Oracle used only behaviorally via subprocess; no pandas source vendored |
| Bayesian runtime policy / EvidenceLedger / RaptorQ | **MET** (as designed, feature-complete crates) | `fp-runtime` 3,920 LOC, tested; ledger wire format documented |

---

## Phase 1 — Where are we REALLY?

### 1. What specifically IS working right now

1. **The Rust DataFrame engine.** 15 crates, 587,294 lines under `crates/` (507,426 under `src/`), 8,755 `#[test]` markers. DataFrame 960+ methods, Series 800+, full typed Index family, 3-path GroupBy with bitwise-equality property tests, all 6 join types with the full `merge_asof` option matrix, eval/query with `@local` + backticks, 14+ IO formats. HEAD compiles (`cargo check -p fp-columnar` clean; the `+sse4.1` release lock is a *deliberate* compile-time guard, verified working from an outside workspace).
2. **The conformance corpus.** 1,341 packet JSONs, every one carrying `fixture_provenance`; drift ledger `artifacts/phase2c/drift_history.jsonl` appended to *today* (FP-P2D-465/467/468 green at 2026-09-03 01:20Z). 26 DISC entries with 15 active, each with root-cause and reproducible packet.
3. **Live differential testing on this checkout.** `.venv-oracle/bin/python` exists and is treated as the *designated* oracle (`HarnessConfig::default_paths`, `fp-conformance/src/lib.rs:211-240`), so `live_oracle_*` tests genuinely execute here — and have found real bugs (to_datetime utc handling `f2mlr`, merge_asof family `fkiu9`), which are tracked, not buried.
4. **Performance vs pandas, where certified.** Scorecard regenerated today: geomean **3.967x** on 143 certified lanes; all 10 categories >1.0x certified geomean; losses reported honestly with structural analysis (2-D block transpose floor 0.004x; str-key groupby factorization; df_dot GEMM). Incumbent runs live in the same invocation with A/A nulls and a bootstrap median-CI gate.
5. **The May audit's blast zones closed.** SeriesGroupBy 69/69; MultiIndex full set/search/restructure surface; DatetimeIndex/TimedeltaIndex/PeriodIndex/RangeIndex/CategoricalIndex all exist as real types (88.3%→~100% vs the stale May drift report).
6. **An unusually honest README.** Limitations table, IO caveats (ORC fail-closed, pickle envelope), loss reporting doctrine, roadmap status column. The measured-metrics audit found headline claims accurate to ±0.2%.

### 2. What is NOT working or not yet implemented

**A. CI is dead, so every gate is advisory (P0, `ey5sl`).**
GitHub Actions API: 1,200 consecutive runs back to 2026-07-13 with **zero successes** (277→123→50 failures then all cancellations). Since 2026-08-16T03:00Z, runs cancel with **zero jobs started** — consistent with an account-level minutes/runner/billing state that only someone with account access can diagnose. `ci.yml`'s configuration is *correct* (FP_REQUIRE_LIVE_ORACLE=1, system-pandas fallback), so the repo currently contains a true description of something that does not execute. README claims "pinned oracle in CI's daily batch" and "live oracle runs in CI on every PR" — true of config, not of reality, until this is fixed.

**B. Live-oracle verification is partial even where the oracle exists.**
- Packet drift rows default to `OracleMode::FixtureExpected` (fixture replay), not live.
- `run_live_oracle_report` → `artifacts/ci/live_oracle_report.json` **does not exist** in the artifacts tree (last write: never committed; `artifacts/ci/` holds only a March governance report).
- 5 `live_oracle_dataframe_groupby_resample` tests are `#[ignore]`d (`FP_GROUPBY_RESAMPLE_UNSUPPORTED`) — documented, but still skips.
- Per-packet `parity_report.json` / `parity_gate_result.json` are **written once and never refreshed** (`fp-conformance/src/lib.rs:4386-4413`): an on-disk report can silently contradict newer drift rows (FP-P2D dirs dated 2026-08-18 while drift rows update daily).
- Fixture residue: 44 fixtures that cannot be honestly restamped (`nvnvr`); 4 fixtures pin "unsupported constructor dtype" for dtypes live pandas constructs (`bhyqp`); `FP-P2D-017` three-way divergence (`rh1od`).

**C. The ready queue is decision-gated; throughput on it is zero (`aj19i`, P1).**
Four representation decisions gate fourteen open beads:
1. int64→float64 null-promotion rule (5 beads; rule already measured, needs a ruling; 81 fixtures ride on it);
2. the timezone carrier (5 beads; `DType::Datetime64` has no zone slot; half the surface silently shipped naive-UTC via `f2mlr`, the other half Utf8);
3. pivot_table dropna oracle default (2 beads; oracle tests the opposite of its name);
4. Timedelta/Timestamp overflow observability (3 beads; DISC-018).
Each new producer reaching the same rules files a fresh bead: the queue grows while throughput on it is zero.

**D. Active parity bugs in core ops** (all bead-tracked, none hidden): `Resampler::asfreq/ffill/bfill` are first()/last() in disguise — no empty buckets, `limit` ignored (`kmy0b`); multikey groupby output flattens row MultiIndex (DISC-006, `wfkzm`); pivot/crosstab/groupby stringify FLOAT/BOOL index labels (`9m9zf`); Period/Interval groupby keys become debug strings (`no6s4`); `groupby().resample()` drops non-numeric columns (`a7faz`); `Series.argsort` tie order (`dxkbb`); Bool**Int64 (`cajyl`); `str.encode` returns byte lengths (DISC-025, `rw01l`); `iloc` duplicate-column selector (`eda3x`); clip array-bounds TypeError (`n8aqm`); `prod(axis=1)` Int→Float (`o9lmv`); no object dtype — bool+numeric coerces (`hlcgl`); mixed-format datetime lists (`mhygz`); constructor dtype parser lowercases nullable dtypes (`jozfk`).

**E. Python drop-in is ~18%** (`6tglr`): wheel builds via maturin; no `loc`/`iloc`/`at`/`iat`, no concat/merge/to_datetime, no Index classes, no wheel CI, no pytest differential harness, no PyPI. The AGENTS.md "true drop-in" mandate is not met for Python users (README states this accurately).

**F. Perf certification covers 40% of lanes.** 143/359 certified; 216 NULL_UNDECIDABLE/DROPPED_HIGH_CV. 6 of 10 certified losses are flagged STALE ELF. `.bench-history/latest.json` — the ratchet's global baseline — is an **empty schema-v3 placeholder** (`claim_validated: false`); standing lock banks defend 5 of 48 workloads (`85clb`). The ratchet machinery refuses cross-worker/cross-harness comparisons by design, so most historical rows can't vote.

**G. Ops/hygiene.** 212 GB of cargo target dirs on a volume with a 42 GB brake (`q35on`); commit-attribution mixing in the shared checkout (`lz1yy`); 122 gitleaks prose findings, no `.gitleaksignore` policy (`piehe`); README carries six stale "1,252 packets / 1,265 fixtures" sites contradicting its own 1,341 claims, and the `thread::scope` site count (~140) overstates the measured 47 scope sites / 58 spawn calls (the ~350–420 µs spawn-cost claim itself is sourced from a header comment that documents it *overstates* live cost: 19–98 µs for the persistent pool).

### 3. What is blocking us

1. **CI root cause** — needs GitHub account/billing visibility no agent has (`ey5sl` scope item 1).
2. **The four representation rulings** — explicitly beyond agent authority (`aj19i`: re-banking 81 fixtures on an agent's own authority is the laundering the campaign forbids).
3. **Certification throughput** — A/A-null + median-CI gates need quiet-host windows and HEAD-matching ELFs; bench corpus has zero new rows since 2026-09-01.
4. Nothing else material: the capability backlog is deep, partitioned, and honest.

### 4. If we implemented all open + in-progress beads, would the gap close?

**Nearly, not completely.** The 86 non-closed beads cover the parity residue, the decision clusters, conformance integrity, perf floors, Python surface, and ops. Cross-checking every vision goal and roadmap row against the graph found these **NO_BEAD gaps** (filed today, see §Beads):

| Gap | Why it matters | Bead |
|---|---|---|
| Full **live** oracle run on this checkout + emit `artifacts/ci/live_oracle_report.json` | The README's 1,323/1,341 claim has no on-disk report backing it; a local full run is possible *today* (venv exists) and is not gated on CI | filed |
| Per-packet parity-report **staleness** (never refreshed once written) | Artifact record can contradict the drift ledger; same "subject/object come apart" class as `l7r1p`/`ey5sl` | filed |
| **fp-columnar `+sse4.1` lock breaks downstream release builds** | Any external consumer of the crates.io crate hits E0080 unless they replicate an internal rustflags stanza (`profile-rustflags` is cargo-unstable, nightly-only). Publish story undocumented | filed |
| Roadmap **High**: Tokio-free PostgreSQL adapter | Empty placeholder feature; zero beads | filed |
| Roadmap **High**: signed tags + 0.3.0 release | ~1,700 commits since 0.2.0; zero beads | filed |
| Roadmap **Medium**: native plotting renderer | PlotSpec/HistogramSpec/BoxPlotSpec data with no renderer; zero beads | filed |
| README **number corrections** (six 1,252/1,265 sites; scope-site count; per-crate LOC table; DISC section hygiene) | Internal inconsistency, not spin; chore lane per spec-editing rules | filed |

Still not closable by beads alone: CI account access (human), the four rulings (maintainer), and "exceeds across the board" on the 216 uncertified lanes (requires the measurement windows above).

### 5. Vision goals not covered by ANY bead (before today)

The seven in the table above. After today's filings: only the human-gated items remain uncovered.

---

## Phase 2 — Bridge Plan (close every conceivable gap)

Ordered by leverage, respecting the campaign's own rules (no gate-weakening; rulings belong to the maintainer; losses are successes):

1. **Revive enforcement (highest leverage).** ey5sl scope: (a) escalate CI zero-job cancellations to the account owner; (b) triage the 12 failed jobs of run 31923213809 individually; (c) decouple the conformance job from `needs: [test]` so parity signal survives unrelated test redness. Until then, treat `main` as green-by-faith.
2. **Bank the local live-oracle evidence.** Run the full live suite against `.venv-oracle` here, emit `artifacts/ci/live_oracle_report.json`, triage every failure into beads or DISC entries. This converts the README's best claim from historical to reproducible.
3. **Break the decision deadlock the agent-legal way.** Produce a one-page ruling brief per decision (measured evidence, options, recommendation, migration/banking cost) so the maintainer can rule in minutes; wire each brief to its gated cluster. Decision 1 first (largest fixture mass, rule already measured).
4. **Fix the named parity bugs** in the ready queue (kmy0b, wfkzm, 9m9zf, no6s4, a7faz, cajyl, eda3x, o9lmv, n8aqm, jozfk, bhyqp, rw01l) — each with its conformance packet and, where the oracle disagrees, a DISC entry or a fix, never a gate edit.
5. **Restore the perf certification surface.** Re-measure the 4 stale-ELF losses on a fresh ELF; drive NULL_UNDECIDABLE down during quiet windows (`host_is_quiet_now.py`); re-link standing locks to live harness identities (85clb); replace the empty `.bench-history/latest.json` baseline with a real certified one or delete its claim.
6. **Unblock the Python story** along 6tglr's own list: `loc`/`iloc`/`at`/`iat`, Index classes, `to_datetime`, wheel CI, pytest differential harness.
7. **Close the packaging landmine** before the next crates.io publish (fp-columnar sse4.1 lock vs downstream release builds).
8. **Land the two roadmap-High NO_BEADs** (postgres adapter, signed 0.3.0) and the plotting renderer decision.
9. **Docs chore lane**: README number corrections + DISC section hygiene (never closes a feature bead).

*(This section is the ambition-round revision target; see Phase 4 log below — revised in place, not duplicated.)*

---

## Coverage since the 2026-05-03 reality check

| Area | 2026-05-03 | 2026-09-03 |
|---|---|---|
| SeriesGroupBy | 14/69 (20%) — CRITICAL | **69/69** |
| MultiIndex | 22/118 (19%) — CRITICAL | **131 pub fns; full set/search/restructure surface** |
| Index variants | 0 dedicated types — CRITICAL | **5 typed variants, ~111–170 pub fns each** |
| DataFrame pandas-named arithmetic | `*_df` only | **generic `add`/`sub`/`mul`/`div`/`eq`/… present** (legacy `*_df` kept alongside) |
| IO formats | 8 | **14+** (HTML/XML/LaTeX/Markdown/Pickle/Stata/HDF5/IPC/SAS; ORC fail-closed; clipboard via OS tools) |
| Packets | ~1,252 | **1,341** (all provenance-stamped) |
| beads open+active | 0 (freshly converged) | 86 (deep, partitioned, honest) |

## README metrics audit (measured 2026-09-03)

| Claim | Measured | Verdict |
|---|---|---|
| 15 crates | 15 | MATCH |
| ~586,000 LOC under crates/ | 587,294 | MATCH (+0.22%) |
| ~506,700 under src/ (badge 507K) | 507,426 | MATCH (+0.14%) |
| 7,991 / 8,753 test markers | 7,993 / 8,755 | MATCH (+2 drift) |
| 1,341 packets, all with provenance | 1,341 / 1,341 | MATCH |
| "1,252 packets / 1,265 fixtures" (6 sites) | 1,341 / 1,354 | **STALE (+89), internal inconsistency** |
| 26 DISC entries, 15 active | 26 / 15 active | MATCH (section hygiene drift: DISC-025/026 after `## Rules`; FIXED used twice; DISC-012 RESOLVED-but-Active) |
| 30 fuzz targets | 30 | MATCH |
| forbid(unsafe_code) everywhere | 15/15 roots | MATCH |
| nightly pinned in rust-toolchain.toml | nightly-2026-08-25 + rustfmt/clippy/rust-src | MATCH |
| "~140 thread::scope sites" | 47 scope sites / 58 spawn calls | **OVERSTATED** |
| "CI daily batch runs pinned oracle" | config true; zero jobs since 2026-08-16 | **CONFIG≠REALITY** |

---

## Phase 4 — Ambition rounds (logged, plan revised in place above)

- **Round 1** ("decent start but"): initial bridge plan had 5 items and treated CI as one line item. Revision: CI split into its three independently-actionable scope items; added the local live-oracle run as an *unblocked* action (previously wrongly queued behind CI); added packaging-landmine bead after the smoke build tripped the sse4.1 lock from an outside workspace.
- **Round 2** ("better but still far from optimal"): the plan was bead-oriented and under-weighted *verification throughput*. Revision: added perf-recertification lever (stale ELFs, NULL_UNDECIDABLE, standing locks, empty ratchet baseline) and explicitly bound every README claim-fix to the chore lane so it can never close feature work.
- **Round 3** (domain depth): none of this needs new math — it needs *rulings and windows*. The genuinely deep lever the plan had missed: **the four decisions are one object, not four** — a single decision-docket session with pre-computed briefs converts 14 blocked beads into work in one maintainer sitting. Plan revised to treat the docket as the critical path.

## Phase 5 — Plan-space refinement (frozen checklist, round-by-round log)

- **R1**: checked every new bead for: real probe? negative case? dependency topology? → added "named probe" requirement to each; merged two overlapping docs beads into one. **Found 3 issues.**
- **R2**: re-verified no bead weakens a gate or re-banks a fixture on agent authority → packaging bead reworded to "decide + document OR soften lock via explicit opt-in feature; never silently disarm". **Found 1 issue.**
- **R3**: checked bead coverage against every vision-checklist row one final time → all rows covered or human-gated. **Found 0 issues. Round found nothing; stopped** (per skill: stop when a round finds nothing).

---

## Filed beads (Phase 3a, label `reality-check`)

| Bead | Pri | Title (abridged) |
|---|---|---|
| `br-frankenpandas-rc-live-oracle-local-run-8oey9` | P1 | Full local live-oracle run vs `.venv-oracle`; emit `artifacts/ci/live_oracle_report.json`; triage every finding (named probe incl. must-fail-red with `FP_PYTHON_BIN=/bin/false`) |
| `br-frankenpandas-rc-stale-packet-reports-2z7q9` | P2 | Refresh-once-written packet parity reports (67 stale fails on disk vs all-green drift); regenerate; fix `FEATURE_PARITY.md` auto-table (probe incl. sandbox-flip negative case) |
| `br-frankenpandas-rc-sse41-downstream-lock-hrnom` | P2 | `fp-columnar` +sse4.1 lock fails every downstream release build (nightly-only `profile-rustflags` stanza); decide + document/soften before next publish |
| `br-frankenpandas-rc-postgres-adapter-k1axt` | P2 | Roadmap-High NO_BEAD: Tokio-free PostgreSQL `SqlConnection` adapter behind `sql-postgresql` |
| `br-frankenpandas-rc-signed-release-030-kf1lc` | P2 | Roadmap-High NO_BEAD: signed tags + 0.3.0 release (blocks on `hrnom`; maintainer-only items marked) |
| `br-frankenpandas-rc-plot-renderer-zf9bf` | P3 | Roadmap-Medium NO_BEAD: renderer for `PlotSpec`/`HistogramSpec`/`BoxPlotSpec` |
| `br-frankenpandas-rc-facade-write-reexports-yo8k1` | P3 | Prelude exports IO read side, not write side — README Quick Example does not compile against the prelude |
| `br-frankenpandas-rc-readme-number-truth-mzox7` | P3 | Chore: six stale 1,252/1,265 packet sites; `~140` scope-sites vs measured 47/58; per-crate LOC table; DISC section/status hygiene |
| `br-frankenpandas-rc-differential-fuzz-decision-kgohb` | P3 | Decision: `DIFFERENTIAL_FUZZ_DESIGN.md` promised a live-pandas differential fuzz target, nothing shipped — implement nightly / mark superseded / defer on roadmap |

Dependency edge added: `kf1lc` → blocks-on → `hrnom` (release gating). `br dep cycles`: empty. No existing beads' graph structure was modified.

## End-to-end run evidence

**Smoke crate** (out-of-tree, `/data/tmp/fp_smoke_20260903`, path-dep on the facade, release profile): README Quick Start steps 1–7 end to end — `read_csv_str` → `query` → `groupby().sum()` → `to_datetime` → CSV/JSON/Feather/HTML/Markdown exports → Feather round-trip → `write_sql`/`read_sql_table` (rusqlite) → `SqlInspector` — plus four parity spot-checks: NaN-skipping `sum` (5+NaN→5), Kleene `null AND false = false`, Series `add` outer-union alignment (len 3), `query_with_locals` with `@thr`. Result: **SMOKE OK**, all passed. Output: `[1] shape=(3,3) … [7] tables=["results"]`.

Two findings fell out of the run itself:
1. Building from *outside* the workspace tripped `fp-columnar`'s deliberate `+sse4.1` compile lock (E0080) — correct behavior, and it exposed the downstream-packaging landmine filed as `hrnom`. (An external consumer must replicate a nightly-only `cargo-features = ["profile-rustflags"]` + per-package rustflags stanza in its own manifest, or release builds fail.)
2. README's Quick Example calls `write_html_string(&summary)` via the prelude — not re-exported there (read side only). Filed as `yo8k1`; the smoke used the trait method `to_html_string()` instead. Feather `equals=false` on the groupby output round-trip matches pandas' own index-dropping Feather semantics — noted, not a failure.

HEAD compile check: `cargo check -p fp-columnar` clean in-repo (dev profile disarms the lock by design; the release lock was verified *working* from the consumer side).

## Planning-docs digest (DocsScout, all 16 docs read)

`docs/planning/` is three doc generations: (1) **Feb-2026 founding specs** (`COMPREHENSIVE_SPEC_FOR_FRANKENPANDAS_V1.md` — strict/hardened doctrine, perf budgets §17, CI gates G1–G6 §18, RaptorQ §19; `EXHAUSTIVE_LEGACY_ANALYSIS.md` 2026-02-13; `PHASE2C_EXTRACTION_PACKET.md` — 5–8 packet era) whose source-line anchors are all stale; (2) an **Apr-2026 execution layer** (`TODO_EXECUTION_TRACKER.md` — M/N/O blocks unchecked; `ERROR_CONFORMANCE.md` 96/1,249 error fixtures; `PANIC_CONTRACT_COVERAGE.md` fp-io/expr/groupby/join at 0.0%; `UPGRADE_LOG.md` "12 crates / 3,171 tests" era; `REVIEW_SESSION_HANDOFF.md`); (3) **living generated docs** (`FEATURE_PARITY.md`, `COVERAGE_MATRIX.md` 10.8% fixture-naming coverage with an explicit note that implementation coverage is ~98%) plus point-in-time audits (`PARITY-COVERAGE.md` 2026-05-25: 98.5%). The packet system outgrew the planning horizon by two orders of magnitude (FP-P2C-001..011 → FP-P2D-468). The single most consequential doc defect: `FEATURE_PARITY.md`'s auto-table read the stale once-written gate files and reported **"Gate passing: 1 (0%)"** while the live drift ledger is all-green — quantified and filed as `2z7q9`. The one unshipped doc promise with zero tracking: the differential-vs-pandas fuzz target (`DIFFERENTIAL_FUZZ_DESIGN.md`, bead liai) — filed as `kgohb`. Stale Apr trackers (`TODO_EXECUTION_TRACKER` M/N/O unchecked, error/panic/categorical trackers) are superseded by the live work graph + DISCREPANCIES + CI doc gates; fold their retirement notes into the docs chore bead.

## bv validation

Post-creation graph state (`bv --robot-triage` 2026-09-03T21:42Z, data_hash `705d6dd0816168a5`): 4,027 issues; open 16→**25** (+9), in_progress 56, blocked 13, not_closed 86→**95**; actionable 91. Top picks unchanged in character (disk ops `tfi8r`/`q35on`, decision docket `aj19i` — PageRank 0.16-0.19), i.e. the triage engine agrees with this audit's conclusion: the blocking constraints are the disk drain, the dead CI/bench certification, and the decision docket. `br dep cycles`: **no cycles detected**. Cycles metric state `skipped` in bv's phase-2 (phase-1 topo clean); the dedicated `br dep cycles` check is authoritative here and is empty.


---

## Closing addendum — execution wave (2026-09-04)

All 12 reality-check beads + 5 spin-offs closed; committed through `083e4eab5`/`0a4dbb652` and beyond.

| Bead | Outcome |
|---|---|
| `8oey9` live-oracle | Report emitted; full triage; **final local pinned run 832 passed / 2 failed / 5 ignored** — both failures owned (bhyqp pyarrow env, odx3k/jozfk bool+numeric constructor cluster); 3 cross-version failures disproven by exact-invocation adjudication (FP matched pinned 2.2.3 on all three) |
| `d8wt4` provenance guard (spin-off) | `verify_oracle_pandas_pin` in `capture_live_oracle_expected` — cross-version oracles now produce honest skips, never comparisons; root cause was rch-offloaded runs resolving `.venv-oracle`'s symlinked interpreter to the worker's pandas; `.venv-oracle/` subsequently added to the rch exclude list (worker syncs skip the ~1 GB venv + symlink trap) |
| `k1axt` PostgreSQL adapter | **Implemented + live-verified against PG 17.10**: all 28 `SqlConnection` methods, typed dynamic decode (NUMERIC wire format, timestamps→`Datetime64` ns), `PgNull` type-agnostic NULL binding, `pg_err` SQLSTATE enrichment, full pg_catalog introspection; 5 module tests + 10-step out-of-tree live probe ALL PASS. README roadmap → Done |
| `yo8k1`/`mzox7`/`kgohb`/`hrnom`/`2z7q9`/`zf9bf`/`kf1lc` | Closed as detailed in the tables above; regression gates landed (doc-tree numbers test, facade drift gate, drift reconciliation test) |
| `zf8eh`/`k5lhz` clippy spin-offs | Five `x == x` NaN idioms → `!x.is_nan()`; clippy 0 errors on fp-columnar/fp-frame/fp-conformance/frankenpandas |
| `ww0m9`/`q5svu`/`p5d2q` parity spin-offs | Closed INVALID with exact-invocation proof — they were cross-version worker-pandas artifacts |

**Why the three "parity failures" mattered more than they looked:** they were the first evidence of the ey5sl class ("the check's subject and its object come apart") attacking the *differential oracle itself* — offloaded runs silently substituted a different library as the reference. The provenance guard closes that class for every future run, host or worker.

**Still open, correctly owned elsewhere:** `ey5sl` (CI account — human), `aj19i` (four representation rulings — maintainer), `nywa8`/`bhyqp`/`odx3k`/`jozfk` (in-progress parity clusters, now carrying fresh live evidence), `3826s` cluster (groupby-resample gaps).
