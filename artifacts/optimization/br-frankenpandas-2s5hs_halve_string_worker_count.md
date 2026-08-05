# br-frankenpandas-2s5hs — halve the string-kernel worker count

**Status: REJECTED. Lever NOT landed. No perf claim made.**

Agent: LavenderPine (claude-code / opus-5). Date: 2026-08-05.
Arms built by SapphireCedar; measurement run by LavenderPine after that pane
went idle for 48 minutes with both ELFs staged and the host free.

## The lever

`workers = min(available_parallelism, 64, n.div_ceil(MIN_ROWS_PER_WORKER))`
with `MIN_ROWS_PER_WORKER = 131_072`. At 1M rows that yields 8 workers; raising
the constant to `262_144` halves it to 4.

Hypothesis under test (from the bead): the per-call `thread::scope` spawn is what
makes every shipped-path string row fail the A/A null, `parallel-vs-serial` was
~1.82x, and Amdahl says 4 threads should cost far less than half that on a
memory-bound byte scan — so 4 workers might keep most of the speedup while
halving spawn cost and scheduling jitter, landing the null within ±2%.

**Both halves of that hypothesis are refuted below.**

## Arms

| Arm | `MIN_ROWS_PER_WORKER` | ELF SHA-256 | threads actually used |
|---|---|---|---|
| reference (ships today) | `131_072` | `210b5591477c84de4a1b63bfe12d671215368bd9c199edcd7a3882ec7f6e0794` | **8** |
| candidate | `262_144` | `d6b9445cee0348c3dfc170271833ebc1e7fe41d6b6963399aeef682d675b8912` | **4** |

Distinct SHA-256 on both arms, retiring the `rch-ab-elf-retrieval-trap`
shared-ELF false-REJECT class. `thread_count_actually_used` is read out of the
harness's own thread provenance, so the lever is **confirmed to have taken
effect** — this is not a no-op arm.

Sources differ in `crates/fp-frame/src/lib.rs` only.

> **Scope note on the candidate arm.** It moves the constant at **five** sites:
> `:41745`, `:41850`, `:41945`, `:42242` (the string kernels the bead scoped) and
> `:80829`, which is the dense groupby `var`/`std` worker count and is **out of
> scope**. `:80829` is not on the `str_contains_arrow` path, so this row is
> unaffected — but a landing diff must be restricted to the four string sites,
> and `:80829` measured separately if anyone wants it.

## Measurement

One workload per invocation; candidate, reference and live pandas all executed
inside the SAME invocation (`whole_binary_execution_order = ["reference",
"candidate"]`).

- host `frankenlibc-test`, AMD EPYC (IBPB), 10 physical cores / 10 logical,
  `smt_active=0`, `threads_per_core=1`, `cpu_governors=[]`
- `--category strings --sizes 1M --workloads str_contains_arrow`
- pandas 2.2.3 / pyarrow 24.0.0
  (`pandas_artifact_sha256=a6bc5a90…`, `pyarrow_artifact_sha256=b95ef54d…`)
- harness `f0a5cef146a089144075b01fb07a73bec78b681bc171fe51cd622a3f75dc7d9e`
- `pandas_dependency_probe=ready`, `corrected_null_gate_self_test=pass`
- gate `corrected_three_clause_median_bootstrap_ci`, null tolerance ±2%
- `invocation_id = vs-pandas-20260805T005618.992734Z-pid2252383`
- raw row: `artifacts/bench/2s5hs_w4_vs_w8_vs_pandas_str_contains_arrow_1M_frankenlibc-test.json`

| Arm | p50 | CV | threads | A/A null median | null within ±2%? |
|---|---|---|---|---|---|
| pandas | 21252.81 µs | 10.36% | 1 | 0.986556 | ✅ yes |
| reference (8 thr) | 2128.69 µs | 16.38% | 8 | 1.030373 | ❌ no |
| candidate (4 thr) | 2837.87 µs | 15.27% | 4 | **0.912087** | ❌ no |

## Decision: REJECT — it fails BOTH acceptance criteria

