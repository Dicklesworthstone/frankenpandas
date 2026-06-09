# br-frankenpandas-uza04.61 source-position promoted lanes

Agent: OrangePeak
Date: 2026-06-09
Target: `perf_profile outer_join 500000`

## Profile-backed target

Fresh current-main baseline for `outer_join 500000x200` showed
`build_single_key_dense_i64_outer_merge_output` at 89.16% of sampled cycles.
The promoted Int64-as-Float64 output lanes were visible as construction-time
value-materialization closures:

- `build_single_key_dense_i64_outer_merge_output::{closure#7}`: 19.30%.
- `build_single_key_dense_i64_outer_merge_output::{closure#9}`: 16.03%.
- CSR construction closure: 13.93%.

Baseline evidence:

- Golden SHA `outer_join 20000`:
  `453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`.
- Direct `outer_join 500000x20`: 0.316s, 15.814 ms/iter.
- Direct `outer_join 500000x200`: 2.183s, 10.917 ms/iter.
- Hyperfine baseline `500000x20`: 336.9 ms +/- 15.2 ms.
- Hyperfine baseline `500000x200`: 2.234 s +/- 0.088 s.

## Lever

Added source-position-backed nullable Float64 lanes for promoted Int64 join
outputs:

- Right repeated-slice lanes now carry `source: Arc<[i64]>`, the shared CSR
  position tape, segment descriptors, and sparse validity. They cast
  `source[positions[k]] as f64` only if a consumer materializes scalar values.
- Left repeat-value lanes now carry `source: Arc<[i64]>`, shared run source
  positions, run validity, run lengths, and sparse validity. They cast one source
  row per valid run only on scalar materialization.

This replaces the hot construction-time `Vec<f64>` tape/run-value gathers while
preserving the same lazy materialized Scalars and validity masks.

## Isomorphism proof

- Output row ordering and left-major tie order are unchanged because the same
  dense outer plan, run lengths, CSR positions, and right segments drive the
  output.
- Null placement is unchanged because the existing sparse invalid-range masks
  are reused unchanged.
- Dtype promotion is unchanged: output columns remain `DType::Float64`.
- Float64 bits for present values are unchanged: both paths use Rust `i64 as f64`
  for the same source row.
- Missing promoted slots still materialize as `Scalar::Null(NullKind::NaN)`.
- RNG/hash behavior is unchanged; no hash or randomized path changed.

Golden SHA after the lever:

`453b318a10d828fcf65cef61898a965ea4e1a402e041cb45b0a7a4001b65b750`

The after SHA matches the baseline SHA exactly.

## Bench gate

Direct timings:

- `outer_join 500000x20`: 15.814 ms/iter -> 13.886 ms/iter.
- `outer_join 500000x200`: 10.917 ms/iter -> 9.282 ms/iter.

Paired hyperfine:

- `500000x20`: baseline 313.0 ms +/- 14.4 ms, after 305.3 ms +/- 14.7 ms,
  after 1.03x +/- 0.07.
- `500000x200`: baseline 2.643 s +/- 0.423 s, after 1.990 s +/- 0.031 s,
  after 1.33x +/- 0.21.
- Reversed `500000x200`: after 1.882 s +/- 0.037 s, baseline 2.092 s +/- 0.078 s,
  after 1.11x +/- 0.05.

Score: Impact 3 x Confidence 4 / Effort 2 = 6.0. Keep.

## Validation

- `cargo fmt -p fp-columnar -p fp-join -- --check`
- `rch exec -- cargo test -p fp-columnar i64_as_f64_matches --lib`
- `rch exec -- cargo test -p fp-join merge_outer_dense_int64_duplicates_matches_generic_validated_route --lib`
- `rch exec -- cargo check -p fp-columnar -p fp-join --all-targets`
- `rch exec -- cargo clippy -p fp-columnar --all-targets --no-deps -- -D warnings`
- `rch exec -- cargo clippy -p fp-join --lib --no-deps -- -D warnings`

The broad dependency clippy command still hits an unrelated existing
`fp-frame/src/lib.rs:5902` `needless_range_loop` lint. UBS over the touched Rust
files reported the existing file-wide inventory and false-positive secret
comparison matches; no sampled finding targets the new source-position
descriptor logic.

## Shifted residual

After profile:

- `build_single_key_dense_i64_outer_merge_output`: 87.84%.
- `__memmove_avx_unaligned_erms`: 19.56%.
- `build_single_key_dense_i64_outer_merge_output::{closure#2}`: 16.73%.
- Descriptor Arc materialization maps: 6.94%, 4.69%, 3.96%.

The old promoted `Vec<f64>` tape/run-value closures are no longer visible.
Next route: remove source-copy/memmove in the descriptor path by carrying
Arc-backed Int64 source provenance from column construction or by making the
dense outer builder share existing Int64 buffers without per-merge Arc copies.
