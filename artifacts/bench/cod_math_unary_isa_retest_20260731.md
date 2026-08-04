# Math-unary x86-64-v3 satisfied-retry re-test

**Agent:** CyanLynx  
**Date:** 2026-07-31  
**Bead:** `br-frankenpandas-h67zz`  
**Historical retry row:** `docs/NEGATIVE_EVIDENCE.md`, 2026-06-26
BlackThrush, "typed_float_unary generic+move"  
**Machine-readable evidence:**
`cod_math_unary_isa_retest_20260731.json` (SHA-256
`1cd248251866ea5d827d1734c6a6bd63ddde39836b05407afa1f003e3a2ddc17`)

## Result

The build-target retry predicate is satisfied and changes four competitive
verdicts. With an x86-64-v3 whole-binary candidate, the exact 1M Float64 rows
are corrected-gate wins against live pandas 2.2.3 for `floor` (**1.285x**),
`ceil` (**1.452x**), `trunc` (**1.303x**), and `round(2)` (**1.098x**).
`sqrt` (**0.361x**) and `log` (**0.630x**) remain losses.

This is not evidence for a blanket global target change. The v3 candidate
beats the default-target control on five rows, including `sqrt`, but regresses
`log` to **0.758x** of the default binary. No Cargo target policy is changed.
The four winning rows are retained as exact-build competitive results; the two
losing rows retain concrete source/runtime-dispatch retry predicates below.

## Live-incumbent rows

Times are microseconds. Each row is a separate invocation containing live
pandas, the immutable default-target binary, and the immutable v3 candidate.
Every ratio below passed all three corrected clauses: the independent
bootstrap effect-median 95% CI excludes 1, the absolute log effect exceeds two
times the widest A/A null log-CI half-width, and both engine A/A medians are
within 2% of unity. CV is provenance only.

| workload | v3 p50 / p95 / p99 | pandas p50 / p95 / p99 | pandas/v3 | effect-median 95% CI | v3 / pandas A/A medians | observed threads v3 / pandas | decision |
|---|---:|---:|---:|---:|---:|---:|---|
| floor | 123.13 / 151.55 / 210.25 | 158.21 / 208.78 / 218.86 | **1.285x** | [1.277046, 1.339627] | 0.996101 / 0.998843 | 1 / 1 | KEEP |
| ceil | 123.72 / 153.99 / 167.77 | 179.61 / 200.63 / 228.79 | **1.452x** | [1.442989, 1.461650] | 0.990558 / 1.003823 | 1 / 1 | KEEP |
| trunc | 122.87 / 165.46 / 195.00 | 160.07 / 186.49 / 209.96 | **1.303x** | [1.273292, 1.341008] | 0.998107 / 0.999936 | 1 / 1 | KEEP |
| round2 | 474.08 / 484.46 / 492.84 | 520.73 / 586.61 / 626.25 | **1.098x** | [1.083005, 1.102684] | 0.989954 / 1.000172 | 1 / 1 | KEEP |
| sqrt | 2763.56 / 2993.11 / 2999.10 | 998.65 / 1145.48 / 1166.09 | **0.361x** | [0.357630, 0.364423] | 0.996720 / 0.992018 | 8 / 1 | REJECT |
| log | 5029.33 / 5275.46 / 5310.37 | 3166.55 / 3198.69 / 3351.14 | **0.630x** | [0.625947, 0.638700] | 1.001090 / 1.000209 | 3 / 1 | REJECT |

The thread column is the operation's actual observation, not the requested
ten-CPU affinity. `runtime_available_parallelism` was ten for every engine.
The four cheap v3 operations stayed serial. `sqrt` observed eight v3 threads;
`log` observed three. pandas observed one operation thread on every row.

## Whole-binary default control

The default binary ran in the same invocation as both the candidate and live
pandas. All default-vs-pandas and candidate-vs-default comparisons also passed
the corrected three-clause gate.

| workload | default p50 / p95 / p99 us | default/pandas | v3/default | default A/A median | observed threads default |
|---|---:|---:|---:|---:|---:|
| floor | 1335.90 / 1343.52 / 1433.88 | 0.118x | **10.849x** | 0.999894 | 1 |
| ceil | 1452.67 / 1782.16 / 1787.71 | 0.124x | **11.742x** | 0.999654 | 1 |
| trunc | 1663.46 / 1671.19 / 1677.14 | 0.096x | **13.539x** | 1.000997 | 1 |
| round2 | 1128.85 / 1229.45 / 1285.39 | 0.461x | **2.381x** | 0.999698 | 1 |
| sqrt | 3988.10 / 4076.26 / 4118.91 | 0.250x | **1.443x** | 0.989529 | 7 |
| log | 3814.33 / 3883.05 / 4504.63 | 0.830x | **0.758x** | 0.995795 | 8 |

Candidate/default output liveness tokens match on every row. They are not
used as cross-engine content hashes; semantic evidence comes from the existing
scalar-oracle/null-edge tests and the exact v3 conformance run recorded under
Validation.

## Exact input and compatibility controls

The previous Python arm used NumPy's RNG with a different seed, so it sampled
the same distribution but not the same values. The harness now reproduces the
Rust `SplitMix64(0x123456789abcdef0)` stream bit-for-bit. Its self-test compares
257 vector-generated values with a scalar Rust-equivalent implementation.
Every value is positive and overwhelmingly non-integral, preventing the
integral floor/ceil/trunc identity witness from short-circuiting and preventing
sqrt/log from entering a NaN path.

