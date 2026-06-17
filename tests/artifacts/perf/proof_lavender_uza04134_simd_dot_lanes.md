# br-frankenpandas-uza04.134 proof: SIMD lane dot micro-kernel

Agent: LavenderStone
Date: 2026-06-15

## Target

Fresh current-main routing after the `.132` keep and `.133`/`vc4de` dot rejections
showed `df_dot 100000x5` as the largest residual in the local matrix:

- `df_dot`: 124.529 ms/iter
- `csv_parse_dates_dt_year`: 70.285 ms/iter
- `str_outer_join`: 43.331 ms/iter

Source: `tests/artifacts/perf/lavender_next_routing_matrix.txt`.

## Lever

One source lever was kept: the existing full 4x4 `DataFrame::dot` tile now uses
portable safe-Rust SIMD vectors across the four independent output-column lanes.

The scalar edge path is unchanged. The full-tile path still iterates `l = 0..k`
once and keeps one accumulator per output cell. It does not introduce unsafe
code, horizontal reductions, fused multiply-adds, or reordered reductions within
any output cell.

## Behavior isomorphism

- Row order: unchanged; the same `i0`, `ii`, and output row indices are used.
- Column order: unchanged; SIMD lanes map directly to the existing `dj = 0..4`
  output columns from the same packed B panel.
- Floating point: each output cell still accumulates terms in increasing
  `l = 0..k` order with `acc += a * b`; no horizontal sum or `mul_add` was
  added.
- Tie-breaking and ordering: not applicable to dense dot, and no index/label
  logic changed.
- RNG: not used by this path.
- Null/NaN semantics: unchanged; the lever only replaces the numeric full-tile
  multiply-add loop after matrix materialization.
- Safety: `#![forbid(unsafe_code)]` remains active; the implementation uses
  `std::simd::Simd` only.

Golden outputs are unchanged:

- `df_dot 2000`: `ddbde1c39c4856c19700fe90a29f6acce2a742a98a298585f896b0b02cdbb535`
- `df_dot 5000`: `04af7c2bb0e772d23ed69b3733da0778c3693ba1e67557c0126fcbd4458fdb3d`

Artifacts:

- Baseline: `tests/artifacts/perf/lavender_uza04134_base_baseline.txt`
- Candidate: `tests/artifacts/perf/lavender_uza04134_after_candidate.txt`

## Benchmark

Baseline binary: `/data/projects/.scratch/cargo-target-lavender-next-base/release-perf/examples/perf_profile`

Candidate binary: `/data/projects/.scratch/cargo-target-lavender-uza04134-after/release-perf/examples/perf_profile`

Internal harness:

- Baseline: 123.108 ms/iter
- Candidate: 113.680 ms/iter

Standalone hyperfine:

- Baseline: 669.4 ms +/- 18.0 ms
- Candidate: 611.5 ms +/- 20.3 ms

Paired hyperfine, 16 runs:

- Baseline: 662.9 ms +/- 11.2 ms
- Candidate: 603.7 ms +/- 11.0 ms
- Candidate ran 1.10x +/- 0.03 faster

Artifacts:

- `tests/artifacts/perf/lavender_uza04134_base_hyperfine_df_dot_100000x5.json`
- `tests/artifacts/perf/lavender_uza04134_after_hyperfine_df_dot_100000x5.json`
- `tests/artifacts/perf/lavender_uza04134_pair_df_dot_100000x5.txt`
- `tests/artifacts/perf/lavender_uza04134_pair_df_dot_100000x5.json`

## Validation

- `rch exec -- cargo check -p fp-frame --all-targets`: pass
- `rch exec -- cargo clippy -p fp-frame --all-targets -- -D warnings`: pass
- `rch exec -- cargo test -p fp-frame df_dot -- --nocapture`: pass
- `rustfmt --edition 2024 --check crates/fp-frame/src/lib.rs`: pass on the
  formatted worktree; only the SIMD source hunk is staged for the commit.
- `ubs crates/fp-frame/src/lib.rs`: started for the touched file; the Rust
  ast-grep phase was still running after the other crate-scoped gates finished.

## Score

Impact 2.0 x Confidence 5.0 / Effort 2.0 = 5.0.

Verdict: keep.
