# br-frankenpandas-vc4de: full-width dot accumulator rejection

Date: 2026-06-14
Agent: LavenderStone
Target: `df_dot 100000x5`

## Profile-backed selection

`br-frankenpandas-vc4de` tracks the current post-parse_dates residual:
`df_dot 100000x5` was measured above `csv_parse_dates_dt_year` and
`str_outer_join` in the routing matrix. Prior rejected `df_dot` families include
BI/BJ widening, statement unroll, worker caps, pre-GEMM finite scans,
output-NaN witness work, row-panel packed-B reuse, and Vec-backed output chunks.

This pass tried a structurally different safe-Rust primitive inside
`DataFrame::dot`: a full-width four-row accumulator for moderate output widths
(`n <= 1024`). For each row tile and inner dimension `l`, it loaded the A lane
once and updated every output column accumulator. For each output cell, the
floating-point accumulation order remained `l = 0..k`; only sibling columns were
interleaved.

## Baseline

RCH build:

```text
RCH_WORKER=vmi1227854 RCH_VERBOSE=1 CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-vc4de-clean-base rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Artifact: `tests/artifacts/perf/lavender_vc4de_fullwidth_base_rch_build.txt`

Baseline binary:

```text
/data/projects/.scratch/cargo-target-lavender-vc4de-clean-base/release-perf/examples/perf_profile
```

Baseline evidence:

```text
df_dot 2000 golden sha256  = ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535
df_dot 5000 golden sha256  = 04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d
internal df_dot 100000x5  = 123.363 ms/iter
hyperfine df_dot 100000x5 = 666.7 ms +/- 15.4 ms
```

Artifacts:

- `tests/artifacts/perf/lavender_vc4de_fullwidth_base_baseline.txt`
- `tests/artifacts/perf/lavender_vc4de_fullwidth_base_hyperfine_df_dot_100000x5.json`

## Candidate

Validation before benchmark:

```text
rustfmt --edition 2024 crates/fp-frame/src/lib.rs
RCH_VERBOSE=1 CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-vc4de-fullwidth-check rch exec -- cargo check -p fp-frame --all-targets
```

`cargo check -p fp-frame --all-targets` passed on `vmi1227854`.
Artifact: `tests/artifacts/perf/lavender_vc4de_fullwidth_check_fp_frame.txt`

Candidate build:

```text
RCH_WORKER=vmi1227854 RCH_VERBOSE=1 CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-vc4de-fullwidth-after rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile
```

Artifact: `tests/artifacts/perf/lavender_vc4de_fullwidth_after_rch_build.txt`

Candidate evidence:

```text
df_dot 2000 golden sha256  = ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535
df_dot 5000 golden sha256  = 04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d
internal df_dot 100000x5  = 158.851 ms/iter
hyperfine df_dot 100000x5 = 821.3 ms +/- 16.2 ms
```

Artifacts:

- `tests/artifacts/perf/lavender_vc4de_fullwidth_after_candidate.txt`
- `tests/artifacts/perf/lavender_vc4de_fullwidth_after_hyperfine_df_dot_100000x5.json`

## Paired gate

Same host, paired binaries:

```text
baseline  = 691.3 ms +/- 23.0 ms
candidate = 837.3 ms +/- 14.1 ms
```

Hyperfine summary: baseline ran `1.21x +/- 0.05x` faster than the candidate.

Artifacts:

- `tests/artifacts/perf/lavender_vc4de_fullwidth_pair_df_dot_100000x5.txt`
- `tests/artifacts/perf/lavender_vc4de_fullwidth_pair_df_dot_100000x5.json`

## Isomorphism proof

- Ordering and column labels: unchanged; the candidate only changed the inner
  accumulator schedule inside `DataFrame::dot`.
- Floating point: for each output cell, additions still occurred in `l = 0..k`
  order, preserving per-cell tie/floating-point semantics.
- NaN/null: unchanged; before/after `df_dot` goldens for both sizes matched
  exactly.
- RNG: not applicable.
- Golden output: `df_dot` 2000 and 5000 SHA256 values are byte-identical before
  and after.

## Decision

Rejected. Score: `Impact 0 x Confidence 5 / Effort 2 = 0`.

The full-width accumulator increased working-set pressure enough to outweigh A
lane reuse. Do not retry this full-width per-row-tile accumulator family. The
source hunk was removed; only rejection evidence is retained.