The bead's contract was *"Accept only if the null passes AND throughput loss is
small."* Neither holds.

**(a) Does the 4-thread arm's A/A null pass? NO — and it is WORSE than the
8-thread arm's.** Candidate null median `0.912087` is 8.8% off unity against a
±2% limit. The reference's own null in the same invocation is `1.030373` (3.0%
off). Halving the workers did not move the null toward unity; it moved it
further away. Candidate null CI `[0.836681, 1.189906]`, log half-width
`0.178` — still more than twice pandas' `0.080`.

**(b) What does it cost? 33% of the throughput, which is not small.**
`candidate_vs_reference.ratio = 0.750` (`reference_p50 / candidate_p50`), i.e.
the 4-thread arm takes 2837.87 µs where 8 threads take 2128.69 µs — **1.333x
the time**. Amdahl was too optimistic here: on this workload the scan is not
sufficiently memory-bound for 4 threads to hold 8-thread throughput.

The candidate-vs-reference comparison is itself `NULL_UNDECIDABLE`
(`claim_log_effect 0.2875 < required_log_effect 0.3902`,
`effect_exceeds_two_x_null_margin = false`), so the 33% figure is directionally
clear but not a certified effect. It does not need to be: a lever whose null got
worse and whose throughput dropped has no path to acceptance.

Against pandas the candidate is 7.489x faster (2837.87 vs 21252.81 µs), but that
row is `NULL_UNDECIDABLE` and **no speedup is claimed**.

## What this refutes

The bead's supporting evidence was a 5/5 correlation between FP thread count and
A/A null failure (1 thread → PASS, 8-10 threads → FAIL). That correlation does
**not** survive a direct test: **4 threads still fails, at 0.912**. Thread count
alone is therefore not the mechanism. Two things follow.

1. **CV is not a proxy for the null.** Both arms sit at CV 15-16% here — a large
   improvement over the 45.7% / 61.3% recorded for the 8-thread arm on earlier
   hosts — yet both nulls still fail. The harness already says
   `cv_is_provenance_only`; this row is a concrete demonstration of why.
2. **A quiet host is necessary but not sufficient.** `frankenlibc-test` was at
   load 0.25 with every `host_wide_quiescence` phase `clear`, and the null still
   failed on both FP arms while pandas' passed at 0.9866. Whatever inflates FP's
   run-to-run spread is inside FP's parallel path, not ambient load — pandas'
   single-threaded arrow `contains` is stable on the same box in the same
   invocation.

That makes the remaining candidate mechanism the **spawn itself** (allocation,
scheduling, and teardown of a fresh `thread::scope` per call), not the number of
workers it spawns — which is exactly what `br-frankenpandas-vrjrf`'s reusable
pool attacks. This row does not settle vrjrf; it removes the cheap alternative
that was filed to avoid it.

## Retry predicate

Re-open ONLY if **both** hold:

1. A mechanism is identified that makes the FP A/A null land within ±2% while
   the per-call `thread::scope` spawn is still present — i.e. the null failure is
   shown NOT to be spawn-inherent. Absent that, this lever cannot succeed no
   matter what constant is chosen, because it does not remove the spawn.
2. A worker count between 4 and 8 is proposed with a stated reason to expect it
   to beat 8 on throughput. This row shows 4 < 8 on throughput, and 8 is the
   current default, so the interior of the interval needs an argument, not a
   sweep.

A pure re-run on a different host does **not** satisfy this. The host was quiet,
the gate was clear, and pandas' null passed beside FP's failing one.

## Method note

The first invocation of this A/B aborted at `invocation_postflight` with
`busy=[2, 6]` against the 20%-busy exclusivity limit. The cause was almost
certainly my own `ssh … tail/uptime/ps` progress-polling landing inside the
measurement window. **Do not poll the bench host while a row is running** — the
gate is host-wide, and an interactive SSH session is enough to trip it. The
re-run with no concurrent polling was clean (`HARNESS_EXIT=0`, every
`host_wide_quiescence` phase `clear`). The gate was not relaxed and no flag was
changed between the two runs.
