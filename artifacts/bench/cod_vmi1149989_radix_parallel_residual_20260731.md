# Parallel radix residual — `br-frankenpandas-j5841`

Decision: **KEEP** as a maintenance self-speedup.

The accepted same-host, same-input, exact-base A/B is 1.4750x faster
(`508.580 ms / 344.811 ms`), with a 95% bootstrap median-ratio CI of
`[1.4289, 1.5333]`. The corrected three-clause null gate passes. This does
not by itself replace the separately gated live-pandas headline.

## Exact build identity

- Base commit: `7a5bf7143bc996d7956dc5faf38628a71390c331`
- Build worker: `vmi1264463`
- Measured-build contract:
  `CARGO_TARGET_DIR=/data/tmp/cargo-target RCH_REQUIRE_REMOTE=1 RCH_NO_SELF_HEALING=1 rch exec --no-self-healing --base 7a5bf7143bc996d7956dc5faf38628a71390c331 --clean-overlay --overlay-path crates/fp-columnar/src/lib.rs -- cargo build -j1 -p fp-bench --profile release-perf`
- Clean confirmation contract:
  `CARGO_TARGET_DIR=/data/tmp/cargo-target RCH_BUILD_TIMEOUT_SEC=3600 RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 RCH_NO_SELF_HEALING=1 rch exec --no-self-healing --base 7a5bf7143bc996d7956dc5faf38628a71390c331 --clean-overlay --overlay-path crates/fp-columnar/src/lib.rs -- cargo build -j1 -p fp-bench --profile release-perf`
  exited zero; Cargo reported `39m 52s` and RCH reported `2769.7s`
  end-to-end.
- Candidate source SHA-256:
  `286d23ffe4116360ba758e62e741451036d176fb3739280d458ab037ce9b36b6`
- Baseline ELF SHA-256:
  `4aa06eda20230eb79ff02d29ba00f9cba4839db8451034cca9cd84c19a852f8d`
  (`73,771,208` bytes)
- Candidate ELF SHA-256:
  `2bb3b92f2e5ccaa258868ce1613c9e0c2716f31bc29f7dfa6e946ea6c894a5d3`
  (`73,873,472` bytes)

## Serial-residual classification

The exact-current baseline profile used `perf record -F 999 --all-user` around
the 10M Float64 `sort_values_single` invocation. Its main thread held 20.703%
of aggregate cycles. These are the main-thread entries, classified by
dependency rather than by whether the current implementation happened to use
threads:

| Baseline main-thread entry | Aggregate cycles | Classification | Reason |
|---|---:|---|---|
| `radix_argsort_u64` | 17.61% | Mixed | The eight stable LSD digits are inherently ordered; digit `n+1` consumes digit `n`. Within a digit, histogram and stable scatter work is parallelizable with precomputed disjoint offsets. |
| `typed_dense_sort_order` | 1.60% | Not yet parallelized/eliminated | Typed key construction and result assembly are element-independent; dispatch itself is tiny and sequential. |
| `Index::take` | 0.46% | Parallelizable | Output positions are independent gathers once the permutation exists. |
| `build_frame` | 0.19% | Outside timed operation | Fixture construction is setup provenance, not an operation residual. |
| kernel faults / `memset` / `memmove` | 0.20% combined | Parallelizable memory service | Pages and ranges are independent, but this is a much smaller lever and partly kernel/vector-library owned. |
| executable SHA-256 | 0.04% | Outside timed operation | Provenance hashing is outside the benchmarked sort. |
| `Column::from_f64_values` | 0.02% | Outside timed operation | Fixture construction precedes the operation. |
| single-owner drop | 0.01% | Inherently sequential per allocation | The final owner performs one destruction/deallocation; negligible here. |

Inside baseline `radix_argsort_u64`, instruction samples attributed 73.26% to
stable scatter, 20.57% to histograms, and 6.17% to initialization, prefix, and
copy work. Stable scatter was therefore the largest genuinely parallelizable
serial residual.

## Lever

One stable high-16-bit scatter establishes globally ordered, disjoint prefix
buckets. Scoped workers then perform the six lower-byte stable LSD passes
inside those disjoint slices, using paired ping-pong buffers. The implementation
uses no unsafe code, shared output writes, or atomics. A single non-trivial
prefix falls back to the existing serial path because it has no useful
bucket-level parallelism.

