# Lane M `df_dot` packed/register-blocked routing A/B — 2026-07-27

This artifact records ProudChapel's strict-remote, same-worker test of the explicit
`br-frankenpandas-gv6eg` retry predicate. The candidate routed large finite `DataFrame::dot`
products away from the per-output-column lazy AXPY plan and into the existing packed-B, 4x4
register-blocked kernel. The candidate source was removed after the median-CI gate rejected it.

## Attribution and protocol

- Worker: `ovh-a` (`51.222.245.56`) for profile, baseline, candidate build, and candidate run.
- Build route: `RCH_REQUIRE_REMOTE=1 RCH_WORKER=ovh-a env -u CARGO_TARGET_DIR rch exec -- cargo
  build --profile release-perf -p fp-bench`.
- Workload: `fp-bench --category linalg --workload df_dot --size 100k --dtype float64`
  (`316 x 316` square product, all output columns materialized).
- Profile: `perf record -F 999 -g --call-graph dwarf`, followed by `perf report --no-children`.
  `<fp_columnar::ScalarValues>::materialize_float64_dot` carried **58.71% self-time** in the
  workload that was subsequently timed. The profile lost 61% of samples under host load, but the
  named target remained far above the 5% admissibility floor.
- Samples: 50 wall-time observations per whole-binary arm. Each invocation also emitted 25
  order-alternating A/A rounds from the operation under test.
- Decision: deterministic 10,000-resample bootstrap 95% CIs over medians. CV is provenance only.
  The claim must clear twice the wider A/A bootstrap-median log-CI half-width.

## Executing binary identity and parity

Both identities are line-one self-reports from the executing process:

| arm | executing ELF SHA-256 | bytes | checksum |
|---|---|---:|---|
| baseline lazy AXPY | `7e53ce12eea2c0c0d0f7b7fcb04917dbe58d134a4d7503a635c22da4eaa5d84e` | 70,153,648 | `4957dea0fe3e2ed1` |
| candidate existing packed/register-blocked route | `e636f8ccd9b2a28519f9d10e28e33309e92bfdf0c0fb6046041fd71e93e5ef87` | 70,151,784 | `4957dea0fe3e2ed1` |

The different executing-ELF hashes prove the source change reached the candidate binary. Equal
checksums prove workload-level numeric parity. In addition, a focused 104x104 candidate test
compared all 10,816 output cells against an explicit `l = 0..k` reference fold and passed every
`f64::to_bits()` comparison on `ovh-a`.

## Median-CI result

| statistic | baseline | candidate |
|---|---:|---:|
| wall p50 | 5469.922 us | 4621.283 us |
| A/A median ratio | 0.999283 | 0.870478 |
| A/A bootstrap 95% CI | [0.997229, 1.002048] | [0.750460, 1.138589] |
| A/A log half-width | 0.002775 | 0.287069 |

- Point effect, baseline/candidate: **1.183637x**.
- Independent bootstrap 95% CI for the median effect: **[0.980745, 1.387111]**.
- Claim log effect: **0.168592**.
- Required log effect (2x the wider A/A log half-width): **0.574138**.
- Combined 2x-null interval: **[0.563190, 1.775599]**.
- Verdict: **REJECT / NULL-UNDECIDABLE**. The nominal direction is favorable, but the effect CI
  crosses 1.0 and the claim is only 29% of the required median-CI gate.

The instability is attributable to the candidate route itself: it enters the eager kernel's
per-call scoped-thread fanout, whereas the baseline AXPY materializes output columns serially. A
single-CPU diagnostic made the candidate A/A substantially tighter but put its p50 near 6.67 ms,
slower than the 5.47 ms baseline. Affinity is not promoted to decision evidence because the
baseline arm was not rebuilt and rerun under that diagnostic setup.

<details>
<summary>Raw 50-sample wall vectors and 25-round A/A ratios</summary>

Baseline `times_us`:

```text
[6006.540, 8908.724, 6526.454, 7647.738, 5332.574, 5490.962, 5480.512, 6503.050,
 7273.295, 5384.111, 5395.793, 5375.976, 5386.145, 5396.956, 5428.975, 5406.153,
 5421.732, 5473.048, 5490.951, 5457.880, 5454.163, 5462.709, 5466.917, 5455.746,
 5456.848, 5452.150, 5449.063, 5458.801, 5466.856, 5443.753, 5476.075, 5472.086,
 5468.940, 5480.051, 5458.310, 5473.479, 5474.141, 5478.068, 5478.379, 5470.904,
 5456.828, 5473.389, 5488.798, 5463.170, 5491.042, 5483.538, 5503.666, 5478.648,
 5452.369, 6601.865]
```

Baseline A/A ratios:

```text
[0.674231, 0.853384, 0.971155, 0.842760, 1.350881, 1.003686, 0.997997, 1.004221,
 0.990624, 1.006059, 0.998436, 1.002048, 1.000862, 0.998216, 1.004244, 1.000729,
 0.997972, 0.997229, 0.999283, 1.001366, 0.996974, 1.004691, 1.001368, 1.004566,
 0.825883]
```

Candidate `times_us`:

```text
[5885.723, 6751.509, 8716.027, 3305.337, 3072.820, 3715.777, 4675.600, 6424.163,
 3412.268, 2996.928, 5601.409, 4135.236, 5086.772, 3906.506, 3326.767, 3480.716,
 3100.503, 3561.839, 3980.615, 3946.201, 4566.966, 4885.895, 3190.301, 5577.304,
 3069.704, 4386.988, 3853.236, 3203.566, 3121.332, 5651.623, 5322.485, 20509.694,
 7255.234, 6242.442, 10379.772, 4754.639, 5296.105, 6523.330, 5814.368, 7747.739,
 3407.870, 5701.367, 6949.430, 8386.468, 3239.403, 4397.898, 6845.735, 8217.361,
 4381.007, 3836.004]
```

Candidate A/A ratios:

```text
[0.871764, 2.636956, 0.826966, 0.727815, 1.138589, 1.354556, 1.302128, 0.955771,
 0.870478, 1.008721, 0.934725, 0.572015, 0.699729, 1.202796, 0.552289, 0.259511,
 1.162243, 2.183083, 0.811871, 0.750460, 0.597729, 0.828648, 0.736580, 0.833082,
 1.142076]
```

</details>

## Concrete retry predicate

Do not retry the current finite-to-fallback router, the existing 4x4 packed-B kernel, worker-cap
tuning, CPU affinity, or another compiler-flag sweep. Reopen `df_dot` only for a genuinely new
packed-panel microkernel that first satisfies both of these preconditions:

1. an isolated same-core counter harness shows at least **1.25x fewer cycles** than the current
   AXPY kernel for the exact 316x316 fold while preserving every output `to_bits()` value; and
2. its production invocation uses amortized/persistent scheduling (not per-call OS-thread spawn)
   and demonstrates an A/A `required_log_effect < 0.08` before an A/B verdict is assigned.

The next implementation may use a 4x8-or-wider register tile with both A and B panels packed, but
the tile shape itself is not evidence; the counted-cycle and production-null predicates above are.
