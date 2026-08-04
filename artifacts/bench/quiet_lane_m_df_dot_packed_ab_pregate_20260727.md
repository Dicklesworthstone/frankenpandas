# Lane M `df_dot` packed-A+B 4x8 counted-cycle pregate — 2026-07-27

This artifact records ProudChapel's execution of the counted-mechanism retry
predicate from `br-frankenpandas-2z3ar`. The experiment stopped before any
production routing change because it did not meet the predeclared cycle gate.

## Scope and protocol

- Worker: `ovh-a`; strict remote compilation via
  `RCH_REQUIRE_REMOTE=1 RCH_WORKER=ovh-a env -u CARGO_TARGET_DIR rch exec --`.
- Build: `cargo test --profile release-perf -p fp-columnar`.
- Exact shape: `316 × 316 × 316`, matching `df_dot @100k`.
- Baseline: the current full-matrix AXPY materialization, one ascending-`l`
  fold per output cell.
- Candidate: a new 4x8 register tile. It packed column-major A into row-major
  A and packed column-major B into 8-column panels on every invocation.
  Candidate output allocation, both pack operations, computation, and output
  destruction were all inside the counted region.
- Isolation: both arms ran from the same test executable, pinned to CPU 0.
  Each arm executed 256 complete products per counter sample; `perf stat -r 7`
  ran seven samples under non-interactive elevated counter access.
- Counters: user cycles, retired instructions, task clock, migrations, and
  context switches. The first unprivileged counter attempt failed before
  either arm ran because `perf_event_paranoid=4`; it has no verdict role.

## Behavior proof

The baseline and packed kernel each accumulated every cell in ascending
`l = 0..316` order. All **99,856** output cells matched by `f64::to_bits()`.
The deterministic reference checksum was `fedb831369ba039e`.

## Counted result

| statistic, 256 products | current AXPY | packed A+B 4x8 | AXPY / packed |
|---|---:|---:|---:|
| user cycles | 5,638,364,969 (±0.43%) | 4,732,693,852 (±0.35%) | **1.191365x** |
| cycles / product | 22,024,863 | 18,487,085 | **1.191365x** |
| retired instructions | 27,189,764,385 | 24,539,207,213 | 1.108013x |
| task clock | 1,269.316 ms | 1,230.604 ms | 1.031457x |
| CPU migrations | 0 | 0 | — |

The median in-test elapsed vectors were 1,250,350,636 ns for AXPY and
1,210,150,123 ns for packed A+B, a 1.033219x wall direction. Wall time is
metadata here; the predeclared decision gate is counted cycles.

## Decision

**COUNTED PREGATE FAILED; production candidate not admitted.** The candidate
removed 16.06% of cycles, but the required `1.25x` ratio means it had to remove
at least 20%. At the observed AXPY count, admission required at most
4,510,691,975 cycles per 256 products (17,619,891 cycles/product). The packed
kernel used 4,732,693,852 cycles, missing the limit by 222,001,877 cycles
(867,195 cycles/product).

No production source was edited. The temporary ignored counter test compiled,
proved parity, produced the counters above, and was then removed.

## Concrete retry predicate

Do not retry the existing packed-B 4x4 router, this packed-A+B 4x8 loop,
worker affinity, or compiler flags. Reopen this vein only for a materially
different safe-Rust schedule with a counted mechanism that predicts removal of
at least another 4.7% of the current candidate cycles, and admit it only if the
same-core exact-shape counter test reaches **≤17,619,891 cycles/product** with
all 99,856 output cells still bit-identical. A tile-width change alone is not
such a mechanism.
