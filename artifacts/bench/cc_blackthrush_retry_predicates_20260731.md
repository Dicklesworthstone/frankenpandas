# Negative-ledger retry predicates — which are now satisfied

- Agent: BlackThrush · Date: 2026-07-31 · Base: `d8c521633`
- Scope: every negative-ledger row carrying an explicit retry-condition predicate.
  Each predicate is **checked against the world as it is today**, and the check is
  recorded whether it passes or fails.

A predicate being satisfied does **not** mean the row flips. It means the row is
now *runnable*. Three of the families below turn out to need different work than
"re-run", and that distinction is the point of this pass.

## Summary

| ledger row | predicate | status today | what the row now needs |
|---|---|---|---|
| L5259 math-unary family (6 rows: floor/ceil/trunc/round2/sqrt/log) | "the build-target call is revisited" | **SATISFIED** | a timed A/B — measurable |
| L12203 `to_dict` shard rerun | "a fail-closed RCH worker with `perf` installed" + both CVs < 5% | **HALF SATISFIED** | `perf` yes; paired estimator still to run |
| L12387 transposed-column arm | same | **HALF SATISFIED** | same |
| L12925 `df_transpose_materialize` | "RCH must retrieve a HEAD-current `release-perf/fp-bench` whose timestamp and SHA can be recorded" | **SATISFIED** | the prescribed harness run |
| L14814 / L14855 fp-join outer-join gates | "strict RCH admission materially changes" | **SATISFIED** | ⚠ **re-implementation, not a re-run** |

## The checks, in detail

### 1. L5259 — the ISA / math-unary family. SATISFIED.

The row records six losses vs pandas and declares them structural:

> floor 0.089x (2.13 vs 0.19ms), ceil 0.11x, trunc 0.13x, round(decimals) 0.090x,
> sqrt ~0.085x, log 0.20x … These need vroundpd (SSE4.1) / vsqrtpd-wide / SVML,
> but fp builds for GENERIC x86-64 … **This is the ceiling for the math-unary
> family until that build-target call is revisited.**

The predicate is "until that build-target call is revisited". Commit `fbc5b3f60`
asserted it was satisfied because `ovh-b` lost the `rust` tag. **I did not take
that on trust — I probed the hardware.** Every rust-tagged worker, individually:

```
ovh-a       avx2 bmi2 fma          vmi1227854  avx2 bmi2 fma
hz1         avx2 bmi2 fma          vmi1264463  avx2 bmi2 fma
hz2         avx2 bmi2 fma avx512f  vmi1156319  avx2 bmi2 fma
w10         avx2 bmi2 fma          vmi1153651  avx2 bmi2 fma
vmi1149989  avx2 bmi2 fma          vmi1152480  avx2 bmi2 fma
vmi1167313  avx2 bmi2 fma
```

11 / 11 carry avx2 + bmi2 + fma. `workers.toml` confirms the one exception is
excluded by policy, not by luck: `ovh-b` is tagged `["bun","go","no-avx2"]` with
the standing note *"2026-07-25 (orchestrator, fleet-wide confirmation): DO NOT ADD
`rust` BACK."* The Rust fleet's ISA floor really is x86-64-v3. **Predicate
satisfied on evidence, not on a tag.**

### 2. L12203 / L12387 — the profile-integrity gate. HALF SATISFIED.

Both rows died on the same error, on `hz2`:

> `perf must be installed on the RCH worker: Os { code: 2, kind: NotFound }`

Probed today:

```
hz1 7.0.12 · vmi1149989 6.17.13 · vmi1152480 6.17.13 · vmi1153651 6.17.13
vmi1156319 6.17.13 · vmi1167313 6.17.13 · vmi1227854 6.17.13
vmi1264463 6.17.13 · w10 6.17.13                     · hz2 NO-PERF
```

9 of the 10 workers probed now have `perf` (the two OVH boxes were not probed for
`perf` in this pass, so this is 9/10 of what was checked, not 9/12 of the fleet).
The rows failed on the one box that still does not have it.
**The `perf` half of the predicate is satisfied — just not on hz2.** The second
half ("a robust paired estimator with both CVs below 5%") is a measurement that
has not been run, so both rows stay OPEN. Their blocker has changed from
"impossible" to "unscheduled", which is a real move and should be recorded as one.

### 3. L12925 — HEAD-current retrievable ELF. SATISFIED.

> Concrete retry condition: RCH must retrieve a HEAD-current `release-perf/fp-bench`
> executable whose timestamp and SHA can be recorded.

The failure mode was "custom-target retrieval returns only ~507 bytes, executable
absent". This session builds exactly that ELF via
`rch exec --base HEAD --clean-overlay --no-overlay`, and the harness self-reports
its SHA-256 from inside the measuring process. Predicate satisfied.

### 4. L14814 / L14855 — fp-join, admission. SATISFIED, but the row does not want a re-run.

> Resume only when strict RCH can admit the full `fp-join`, conformance, check,
> clippy, and fmt sequence.

Admission was genuinely broken at the start of this session:

```
no admissible workers: critical_pressure=2, insufficient_slots=2, hard_preflight=8
```

Diagnosis: the only two workers carrying the pinned `nightly-2026-04-22` toolchain
(hz1, hz2) were both in critical disk pressure, and the other eight failed hard
preflight for want of that toolchain. Repaired by syncing the pinned toolchain to
`vmi1149989` and forcing `rch workers capabilities --refresh` — admission is
cached, so the sync alone changes nothing until the cache is invalidated. Builds
now dispatch. **Predicate satisfied.**

But read what those rows actually say: *"The candidate hunk was removed manually;
`crates/fp-join/src/lib.rs` is again byte-identical to `origin/main`."* There is
no candidate in the tree to re-measure. These rows need the lever
**re-implemented** before anything can be re-run. Classing them as "a re-run away"
would be wrong, and it is the kind of wrong that quietly inflates a conversion
queue.

## Fleet repairs made in this session (side effects, recorded on purpose)

1. `nightly-2026-04-22` synced to `vmi1149989` (was: only hz1/hz2 had it).
2. `rch workers capabilities --refresh` — cleared the stale admission cache that
   was refusing every worker.
3. pandas 2.2.3 + pyarrow 24.0.0 installed to `/root/fpbench-libs` on `vmi1149989`
   via `pip --target` (no venv, no system packages touched). The harness's own
   `--dependency-probe` now returns
   `pandas_dependency_probe=ready version=2.2.3 pyarrow_version=24.0.0`.
   pyarrow 25.0.0 was installed first and **the probe rejected it** — the pin is
   exact and fail-closed, which is correct behaviour and worth noting as working.
4. `vmi1149989` drained (`rch workers drain -y`) so it can reach quiescence. **It
   must be re-enabled**; see the closing note of the companion measurement artifact.
