# br-frankenpandas-csapr — string-kernel spawn: serial vs parallel

**Status: MEASURED. First DECIDABLE, null-passing vs-pandas row of this campaign.
Threshold NOT changed — parallel is raw-faster; serial is the measurement
instrument.**

Agent: MossyMeadow (claude-code / opus-5). Date: 2026-08-04.
Host `frankenlibc-test` (vmi1152480, drained), AMD EPYC, 10 physical / 10
logical. Workload `str_startswith_arrow @ 1M/float64`, live pandas 2.2.3 +
pyarrow 24.0.0 in the SAME invocation as each FP arm.

## Arms

Two trees from HEAD `a4a08fbd5` differing ONLY in
`crates/fp-frame/src/lib.rs` — `apply_str_bytes_bool`'s `PARALLEL_MIN_BYTES`,
`8 MiB` (ships) vs `usize::MAX` (forced serial). Both carry the i7znp byte path,
so the ONLY variable is the per-call `thread::scope` spawn.

| arm | ELF SHA-256 | threads actually used |
|---|---|---:|
| parallel (ships) | `a41869d5814b5334…` | **7** |
| serial (experiment) | `1797503325e967b2…` | **1** |

Distinct SHA-256. Thread count is the content check — a comment marker does not
survive into a binary, so `thread_count_actually_used` from the harness's own
provenance is the proof each arm is what it claims.

## Result

| arm | verdict | FP p50 | FP CV | pandas p50 | ratio vs pandas | CI95 | **FP A/A null** |
|---|---|---:|---:|---:|---:|---|---:|
| **serial (1 thr)** | **FASTER** | 4518.0 us | **21.1%** | 5965.9 us | **1.320x** | [1.280, 1.424] | **0.9992 PASS** |
| parallel (7 thr) | NULL_UNDECIDABLE | 2481.6 us | 45.7% | 5939.9 us | 2.394x | [1.962, 2.615] | 1.0359 FAIL |

Serial gate clauses — **all three true**:
`effect_ci_excludes_unity`, `effect_exceeds_two_x_null_margin`,
`null_medians_within_2pct_unity`. `decidable_workloads: 1`,
`null_undecidable_workloads: 0`.

(`summary.claim_validated: false` on both is NOT a row verdict — it requires
EVERY category to score >1.0 and only `strings` was run. The per-row verdict is
the decision.)

## What this establishes, and what it does not

**ESTABLISHED — the str.startswith 0.42x LOSS is refuted.** Single-threaded FP
is a decidable **1.320x FASTER** than single-threaded pandas, null passing at
0.9992. Both engines at 1 thread, so this is an apples-to-apples core-for-core
comparison. The standing 0.42x figure did not reproduce on any of the four rows
measured across i7znp and csapr.

**NOT ESTABLISHED — that serial beats parallel.** It does not. Raw p50 is
2481.6 us parallel vs 4518.0 us serial, i.e. the shipped parallel path is
~1.82x faster in raw median. The parallel row is merely *undecidable*, not
*worse*. Hypothesis (a) from the bead — "the spawn may be net-negative at this
size" — is **REFUTED**; the spawn pays for itself on throughput.

**ESTABLISHED — hypothesis (b), the measurement unblock.** Going serial moves
the FP A/A null from 1.0359 (FAIL) to 0.9992 (PASS) and halves CV, 45.7% ->
21.1%. That is the difference between an undecidable row and a decidable one.
The blocker on measuring FP string kernels was never the exclusivity gate or
fleet disk pressure — it is the per-call `thread::scope` spawn's own variance.

## Decision

**Do NOT raise `PARALLEL_MIN_BYTES`.** It would trade ~1.82x raw throughput for
decidability. The threshold stays at 8 MiB; no code change lands from this bead.

Two things this buys instead:

1. **A measurement instrument.** A forced-serial build produces decidable,
   null-passing rows on this fleet. Any future string-kernel lever can be
   measured serially for a *valid* ratio, then separately checked for parallel
   throughput. This is how to get numbers out of a noisy shared VPS without
   touching the gate.
2. **A sharpened next lever.** The spawn is worth its cost but not its variance.
   A reusable pool (instead of per-call `thread::scope`) would keep the ~1.82x
   and cut the jitter that makes the shipped path unmeasurable. That is the
   remaining move, and it is now backed by measurement rather than a hunch —
   see [[parallel-per-call-overhead-ledger]] (`thread::scope` ~397 us/8thr).

## Provenance

Raw rows: `artifacts/bench/csapr_serial_1thread_vs_pandas_1M.json`,
`artifacts/bench/csapr_parallel_7thread_vs_pandas_1M.json`. Both carry each
executable's SHA-256 self-reported from inside the measuring process, the
host-wide exclusivity observations, and the 25-round A/A null series.

Gate untouched. Serial arm accepted on attempt 11, parallel on attempt 3, under
the same split-invocation shape (four no-retry `require_quiet` samples per
invocation) documented in
[[br-frankenpandas-i7znp_str_prefix_byte_path]].
