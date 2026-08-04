# br-frankenpandas-uza04.55 route-staging reroute proof

## Target

- Bead: `br-frankenpandas-uza04.55`
- Baseline commit: `d553106d`
- Scenario: `perf_profile outer_join 500000 20`
- Hypothesis: speculative dense outer all-matched route staging remained a top residual after `br-frankenpandas-uza04.54`.

## Baseline Evidence

- Build command: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0455-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`
- RCH status: crate-scoped command; all workers failed preflight and RCH failed open to local execution.
- Golden command: `/data/projects/.scratch/cargo-target-orangepeak-uza0455-base/release-perf/examples/perf_profile golden outer_join 20000`
- Golden sha256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
- Baseline hyperfine: `358.0 ms +/- 11.3 ms` for `outer_join 500000 20` over 10 runs.

## Profile Result

The short `500000x20` profile had only 85 samples, so the route was confirmed with a longer `500000x200` profile:

- Direct timer: `13.596 ms/iter`, `sink=48928115200`.
- Samples: 704 cycles samples.
- `build_single_key_dense_i64_outer_merge_output`: `91.65%` children.
- `build_single_key_dense_i64_outer_merge_output::{closure#0}` / CSR construction: `16.47%` children, `12.27%` self.
- `build_single_key_dense_i64_outer_merge_output::{closure#6}` / left promoted run values: `15.53%` self.
- `build_single_key_dense_i64_outer_merge_output::{closure#8}` / right promoted tape: `11.89%` self.
- `Column::from_f64_nullable_repeated_slices_shared`: `11.51%` children.
- `build_single_key_dense_i64_outer_all_matched_merge_output`: `5.32%` self.

## Decision

No source change was made for `br-frankenpandas-uza04.55`. The current profile invalidates the all-matched route-staging hypothesis: speculative staging is no longer the dominant residual, and optimizing it would be off-target.

Follow-up `br-frankenpandas-uza04.56` targets a different memory-layout primitive: cursor-free CSR construction in the general dense outer builder, preserving stable bucket order while removing the extra cursor allocation/copy.