The pass-to-pass dependency remains sequential within each prefix. What moved
off the main thread is the independent work across prefixes, including the
largest lower-byte scatter component.

## Post-change profile

Profile rows carry host identity. The baseline and candidate profiles are used
only as routing evidence because they ran on different workers; the acceptance
timing below is same-host.

| Profile | Host | perf-data SHA-256 | Samples | Lost | Main-thread share | Key symbols |
|---|---|---|---:|---:|---:|---|
| baseline | `vmi1227854` | `4dd90abec4295f3eede2394ab98207915b80c611d0ed9087978fcfea96ec7519` | 96,198 | 0 | 20.703% | main `radix_argsort_u64` 17.61% |
| candidate | `vmi1149989` | `1ac8a0f021d40383b295ae15e7999098d5b57c442035d3d119e48bc5e434b241` | 87,432 | 0 | 5.560% | main `radix_argsort_u64` 3.203%; worker `radix_scatter_entries` 6.69% aggregate |

The candidate routing profile used the algorithm-identical pre-lint ELF
`df5aff7e7f0c79190182c5ecd1cbe5040eb716b12dbb452d112d57b72c0af346`.
The accepted timing below uses the final exact-source ELF
`2bb3b92f2e5ccaa258868ce1613c9e0c2716f31bc29f7dfa6e946ea6c894a5d3`;
the cross-host profile remains routing evidence only.

The candidate's aggregate leader is now parallel `Column::take_positions`
(84.92%), while the main-thread critical share is 5.560%. The targeted radix
serial residual was removed; a subsequent vein should address gather work
rather than retest lower-byte radix scatter. Follow-up
`br-frankenpandas-qm0bm` requires an inside-gather bandwidth/allocation profile
before any nested-parallelism change.

## Accepted maintenance A/B

Command:

```text
AB_ROUNDS=16 python3 /data/tmp/cod-j5841-radix-ab.py \
  /data/tmp/fp-bench-cod-radix-base-7a5bf7143 \
  /data/tmp/cod-j5841-final-target/release-perf/fp-bench
```

| Host | CPU / topology | Kernel | Arm | p50 | Samples | Observed operation threads | Checksum |
|---|---|---|---|---:|---:|---:|---|
| `vmi1149989` | AMD EPYC Processor (with IBPB), 10 physical / 10 logical, SMT inactive | `6.17.0-40-generic` | exact-base baseline | 508.580 ms | 800 | 10 | `4957dea0fe3e2ed1` |
| `vmi1149989` | AMD EPYC Processor (with IBPB), 10 physical / 10 logical, SMT inactive | `6.17.0-40-generic` | candidate | 344.811 ms | 800 | 10 | `4957dea0fe3e2ed1` |

- Median ratio, baseline/candidate: **1.474951**
- 95% bootstrap median-ratio CI, 20,000 resamples:
  **[1.428928, 1.533264]**
- All sixteen interleaved per-round ratios exceed one:
  `[1.4487, 1.5832, 1.5544, 1.3983, 1.2194, 1.3900, 1.5489, 1.7060,`
  `1.4625, 1.5554, 1.3714, 1.4392, 1.3417, 1.5754, 1.5600, 1.4885]`
- Corrected clause 1: effect CI excludes `1.0` — **pass**
- Corrected clause 2: effect deviation `0.474951` exceeds two times the
  null-CI half-width `0.061852` — **pass**
- Corrected clause 3: A/A median `1.002850` is only `0.2850%` from unity —
  **pass**
- CV (`42.07%` baseline, `57.77%` candidate) is provenance only and does not
  gate.

The first final-ELF invocation is retained as rejected evidence rather than
discarded: eight rounds gave 1.383108x with CI `[1.336659, 1.447884]`, but its
A/A median was `1.023310`, 2.3310% from unity, so corrected clause 3 failed.
After the host's one-minute load returned below 1 and with no RCH work on the
host, exactly one predeclared confirmatory retry doubled sampling to sixteen
rounds. The accepted row above is that higher-information retry; no third
attempt was run.

The requested affinity cap was 10 logical CPUs. The evidence headline uses
the **observed** operation count: 10 threads in both arms.

Machine-readable summary:
`artifacts/bench/cod_vmi1149989_radix_parallel_residual_20260731_ab.json`.

## Semantic guard

