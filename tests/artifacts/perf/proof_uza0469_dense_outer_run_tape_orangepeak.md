# br-frankenpandas-uza04.69 keep proof: witness-only dense outer run tape

## Target

- Bead: `br-frankenpandas-uza04.69`
- Workload: `perf_profile outer_join 500000 {20,200}`
- Baseline head: `f91d3233`
- Lever kept: for exact `Int64DenseCycleWitness` inputs, build a dense outer run tape directly from contiguous key-domain blocks, bypassing generic witness CSR construction, tuple-plan triples, and post-plan descriptor scans. Uncertified inputs still use the previous CSR/plan fallback.

## Profile-backed hotspot

Fresh baseline profile (`perf_report_base_uza0469_outer_join_500000x200_nochildren.txt`) showed the post-`.68` residual:

- `build_single_key_dense_i64_outer_merge_output::{closure#5}`: `20.41%` self, witness CSR/position construction.
- `build_single_key_dense_i64_outer_merge_output`: `18.81%` self, with tuple triple write/push visible.
- Descriptor materialization from the tuple plan: `ToArcSlice` rows at `11.87%`, `11.31%`, `10.38%`, and `6.68%` self.
- `__memmove_avx_unaligned_erms`: `10.28%` self, including `Arc<[usize]>` conversion and allocation growth.

## Change

Added a `DenseOuterRunTape` helper in `fp-join` that:

1. Pre-counts exact run count, active key count, output length, and side-missing flags over the certified dense-cycle key domain.
2. Emits run lengths, left validity, left positions, right bucket-order positions, right segments, key runs, and sparse invalid ranges in one domain-ordered pass.
3. Falls back to the prior CSR/tuple-plan implementation for any missing witness or arithmetic inconsistency.

This is not the rejected `.67` direct descriptor-vector retry: the kept path removes the generic CSR construction and tuple plan for certified dense-cycle domains, while the old path stays intact for uncertified inputs.

## Behavior proof

- Ordering: unchanged. The helper iterates the same ascending bucket keys and emits left positions as `offset + k * period`, preserving left-major duplicate order.
- Tie-breaking: unchanged. Matched buckets still emit one run per left row, each pointing at the same right bucket-order segment as the prior CSR.
- Null placement: unchanged. Left-only buckets append right invalid ranges; right-only buckets append left invalid ranges with the same output offsets.
- DType promotion: unchanged. `has_left_missing` and `has_right_missing` still drive the same lane constructors.
- Floating point: unchanged. The only f64 path remains the existing `i64 as f64` nullable promotion constructor.
- RNG/hash behavior: unchanged; this path is deterministic and hash-free.
- Golden `outer_join 20000` SHA stayed `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- Byte compare: `golden_cmp=0`.

## Benchmark evidence

Direct harness timing:

- `outer_join 500000x20`: `11.719 ms/iter` -> `6.814 ms/iter`.
- `outer_join 500000x200`: `6.567 ms/iter` -> `3.434 ms/iter`.

Paired hyperfine:

- `500000x20`: baseline `199.1 ms +/- 9.3`, after `168.6 ms +/- 11.2`; after `1.18x +/- 0.10` faster.
- `500000x200`: baseline `918.0 ms +/- 50.0`, after `722.1 ms +/- 45.0`; after `1.27x +/- 0.11` faster.

Score: Impact 4 x Confidence 4 / Effort 3 = 5.33. Keep.

## Validation

- `cargo fmt -p fp-join -- --check`: passed.
- `rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib`: passed.
- `rch exec -- cargo check -p fp-join --all-targets`: passed.
- `rch exec -- cargo clippy -p fp-join --all-targets --no-deps -- -D warnings`: passed.
- Full `cargo clippy -p fp-join --all-targets -- -D warnings` was attempted and failed in upstream `fp-frame` at `crates/fp-frame/src/lib.rs:45570` (`needless_range_loop`) before linting this `fp-join` change; that lint is from current `origin/main`, not this bead.
- `ubs crates/fp-join/src/lib.rs .skill-loop-progress.md .beads/issues.jsonl tests/artifacts/perf/proof_uza0469_dense_outer_run_tape_orangepeak.md`: exited nonzero on pre-existing broad `fp-join` inventory. A new sentinel-comparison false positive was removed; remaining critical entries are existing dtype/key equality false positives at `fp-join` lines 960, 961, and 10485.

## After profile

The generic CSR/tuple-plan hot rows disappeared from the visible list. The next residual is now inside `build_dense_cycle_outer_run_tape` itself, mostly descriptor vector pushes and append loops. The next bead should attack that new run-tape write pattern directly, not the old generic CSR/tuple-plan path.
