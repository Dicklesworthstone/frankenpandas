# Lane M pivot-family live-incumbent gate — 2026-07-28

## Outcome

At `c9267880d`, `df_pivot` and `df_pivot_table` were run side-by-side with
pandas 2.2.3 in the same harness invocation at 1M and 10M rows. Across three
self-identifying FrankenPandas ELF binaries, eight of nine rows were
median-CI-decidable wins and one was null-undecidable. Every point estimate was
above 1.0x.

The competitive win is reproducible, but the ratio-growth hypothesis is not.
The local ELF's ratios increased with N, while the complete strict-remote ELF's
ratios decreased with N. The public claim therefore reports the current
strict-remote measurements and does not describe this surface as a
scale-amplified Class-1 result.

## Strict-remote complete gate

Worker: `vmi1264463`

Invocation:
`vs-pandas-20260728T230137.754366Z-pid3096432`

FrankenPandas ELF:
`a4178caffbf7cf26b99f162dd68646c2994a37015a94204b52f99f0809a0d1d5`
(70,294,112 bytes)

pandas 2.2.3 artifact:
`051be80fe43b4e0be4e04af314c42db966950eb877b0634d482099f42535e9bb`

| workload | size | FP p50 | pandas p50 | ratio | combined 2x A/A interval | effect / required | verdict |
|---|---:|---:|---:|---:|---|---:|---|
| `df_pivot` | 1M | 55.980 ms | 135.162 ms | 2.414x | [0.819835, 1.219758] | 0.88148262 / 0.19865211 | FASTER |
| `df_pivot` | 10M | 1,215.589 ms | 1,398.645 ms | 1.151x | [0.695187, 1.438462] | 0.14027514 / 0.36357417 | NULL_UNDECIDABLE |
| `df_pivot_table` | 1M | 25.042 ms | 65.996 ms | 2.635x | [0.779796, 1.282386] | 0.96902563 / 0.24872262 | FASTER |
| `df_pivot_table` | 10M | 266.440 ms | 553.235 ms | 2.076x | [0.825871, 1.210842] | 0.73063521 / 0.19131626 | FASTER |

All four rows share one invocation and one FrankenPandas ELF. Each engine ran
25 alternating A/A pairs per row. CV was recorded as provenance only and had
no vote.

## Strict-remote 10M confirmation

Worker: `vmi1264463`

Invocation:
`vs-pandas-20260728T231721.145860Z-pid3114728`

FrankenPandas ELF:
`95bcac44a908ea7db67c069a319ef0b37886892892c9c55b581cde82c4b8d37a`
(70,294,080 bytes)

| workload | size | FP p50 | pandas p50 | ratio | combined 2x A/A interval | effect / required | verdict |
|---|---:|---:|---:|---:|---|---:|---|
| `df_pivot` | 10M | 1,041.735 ms | 1,343.116 ms | 1.289x | [0.837838, 1.193549] | 0.25410519 / 0.17693107 | FASTER |

The confirmation was built from the same checkout on the same worker, but its
self-reported ELF SHA-256 differs from the complete gate. It is independent
corroboration, not an A/A repeat of the first binary.

## Local same-invocation corroboration

FrankenPandas ELF:
`e3f48b7795e4cdde321e6cda4304cde6a859b7798a3a9e8bc8590b593bed42a5`
(70,268,904 bytes)

pandas 2.2.3 artifact:
`c10b13e6b6bec9a38bef8a24062c35f84c343a67973eec708b0c523302a5845f`

| workload | size | FP p50 | pandas p50 | ratio | combined 2x A/A interval | effect / required | verdict |
|---|---:|---:|---:|---:|---|---:|---|
| `df_pivot` | 1M | 30.448 ms | 102.553 ms | 3.368x | [0.801479, 1.247694] | 1.21436358 / 0.22129712 | FASTER |
| `df_pivot` | 10M | 356.068 ms | 1,422.974 ms | 3.996x | [0.815135, 1.226791] | 1.38538215 / 0.20440176 | FASTER |
| `df_pivot_table` | 1M | 15.234 ms | 42.727 ms | 2.805x | [0.884206, 1.130958] | 1.03132759 / 0.12306518 | FASTER |
| `df_pivot_table` | 10M | 148.331 ms | 537.886 ms | 3.626x | [0.859238, 1.163821] | 1.28819727 / 0.15170875 | FASTER |

The 1M rows share invocation
`vs-pandas-20260728T224938.391149Z-pid584782`; the 10M rows share invocation
`vs-pandas-20260728T224950.798204Z-pid590958`. Each engine ran 25 alternating
A/A pairs per row. CV was provenance only.

## Semantic boundary

- `df_pivot` uses unique `(r, c)` pairs with `r = i / 10`, `c = i % 10`,
  and one Float64 value column.
- `df_pivot_table` uses `r = i % 100`, `c = i % 10`, a Float64 value
  column, and the `mean` aggregation.
- Fixture construction is outside the timed region.
- Existing `fp-frame` pivot tests cover long-to-wide output, duplicate-key
  rejection, and typed-versus-generic parity. The live conformance suite covers
  pandas `pivot_table(..., aggfunc="mean")`.

These are pandas-vectorized reshape operations, not per-element Python callback
workloads. The local binary showed ratio growth, but the complete strict-remote
binary showed `2.414x -> 1.151x` for pivot and `2.635x -> 2.076x` for
pivot_table. Absolute time saved grew with N, while relative scaling was
binary/host-sensitive.

## Raw evidence

- `quiet_lane_m_pivot_1m_20260728.json`:
  `6b8d912abfe9429b6cac94f8bc71f498e9a118620d20c3ae68725f8de56025ab`
- `quiet_lane_m_pivot_10m_20260728.json`:
  `07251adb354b258a594ce05b4d57fe15410dca5cfe5dce5a17af9881f817c929`
- `proud_lane_m_pivot_1m_10m_20260728.json`:
  `634d3999907e969ae78330ad55dfc40692d6236d44c3b1cee84224164380788b`
- `proud_lane_m_pivot_10m_confirm_20260728.json`:
  `10cdfc68b1cd20b52a83020c6c64f9d8714759409de560da84d6bb54d0ed0f6b`

## Validation

- Strict-remote command:
  `RCH_WORKER=vmi1264463 RCH_REQUIRE_REMOTE=1 env -u CARGO_TARGET_DIR rch exec -- cargo test --locked --profile release-perf -p fp-frame dataframe_pivot_long_to_wide_wa8t3`
- Result: `tests::dataframe_pivot_long_to_wide_wa8t3` passed; 1 passed,
  0 failed.
- `python3 scripts/perf_candidate_preflight.py --self-test`: PASS (53 cases).
- `python3 scripts/perf_candidate_preflight.py --check-new-rows --base HEAD`:
  policy contract satisfied.

## Retry predicates

- Keep the incumbent-win classification until the workload boundary, fixture,
  FrankenPandas implementation, harness source, pandas artifact, allocator,
  compiler, or worker ISA changes.
- Re-open a monotonic ratio-growth claim only when the same self-identified ELF
  runs both sizes on the same worker in one invocation and two clean repeats
  show the ratio increase outside the combined A/A intervals.
- Re-run the 10M pivot row after any provenance change because one complete
  strict-remote gate was null-undecidable.
- Do not propose a source lever without a current profile naming a non-zero-self
  frame and a computed Amdahl ceiling.
