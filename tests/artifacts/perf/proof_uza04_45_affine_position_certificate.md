# br-frankenpandas-uza04.45 affine position certificate proof

## Change

`Column::take_strided_all_valid_float64_positions` now acquires the Float64
backing before validating regular positions, then uses
`bounded_arithmetic_progression_positions(positions, source_len)` to prove a
bounded affine certificate `(start, step)` for the selection vector.

The accepted path is still the existing `LazyStridedFloat64` projection. The
fallback path is unchanged: non-affine, duplicate, descending, overflowing, or
out-of-bounds positions return `None` and fall through to the existing gather.

## Baseline and profile

Before binary:
`/data/projects/.scratch/cargo-target-orangepeak-uza04-45-pass1-worker/release-perf/examples/perf_profile`

After binary:
`/data/projects/.scratch/cargo-target-orangepeak-uza04-45-after/release-perf/examples/perf_profile`

Fresh before baseline:

- `filter_bool 100000 1000`: 487.5 ms +/- 57.6 ms
- Profile: `take_positions` / `take_strided_all_valid_float64_positions` 56.17% children
- Profile: `arithmetic_progression_positions` 49.95% children
- Profile: `checked_sub` 24.87%

## Isomorphism proof

- Ordering preserved: yes. The certificate validates every selected position in the original slice order.
- Tie-breaking unchanged: N/A. Boolean filtering has no comparison tie policy.
- Floating-point unchanged: yes. The accepted path reads the same `f64` backing values through the same `LazyStridedFloat64` materializer, preserving `-0.0`, infinities, and NaN payload bits.
- Null and validity unchanged: yes. This path still applies only to all-valid Float64 columns and emits `ValidityMask::all_valid(len)`.
- RNG unchanged: N/A.
- Fallback behavior unchanged: yes. Invalid certificates return `None`; existing gather paths still handle or panic on invalid positions as before.

Golden SHA:

```text
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/golden_before_filter_bool_1000_uza04_45.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/golden_before_filter_bool_100000_uza04_45.txt
f7eb0f99728d924edb7398ec4cbcc07808651a0f091db01e1a3c12e011d6265c  tests/artifacts/perf/golden_after_filter_bool_1000_uza04_45.txt
2f77640d30dc32e1db9ab52eb869dc5ad503434f5a8a647a9d92f5f1ffa69cea  tests/artifacts/perf/golden_after_filter_bool_100000_uza04_45.txt
```

`sha256sum -c tests/artifacts/perf/golden_pair_uza04_45.sha256` passed.

## Benchmarks

Paired run, 10 samples:

- Before: 446.6 ms +/- 10.9 ms
- After: 425.1 ms +/- 11.4 ms
- Ratio: 1.05x +/- 0.04

Confirmation run, 20 samples:

- Before: 462.6 ms +/- 29.6 ms
- After: 434.5 ms +/- 10.7 ms
- Ratio: 1.06x +/- 0.07

After profile:

- `checked_sub` no longer appears as a material hotspot.
- `bounded_arithmetic_progression_positions` is now the residual at 46.25% children.

Score: Impact 2 x Confidence 3 / Effort 2 = 3.0. Keep.

## Validation

- `cargo fmt -p fp-columnar --check`
- `rch exec -- cargo test -p fp-columnar bounded_arithmetic_progression_positions_rejects_non_affine_or_oob --lib`
- `rch exec -- cargo test -p fp-columnar float64_take_positions_regular_stride_defers_contiguous_gather --lib`
- `rch exec -- cargo check -p fp-columnar --all-targets`
- `rch exec -- cargo clippy -p fp-columnar --all-targets -- -D warnings`
- `ubs crates/fp-columnar/src/lib.rs`

RCH failed open locally for these crate-scoped commands because workers failed
preflight. UBS reported zero critical findings; warnings were inherited
large-file inventory.

## Shifted bottleneck

The next route should not repeat this local helper rewrite. The remaining
profile-backed target is the repeated per-column affine certificate scan itself:
`bounded_arithmetic_progression_positions` under `take_positions`. A deeper next
primitive needs to remove repeated per-column row-selection verification while
avoiding the rejected `.42` DataFrame descriptor shape.
