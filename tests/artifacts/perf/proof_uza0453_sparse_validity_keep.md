# br-frankenpandas-uza04.53 keep proof: sparse nullable validity ranges

## Target

- Bead: `br-frankenpandas-uza04.53`
- Scenario: `perf_profile outer_join 500000 20`
- Baseline commit: `9e5e5be2`
- Lever: store nullable repeated-slice and repeat-value validity as sorted sparse invalid ranges instead of eagerly allocating and clearing a packed bitmap for the full outer-join output.

## Profile-backed baseline

- Baseline build: `tests/artifacts/perf/rch_build_uza0453_base.txt`
- Direct baseline: `tests/artifacts/perf/perf_profile_uza0453_outer_join_500000x20_base.txt`
  - `56.741 ms/iter`, sink `4892811520`
- Baseline hyperfine: `tests/artifacts/perf/hyperfine_base_uza0453_outer_join_500000x20.txt`
  - `1.136 s +/- 0.029 s`
- Baseline profile:
  - `fp_join::build_single_key_dense_i64_outer_merge_output`: `79.29%` children
  - `Column::from_f64_nullable_repeated_slices_shared`: `24.12%` children
  - `__memset_avx2_unaligned_erms`: `22.74%` under nullable validity construction

## Final candidate

- Candidate build: `tests/artifacts/perf/rch_build_uza0453_sparse_validity_after.txt`
- Direct candidate: `tests/artifacts/perf/perf_profile_uza0453_outer_join_500000x20_sparse_validity_after.txt`
  - `17.729 ms/iter`, sink `4892811520`
- Paired hyperfine: `tests/artifacts/perf/hyperfine_pair_uza0453_sparse_validity_outer_join_500000x20.txt`
  - baseline: `1.138 s +/- 0.015 s`
  - candidate: `341.9 ms +/- 10.7 ms`
  - candidate is `3.33x +/- 0.11` faster
- Fresh candidate profile:
  - `build_single_key_dense_i64_outer_merge_output`: `69.49%` children
  - `Column::from_f64_nullable_repeated_slices_shared`: `8.28%` children
  - `Column::nullable_repeated_slices_validity`: `5.66%` children
  - `__memset_avx2_unaligned_erms`: `3.04%` in the outer builder

## Isomorphism proof

- Output ordering and tie behavior are unchanged. The merge code and segment/run descriptor order are untouched; only the validity mask representation behind nullable lazy columns changes.
- Floating-point and RNG behavior are unchanged. No arithmetic, comparison, NaN payload, random sampling, or seed path changes.
- Null semantics are unchanged. `ValidityMask::get`, `bits`, `count_valid`, `count_invalid`, `PartialEq`, and serde all observe the same bit stream as the old eager bitmap.
- Mutation and cold mask algebra preserve the old representation contract by materializing sparse invalid ranges before `set`, `and_mask`, `or_mask`, `xor_mask`, `not_mask`, and serialization.
- Sparse invalid ranges are sorted and non-overlapping by construction: both builders walk monotone segment/run descriptors and append the current output position only for null runs.
- Golden output:
  - baseline SHA256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
  - candidate SHA256: `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`
  - `diff -u` between baseline and candidate golden dumps is empty.

## Validation

- `rustfmt --check crates/fp-columnar/src/lib.rs`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0453-check rch exec -- cargo test -p fp-columnar sparse_invalid_ranges -- --nocapture`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0453-check rch exec -- cargo check -p fp-columnar -p fp-join --all-targets`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0453-check rch exec -- cargo clippy -p fp-columnar -p fp-join --all-targets -- -D warnings`
- `CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0453-check rch exec -- cargo test -p fp-columnar`
- `ubs crates/fp-columnar/src/lib.rs`: no critical findings; clippy/fmt/build clean, broad existing warning inventory remains.

## Score and next profile

- Score: `5 impact * 4 confidence / 2 effort = 10.0`, keep.
- Next profile target after this keep: the shifted outer-join hot path is descriptor construction inside `build_single_key_dense_i64_outer_merge_output` (`closure#6`, `closure#8`, `ToArcSlice` allocation/copy) plus fixture frame construction.
