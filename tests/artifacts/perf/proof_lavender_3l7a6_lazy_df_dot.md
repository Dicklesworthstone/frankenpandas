# br-frankenpandas-3l7a6 lazy finite df_dot output columns

Agent: LavenderStone
Date: 2026-06-15

## Target

Profile-backed `df_dot 100000x1` after `br-frankenpandas-uza04.145` phase timing.
The profile showed `DataFrame::dot` internals at about 61-75 ms/call, dominated
by compute, while the harness still reported about 156 ms/iter because the full
205 MB result was materialized and then dropped even when the caller only read
shape metadata.

## Lever

One lever: in `DataFrame::dot`, detect all-valid finite Float64 operands and
return lazy Float64 dot-product output columns. The storage primitive is present
in `51e383f4` (`perf(fp-columnar): zero-copy Float64DotInput for f64 dot
products`). This bead wires that primitive into `fp-frame`.

Fallback is unchanged for non-Float64, nullable, NaN, or infinite operands. The
lazy materializer computes each cell with the same `l = 0..k` left fold as the
eager kernel, so ordering and floating-point operation order are preserved when
values are read.

## Isomorphism proof

Baseline binary:
`/data/projects/.scratch/cargo-target-lavender-3l7a6-base/release-perf/examples/perf_profile`

Candidate binary:
`/data/projects/.scratch/cargo-target-lavender-3l7a6-candidate-current/release-perf/examples/perf_profile`

Golden command:
`perf_profile golden df_dot 5000`

Baseline SHA256:
`04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

Candidate SHA256:
`04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

`cmp` result: matched.

Artifacts:
- `tests/artifacts/perf/lavender_3l7a6_base_golden_df_dot_5000.txt`
- `tests/artifacts/perf/lavender_3l7a6_candidate_current_golden_df_dot_5000.txt`
- `tests/artifacts/perf/lavender_3l7a6_golden_compare_current.txt`

## Benchmark

Baseline-only:
- `df_dot 100000x1`: `341.1 ms +/- 8.5`

Paired forward:
- Baseline: `353.1 ms +/- 12.7`
- Candidate: `255.5 ms +/- 11.1`
- Ratio: candidate `1.38x +/- 0.08` faster

Paired reversed:
- Candidate: `252.5 ms +/- 9.5`
- Baseline: `351.3 ms +/- 13.7`
- Ratio: candidate `1.39x +/- 0.08` faster

Artifacts:
- `tests/artifacts/perf/lavender_3l7a6_base_hyperfine_df_dot_100000x1.txt`
- `tests/artifacts/perf/lavender_3l7a6_pair_current_hyperfine_df_dot_100000x1.txt`
- `tests/artifacts/perf/lavender_3l7a6_pair_current_reversed_hyperfine_df_dot_100000x1.txt`

## Validation

- `rch exec -- cargo check -p fp-columnar -p fp-frame --lib`
- `rch exec -- cargo clippy -p fp-columnar -p fp-frame --lib -- -D warnings`
- `rch exec -- cargo test -p fp-columnar from_f64_all_valid_dot_product_materializes_left_fold_rows --lib`
- `rch exec -- cargo test -p fp-frame df_dot --lib`
- `cargo fmt --check --package fp-columnar --package fp-frame`

RCH was mixed: early baseline/candidate builds fell open locally; final
current-source candidate build and the `fp-frame df_dot` test ran remotely on
`vmi1227854`.

UBS note: `ubs crates/fp-frame/src/lib.rs` was attempted and recorded to
`tests/artifacts/perf/lavender_3l7a6_ubs_fp_frame.txt`, but the scan produced no
findings after the banner and remained stuck in an `ast-grep` subprocess for
over five minutes. The process tree was terminated with SIGTERM at
approximately 2026-06-15T22:51Z.

## Score

Impact 3.0 * Confidence 4.0 / Effort 2.0 = `6.0`; accepted.
