# br-frankenpandas-uza04.71 keep proof: descriptor-free dense-cycle right lane

## Target

- Bead: `br-frankenpandas-uza04.71`
- Workload: `perf_profile outer_join 500000 {20,200}`
- Baseline head: `f5c2e77a`
- Lever kept: promoted RIGHT-side Int64 lanes in certified dense-cycle OUTER joins now use a parametric lazy column that stores the left/right dense-cycle witnesses instead of materializing `right_positions_csr` and repeated-slice descriptors in the merge hot path. Fallback and all-valid right lanes still use the prior descriptor path.

## Profile-backed hotspot

Fresh baseline profile (`perf_report_base_uza0471_outer_join_500000x200_nochildren.txt`) showed:

- `build_dense_cycle_outer_run_tape`: `46.29%` self.
- Right-position CSR append/push frames remained visible under `append_dense_cycle_positions`, `push<usize>`, and `push_mut<usize>`.
- The target was not another repeated-descriptor tuning pass; it was the right-position tape/materialization boundary for promoted right lanes.

## Change

Added a hidden `ScalarValues::LazyNullableDenseCycleRightI64AsFloat64` variant and a matching `Column::from_i64_nullable_dense_cycle_right_as_f64_with_sparse_validity` constructor. The new lazy lane stores:

- the right source `Arc<[i64]>`
- left and right `Int64DenseCycleWitness`
- `min_key`, `span`, `total_len`
- the existing sparse validity witness

For the certified promoted-right dense outer path, `build_dense_cycle_outer_run_tape` skips the right-position CSR and right segment descriptors. If the right lane is all-valid, or if the dense witness route is unavailable, the old descriptor-backed constructors are still used.

## Behavior proof

- Ordering: unchanged. Lazy materialization iterates the same ascending bucket keys.
- Tie-breaking: unchanged. Matched buckets still emit right rows inside each left-major duplicate run by source position `offset + k * period`.
- Null placement: unchanged. Left-only buckets still produce `Null(NaN)` runs and use the same sparse validity mask.
- DType promotion: unchanged. The new constructor returns `DType::Float64`, matching the previous promoted `i64 as f64` lane.
- Floating point: unchanged. Valid slots still use the same `source[pos] as f64` cast; missing slots remain `Null(NullKind::NaN)`.
- RNG/hash behavior: unchanged. The path is deterministic and hash-free.
- Golden `outer_join 20000` SHA stayed `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- Byte compare: `golden_cmp=0`.

## Benchmark evidence

Direct harness timing:

- `outer_join 500000x20`: `6.323 ms/iter` -> `4.032 ms/iter`.
- `outer_join 500000x200`: `3.068 ms/iter` -> `1.185 ms/iter`.

Paired hyperfine:

- `500000x20`: baseline `125.2 ms +/- 9.5`, after `96.1 ms +/- 6.2`; after `1.30x +/- 0.13` faster.
- `500000x200`: baseline `503.4 ms +/- 27.3`, after `223.7 ms +/- 11.8`; after `2.25x +/- 0.17` faster.

Score: Impact 5 x Confidence 5 / Effort 4 = 6.25. Keep.

## Validation

- `cargo fmt -p fp-columnar -p fp-join -- --check`: passed.
- `rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib`: passed.
- `rch exec -- cargo check -p fp-columnar --all-targets`: passed.
- `rch exec -- cargo check -p fp-join --all-targets`: passed.
- `rch exec -- cargo clippy -p fp-columnar --all-targets --no-deps -- -D warnings`: passed.
- `rch exec -- cargo clippy -p fp-join --all-targets --no-deps -- -D warnings`: passed.
- Full `cargo clippy -p fp-join --all-targets -- -D warnings` was attempted and failed in upstream `fp-frame` at `crates/fp-frame/src/lib.rs:45570` (`needless_range_loop`) before linting this `fp-join` change.
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs .skill-loop-progress.md .beads/issues.jsonl tests/artifacts/perf/proof_uza0471_dense_cycle_right_lazy_orangepeak.md`: exited nonzero on pre-existing broad inventory (`3` critical, `8082` warning, `789` info); no finding targets the new dense-cycle lazy variant or join-routing hunk.

## After profile

Final after profile (`perf_report_after_uza0471_outer_join_500000x200_nochildren.txt`) still shows `build_dense_cycle_outer_run_tape` as the top residual at `50.23%` self, but the absolute runtime fell sharply and the right-position CSR tape is gone from the hot path. The next route should attack the remaining left-side run-position / run-length descriptor construction, not the removed right-position tape.
