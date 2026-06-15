# br-frankenpandas-vj4mq evidence-only closeout

## Target

`br-frankenpandas-vj4mq` proposed zero-copy Arc-backed column slice views for
the recorded `iloc_slice` vs-pandas gap. Before editing, I rebuilt the current
`perf_profile` example through `rch` and re-measured the target.

## Baseline

- Build: `rch exec -- env CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-lavender-vj4mq-base cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Successful worker: `vmi1227854`
- Direct target:
  - `perf_profile iloc_slice 100000 5000`
  - `0.023s total`, `0.005 ms/iter`, `sink=250000000`
- Hyperfine target:
  - `perf_profile iloc_slice 100000 5000`
  - `30.4 ms +/- 3.1 ms` for 5000 iterations

This is not the bead's stated `~127 us/iter` residual. The current code already
routes `DataFrame::iloc_slice` through `take_contiguous_row_range_unchecked`,
and `Column::take_contiguous_range` already returns zero-copy Float64 and Utf8
slice views. The session loop also records the accepted `br-frankenpandas-2ol0s`
keep: bench-runner `indexing/iloc_slice` 100000 p50 moved `0.905 ms -> 0.002 ms`.

## Correctness / Isomorphism

No source code was changed for this bead. The current accepted slice primitive
already has the `br-frankenpandas-2ol0s` golden proof for `iloc_slice` at 5000
and 100000 rows, with identical row order, index labels, column order, dtype,
and Float64 payload bits.

Because this closeout does not alter source, there is no new ordering,
tie-breaking, floating-point, RNG, or null/NaN behavior to validate.

## Decision

Reject/close `br-frankenpandas-vj4mq` as a stale duplicate target, not as a
ceiling. The live profile-backed target moved: a fresh current-main matrix from
the same RCH-built binary shows `df_dot 100000x5` at `122.242 ms/iter`, ahead of
`str_outer_join 41.362 ms/iter`, `df_kendall 39.504 ms/iter`, and
`str_left_join 35.001 ms/iter`.

Next optimization should attack `df_dot` with a different compute primitive,
not another `iloc_slice` copy-elision pass.
