# br-frankenpandas-uza04.72 proof - dense-cycle promoted LEFT lazy lane

Agent: OrangePeak
Date: 2026-06-10
Head before change: c1208305

## Profile-backed target

Baseline build:

`CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0472-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`

RCH failed open locally because no worker was admissible; the build remained crate-scoped.

Baseline profile for `outer_join 500000 200`:

- `fp_join::build_dense_cycle_outer_run_tape`: 40.85% self.
- Visible residuals included `Vec<usize>::push` and `repeat_n` descriptor writes.

## One lever

Added a witness-backed `LazyNullableDenseCycleLeftI64AsFloat64` column backing and wired certified dense-cycle OUTER joins to skip `run_lens`, `left_run_valid`, and `left_run_positions` when left lanes are promoted by right-only buckets.

Fallbacks are unchanged:

- Non-certified inputs use the existing CSR/plan descriptor path.
- Preserved left lanes still build the existing run descriptors.
- Right lanes continue to use the existing descriptor-free promoted RIGHT path from `.71`.

## Behavior isomorphism

- Ordering preserved: yes. Materialization walks buckets in ascending dense key order, identical to `build_dense_cycle_outer_run_tape`.
- Tie-breaking unchanged: yes. Matched buckets emit left-major duplicate order; each left position is repeated for `right_count` rows before the next left position.
- Null placement unchanged: yes. Right-only buckets emit `Null(NullKind::NaN)` for exactly `right_count` promoted-left rows via the same sparse validity mask.
- DType promotion unchanged: yes. The new lane is used only where the previous promoted-left path produced nullable `Float64`; preserved left lanes still use `Int64`.
- Floating-point unchanged: yes. Valid rows cast `source[left_pos] as f64`, the same cast used by the descriptor-backed promoted-left lane.
- RNG unchanged: yes. No RNG use was added or reordered.
- Fallback behavior unchanged: yes. Any missing dense-cycle proof or descriptor requirement returns to the pre-existing descriptor path.

Golden output:

- Baseline `outer_join 20000` SHA: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- After `outer_join 20000` SHA: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- `golden_cmp=0`.
- `sha256sum -c tests/artifacts/perf/golden_after_uza0472_outer_join_20000.sha256` passed.

Focused parity:

- `cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib` passed.

## Benchmarks

Baseline direct:

- `outer_join 500000 20`: 5.081 ms/iter.
- `outer_join 500000 200`: 1.391 ms/iter.

After direct:

- `outer_join 500000 20`: 3.927 ms/iter.
- `outer_join 500000 200`: 0.385 ms/iter.

Paired hyperfine:

- `outer_join 500000 20`: base `123.2 ms +/- 13.6`, after `92.7 ms +/- 9.2`; after `1.33x +/- 0.20` faster.
- `outer_join 500000 200`: base `311.3 ms +/- 26.6`, after `93.0 ms +/- 10.8`; after `3.35x +/- 0.48` faster.

Score:

- Impact 5 x Confidence 5 / Effort 3 = 8.33. Keep.

## Validation

Passed:

- `cargo fmt -p fp-columnar -p fp-join -- --check`
- `cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib`
- `cargo check -p fp-columnar -p fp-join --all-targets`
- `cargo clippy -p fp-columnar -p fp-join --all-targets --no-deps -- -D warnings`

Known unrelated blocker:

- `cargo clippy -p fp-join --all-targets -- -D warnings` fails in pre-existing dependency code at `crates/fp-frame/src/lib.rs:45570` (`clippy::needless_range_loop`).

After profile:

- `fp_join::build_dense_cycle_outer_run_tape` dropped to about 1.51% self.
- The next visible bottleneck is input-frame construction, led by `fp_columnar::Column::new` at about 17.83% self in the no-children report.
- A longer `outer_join 500000 2000` after-profile amortizes input-frame setup and routes the next join residual back to `build_dense_cycle_outer_run_tape` at about 13.97% self, now mostly witness arithmetic and `key_runs` pushes rather than the removed left descriptors.

## Artifacts

- `tests/artifacts/perf/golden_base_uza0472_outer_join_20000.txt`
- `tests/artifacts/perf/golden_after_uza0472_outer_join_20000.txt`
- `tests/artifacts/perf/hyperfine_pair_uza0472_outer_join_500000x20.json`
- `tests/artifacts/perf/hyperfine_pair_uza0472_outer_join_500000x200.json`
- `tests/artifacts/perf/perf_base_uza0472_outer_join_500000x200.no_children.txt`
- `tests/artifacts/perf/perf_after_uza0472_outer_join_500000x200.no_children.txt`
- `tests/artifacts/perf/perf_after_uza0472_outer_join_500000x2000.no_children.txt`
