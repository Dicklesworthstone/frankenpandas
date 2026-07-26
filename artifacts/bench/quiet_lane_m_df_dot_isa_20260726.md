# Lane M `df_dot` whole-binary ISA A/B — 2026-07-26

This artifact banks QuietHarbor's strict-remote, same-worker re-decision of the historical
`df_dot` ISA blocker. The decision statistic is wall time; instruction count has no gate role.

## Protocol

- Worker: `ovh-a` for every admitted arm.
- Build/run route: `RCH_REQUIRE_REMOTE=1`, with `CARGO_TARGET_DIR` unset.
- Arm A: default compiler target.
- Arm B: `RUSTFLAGS=-C target-cpu=x86-64-v3`.
- Samples: 50 per arm.
- Null: each arm's `fp-bench` invocation emitted its own 25-round order-alternating A/A control.
- Decision: deterministic 10,000-resample bootstrap 95% CI over median ratios.
- Parity: `checksum=4957dea0fe3e2ed1` in both `df_dot` arms.

The first `df_abs` A arm landed on `hz2` and was discarded before analysis. The table below uses
only the replacement A arm and B arm that both ran on `ovh-a`.

## Executing binary identity

The identities below are the first-line self-reports from the executing process, not hashes
computed by an adjacent shell:

| workload | arm | `bench_elf_sha256` | bytes |
|---|---|---|---:|
| `df_dot` | A default | `bdc765dd38ce7bca09c7575dfe8546ec316e0d7ad4f5a4260082349ca836d6dc` | 69,714,072 |
| `df_dot` | B x86-64-v3 | `ae9cfc41b9e3861b7ad301de72790af4845ca7d690d41eb3d3290c9efffa3d98` | 69,714,112 |
| `df_abs` | A default, matched rerun | `b08554359e5ebd17737c6aa38097a4ef973db01d2f317793a57d2aa3bc4239d1` | 70,125,424 |
| `df_abs` | B x86-64-v3 | `01022db95abaaf0bdfbbaf2b14f84c9862b12ad86fe704b3b43e6071469b6073` | 69,718,528 |

Same source plus same worker plus different arm SHA proves that the compiler flag changed the
whole binary.

## Wall-time decision

| workload | shape | A median | B median | effect A/B | bootstrap 95% CI | A/A floor | decision |
|---|---|---:|---:|---:|---|---:|---|
| `df_dot` | 100k | 5239.1 µs | 5071.3 µs | **1.0327x** | [1.0240, 1.0376] | ±0.24% | **DECIDABLE**; v3 is faster |
| `df_abs` | 1M | 6709.9 µs | 6452.5 µs | 1.0071x | [0.9870, 1.0261] | ±0.81% | **NULL-UNDECIDABLE** |

The historical cross-worker `1.4x` AVX2 claim is withdrawn. The admissible same-worker result is
3.27%, while the v3 arm remains about 4.1x slower than pandas' recorded single-thread 1229 µs.

## Verdict and retry predicate

The old “flag cannot be built remotely” blocker is resolved, but x86-64-v3 is not the missing
`df_dot` lever. Do not run another compiler-flag sweep. Reopen the runtime surface only for a
hand-written, register-blocked and packed-panel GEMM microkernel, with numeric parity, same-worker
whole-binary A/B, in-process ELF identities, and an in-invocation A/A median-CI gate.
