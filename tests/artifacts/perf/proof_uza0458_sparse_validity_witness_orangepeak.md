# br-frankenpandas-uza04.58 proof - dense outer sparse validity witness

## Baseline and profile

- Head: `2786d412`.
- Build: `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0458-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --example perf_profile`.
- Golden baseline SHA256 for `perf_profile golden outer_join 20000`: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- Hyperfine baseline for `outer_join 500000 20`: `386.2 ms +/- 22.3 ms`.
- Fresh profile for `outer_join 500000 200`: `13.697 ms/iter`, `build_single_key_dense_i64_outer_merge_output` `88.29%` children, `nullable_repeated_slices_validity` `5.43%` self / `8.66%` children, and nullable repeat-values validity `4.16%` inlined.

## Lever kept

One lever: dense outer join now constructs coalesced sparse invalid-range witnesses during the bucket/plan walk and passes those witnesses into hidden nullable Float64 constructors:

- `Column::from_f64_nullable_repeated_slices_shared_with_sparse_validity`
- `Column::from_f64_nullable_repeat_values_run_lengths_with_sparse_validity`

This avoids rescanning `right_segments`, `left_run_valid`, and `run_lens` to rebuild the same validity bits after the join builder already knows which output runs are null.

## Isomorphism proof

- Output ordering: unchanged. The join still emits the same `plan`, `key_runs`, `run_lens`, `left_run_valid`, and `right_segments` in ascending dense-bucket order.
- Tie-breaking: unchanged. Left positions and right CSR positions are still consumed in their original per-bucket order.
- Floating point: unchanged. Promoted values still use the same `i64 as f64` casts in `left_run_values_f64` and `tape_f64`; this lever only supplies validity.
- Null/NaN semantics: unchanged. Missing left runs still map to `Null(NullKind::NaN)` in nullable repeat-values lanes; missing right runs still map to `Null(NullKind::NaN)` in nullable repeated-slices lanes. Coalesced invalid ranges materialize the same validity bits as the previous per-segment scans.
- RNG/hash behavior: unchanged. No RNG path or hash table iteration changed.
- Golden output: after SHA256 stayed `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`; `cmp=0`.

## Benchmarks

Paired `outer_join 500000 20`:

- Baseline: `344.5 ms +/- 14.6 ms`.
- After: `320.7 ms +/- 20.4 ms`.
- Ratio: after `1.07x +/- 0.08`.

Confirmation `outer_join 500000 200`:

- Baseline: `2.379 s +/- 0.046 s`.
- After: `2.113 s +/- 0.116 s`.
- Ratio: after `1.13x +/- 0.07`.

Score: Impact `3` x Confidence `4` / Effort `2` = `6.0`; keep.

## Validation

- `cargo fmt -p fp-columnar -p fp-join --check`: passed.
- `rch exec -- cargo test -p fp-columnar precomputed_sparse_validity --lib`: passed.
- `rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib`: passed.
- `rch exec -- cargo check -p fp-columnar -p fp-join --all-targets`: passed.
- `rch exec -- cargo clippy -p fp-columnar --all-targets --no-deps -- -D warnings`: passed.
- `rch exec -- cargo clippy -p fp-join --lib --no-deps -- -D warnings`: passed.
- Broad dependency Clippy surfaced unrelated current-head `fp-frame/src/lib.rs:5877` `needless_range_loop`; it is outside this bead and not changed here.
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs`: exited 0. Reported broad existing inventories/false positives, including dtype/sentinel comparisons misclassified as secret comparisons.

## After profile

After profile for `outer_join 500000 200`: `10.317 ms/iter`. `nullable_repeated_slices_validity`, nullable repeat-values validity, and `from_f64_nullable_*` validity construction are no longer visible in the hot list. Residual shifted back to CSR construction and single-lane promoted f64 value materialization.