The target flag changes code generation, not the API or null contract. Existing
edge coverage includes scalar-oracle floor/ceil/trunc, signed zero/infinity,
nullable sqrt/log NaN-to-missing behavior, and half-even rounding. No source
kernel or compatibility behavior changed in this deliverable.

## Provenance

Both ELFs were built on `vmi1227854` from exact base
`e7eb7b5a0edcbf16a891f59d0b80ef85d2c0d932` with strict remote execution,
`--base`, `--clean-overlay`, and `--no-overlay`; local fallback was disabled.
The current HEAD has no differences from that base in Cargo manifests,
toolchain/configuration, or `crates/**`.

```text
default command:
  CARGO_TARGET_DIR=/data/tmp/cargo-target RCH_WORKER=vmi1227854 \
  RCH_REQUIRE_REMOTE=1 RCH_NO_SELF_HEALING=1 rch exec \
  --no-self-healing --base e7eb7b5a0edcbf16a891f59d0b80ef85d2c0d932 \
  --clean-overlay --no-overlay -- \
  cargo build -j1 -p fp-bench --profile release-perf

v3 command:
  CARGO_TARGET_DIR=/data/tmp/cargo-target RCH_WORKER=vmi1227854 \
  RCH_REQUIRE_REMOTE=1 RCH_NO_SELF_HEALING=1 rch exec \
  --no-self-healing --base e7eb7b5a0edcbf16a891f59d0b80ef85d2c0d932 \
  --clean-overlay --no-overlay -- env RUSTFLAGS='-C target-cpu=x86-64-v3' \
  cargo build -j1 -p fp-bench --profile release-perf
```

| artifact | in-process SHA-256 | bytes | remote elapsed |
|---|---|---:|---:|
| default fp-bench | `534600c75708162e351e56d803187ef9204ec855b53009185b199ac626cf0b68` | 75,019,416 | 15m57s |
| x86-64-v3 fp-bench | `97d30b363332e7f687ea74331973461fade550898202f988b1f4bdc34cd545ee` | 75,329,736 | 16m17s |
| Python 3.13.7 | `efb29ce53d36ebaeee80e3aa44fd6c7f9d71bbded5fe1665240b2ed8ecaeee0e` | 6,894,448 | n/a |
| harness | `b9affde33c6e14a19ad118d6d3f0252c72fd38ee4cc9983517f5d244ebf859d4` | 141,699 | n/a |

pandas 2.2.3 artifact SHA-256 was
`a5747a243aba6bcbe01fdb4bcfd77c2dea7ac4ca801f7b1cf32cc8d866a31489`;
pyarrow 24.0.0 was
`796c9609389368aceb57b06402d05cc781dc2387f6c23f3121f0332184fdc330`.
All executables self-reported their identity from inside the process.

Whole-ELF disassembly supplies the mechanism, not the verdict: the default
binary contains zero `vroundpd` and `vsqrtpd`, while v3 contains 17 and 7,
respectively. The default contains four legacy `sqrtpd` instructions; v3
contains zero. Neither binary contains `vfmadd*`, so the exact fixture generator
does not introduce a fused-arithmetic mismatch.

## Measurement host and admission

All accepted rows ran on `threadripperje`, AMD Ryzen Threadripper PRO 5995WX,
64 physical / 128 logical cores, SMT active, one NUMA node, kernel
`6.17.0-41-generic`, 536,069,869,568 bytes RAM, performance governor. The
process affinity was CPUs 0-9, while the admission gate sampled all 128 online
CPUs. Fifty-four adjudicating checkpoints all cleared; the maximum observed
busy fraction was 16% against the predeclared 20% ceiling.

The harness admits before hashing its own 228 MiB pandas/pyarrow provenance,
then requires two consecutive one-second clear samples after self-induced
setup work. Readiness probes remain in the artifact but do not vote. The second
consecutive clear sample is promoted to the adjudicating checkpoint; immediate
post-arm checks still fail closed. A deterministic `clear, blocked, clear,
clear` sequence test prevents a transient clear probe from bypassing this
rule.

Earlier attempts on `vmi1149989` and `vmi1152480` were rejected when unrelated
Rust work appeared at a checkpoint. They emitted no result JSON and contribute
no timing or ratio.

## Validation

- `python3 -m py_compile benches/vs_pandas_harness.py`
- `python3 benches/vs_pandas_harness.py --corrected-null-gate-self-test`
- `python3 benches/vs_pandas_harness.py --host-exclusivity-self-test`
- exact x86-64-v3 fp-columnar conformance command through strict RCH: recorded
  in the closeout after completion
- `git diff --check`
- `ubs benches/vs_pandas_harness.py`

## Decisions and retry predicates

- **KEEP** the four exact v3/live-pandas wins: floor, ceil, trunc, and round2.
- **REJECT** a global x86-64-v3 Cargo policy from this evidence because the
  candidate regresses log and still loses sqrt/log to pandas.
- **RETRY sqrt** only after a profile proves whether worker setup, the serial
  output validity scan, or another post-map cost dominates the now-wide sqrt
  kernel; remove the largest source-parallelizable residual, then rerun this
  same live-incumbent contract.
- **RETRY log** only after a profile explains why the v3 binary observed three
  workers and regressed against the eight-worker default, or after a
  runtime-dispatched vector-log implementation exists. Another compile-target
  flag without that evidence does not satisfy the predicate.