Null/NaN placement remains outside the radix helper: existing nullable sort
logic filters valid keys, applies the stable order, then reattaches missing
positions according to the requested null placement. The new helper preserves
exact-key stability. Its focused test uses 257 high prefixes and a 4,096-value
low-key domain to force both parallel fan-out and exact ties, compares against
Rust's stable reference sort, and verifies the single-prefix serial fallback.

## Validation and disk discipline

- The final exact-base, clean-overlay `release-perf` confirmation build exited
  zero on `vmi1264463`; local fallback was disabled.
- Strict-remote workspace check passed on `vmi1153651`.
- Strict-remote final-source tests passed: `fp-columnar` 593 passed / 57
  intentionally ignored / 0 failed; `fp-conformance` 1,596 passed / 0 failed.
- Workspace Clippy reached the known 25 pre-existing `fp-columnar` errors under
  `-D warnings`; none points into the owned radix hunk. Focused rustfmt and
  `git diff --check` pass.
- The bounded final-file UBS scan reproduced the broad inventory (52 critical,
  6,587 warnings, 3,122 informational), found no unsafe block, and reported no
  new focused radix finding.
- Every Cargo invocation set the single reusable
  `CARGO_TARGET_DIR=/data/tmp/cargo-target`; no per-run local target suffix was
  created.
- At `2026-07-31T06:36:06Z`, `/data` had `283,651,051,520` bytes free and the
  live shared target occupied `70,171,809,361` bytes. A transient set of local
  `rustc` processes for peer-owned FrankenMermaid work appeared during the
  closeout recheck and had exited by the immediate follow-up; neither the
  processes nor their shared artifacts were touched.
- Positively identified superseded scratch was moved off `/data`, not deleted:
  `cod-j5841-corrected-null-wrapper.py` (1,587 bytes),
  `cod-j5841-radix-ab.py` (8,455 bytes), and
  `cod-j5841-final-fp-bench` (73,871,896 bytes), for exactly 73,881,938 bytes
  reclaimed. The user-named `cargo-target-h2-receipts`,
  `cargo-target-h1-continuous`, and `cargo-target-p8-retry` directories were
  already absent when inspected. The live shared target and peer-owned
  `cargo-target-franken-whisper` / `cargo-target-h3-scaling` were left intact.

## Competitive status

No new live-pandas headline is accepted unless one canonical same-invocation
row clears host-wide tenancy, effect decisiveness, and the corrected A/A-unity
clause for both engines. Rows that clear the old log-band gate but fail the
corrected null clause are diagnostic only.

Six complete rows cleared every host-wide checkpoint, but every row had at
least one engine A/A median more than 2% from unity:

| Host identity | pandas / FP diagnostic ratio | Observed operation threads, FP / pandas | FP A/A median | pandas A/A median | Corrected verdict |
|---|---:|---:|---:|---:|---|
| `vmi1149989` | 7.745x | 10 / 1 | 0.916468 | 0.985557 | reject |
| `vmi1149989` | 6.391x | 10 / 1 | 1.072611 | 0.988726 | reject |
| `vmi1149989` | 7.120x | 10 / 1 | 1.071710 | 0.992730 | reject |
| `vmi1227854` | 7.664x | 10 / 1 | 1.114704 | 1.043016 | reject |
| `vmi1227854` | 7.882x | 10 / 1 | 0.971774 | 1.053060 | reject |
| `vmi1227854` | 9.014x | 10 / 1 | 0.894001 | 0.989966 | reject |

`vmi1227854` was the same AMD EPYC 10-physical/10-logical, SMT-inactive
topology with 63,196,905,472 bytes RAM and kernel `6.17.0-35-generic`.
The canonical harness SHA-256 was
`b3e644a0eaced8b9e9b95461f91020c744dfbe640120e6d9b5c55a8702b7b29d`;
the corrected-gate wrapper SHA-256 was
`5ea19c2cefe4f4c216178f83210b2d4b3cdcefa4ccb4bbecfece20b668ab3c8e`.
These rejected competitive rows used the algorithm-identical pre-lint
candidate ELF
`df5aff7e7f0c79190182c5ecd1cbe5040eb716b12dbb452d112d57b72c0af346`;
they are not promoted by the final maintenance result.
The observed counts above, not the ten-CPU affinity request, are reported.
No new live-pandas ratio is published from these rejected rows.
