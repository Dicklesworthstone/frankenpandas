# br-frankenpandas-483i5 proof - contiguous UTF8 strict-witness cache

Agent: OrangePeak
Date: 2026-06-08

## Target

Profile-backed hotspot: `str_inner_join 1000000` in `fp-conformance` `perf_profile`.

Baseline profile:

- `merge_single_key_inner_unsorted`: 66.70%
- `__memcmp_avx2_movbe`: 30.74%
- `strictly_increasing_utf8_key_spans`: 19.51%
- `Column::take_positions`: 7.34%
- `build_str_join_frame`: 20.37%

Lever: cache the strict byte-order witness on immutable `LazyContiguousUtf8`
column backing and route sorted string joins through that cached witness instead
of rescanning both key columns on every join iteration.

## Benchmark

Harness build:

```text
rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-483i5 RUSTFLAGS=-Cforce-frame-pointers=yes cargo build -p fp-conformance --profile release-perf --example perf_profile
```

The final formatted-source build completed on RCH worker `vmi1156319`.

Hyperfine baseline:

```text
rch exec -- hyperfine --warmup 2 --runs 10 --export-json tests/artifacts/perf/fp_before_str_inner_join_483i5_orangepeak.json '/data/projects/.cargo-target/frankenpandas-orangepeak-483i5/release-perf/examples/perf_profile str_inner_join 1000000 10'
```

Result: 198.4 ms +/- 5.4 ms

Hyperfine after:

```text
rch exec -- hyperfine --warmup 2 --runs 10 --export-json tests/artifacts/perf/fp_after_str_inner_join_483i5_orangepeak.json '/data/projects/.cargo-target/frankenpandas-orangepeak-483i5/release-perf/examples/perf_profile str_inner_join 1000000 10'
```

Result: 139.5 ms +/- 6.7 ms

Mean delta:

- before mean: 0.19844258626000003 s
- after mean: 0.13946096824 s
- speedup: 1.4229x
- reduction: 29.72%
- score: Impact 1.4229 * Confidence 0.96 / Effort 0.5 = 2.73

## Golden output

```text
76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e  tests/artifacts/perf/golden_before_str_inner_join_483i5_orangepeak.txt
76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e  tests/artifacts/perf/golden_after_str_inner_join_483i5_orangepeak.txt
```

`cmp -s` passed.

## Isomorphism proof

- Ordering/tie-breaking: unchanged. The merge still uses the same two-cursor
  ordered UTF8 route and emits the same absolute left/right positions. The
  strict witness uses the exact prior comparator, `previous >= current`, over
  the same byte spans.
- Null/NaN semantics: unchanged. The cached witness is only exposed for
  `DType::Utf8`, all-valid contiguous UTF8 columns. Nullable and scalar-backed
  columns return `None` and keep the existing fallback route.
- Floating-point: not involved.
- RNG: not involved.
- Mutation: `LazyContiguousUtf8` bytes and offsets are immutable after column
  construction. The cache stores only whether the existing byte spans are
  strictly increasing; it does not cache output rows or alter data.
- Cloning: cloning a column recreates the lazy contiguous backing and its cache.
  That may recompute the witness, but it cannot change observed values.

## Validation

- `cargo fmt -p fp-columnar -p fp-join --check`: passed
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-483i5 cargo test -p fp-columnar -p fp-join`: passed
  - `fp-columnar`: 379 passed, 5 ignored
  - `fp-join`: 105 passed
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-483i5 cargo check -p fp-columnar -p fp-join --all-targets`: passed on `vmi1153651`
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-483i5 cargo clippy -p fp-columnar -p fp-join --all-targets -- -D warnings`: passed on `vmi1153651`
- `ubs crates/fp-columnar/src/lib.rs crates/fp-join/src/lib.rs`: nonzero due
  broad existing findings in the two large files. The reported critical items
  are false positives around dtype/key equality (`DType::Int64` and test key
  equality), and the Rust fmt/clippy/build/test sections were clean.

## Re-profile after

After profile:

- `core::fmt::Formatter::pad_integral`: 19.95%
- `Column::take_positions`: 13.61%
- `build_str_join_frame`: 6.82%
- `merge_single_key_inner_unsorted`: 6.29%
- `__memcmp_avx2_movbe`: 3.45%

The prior `strictly_increasing_utf8_key_spans` scan no longer appears in the
searched after-profile report. The next profile-backed target is output
construction / `take_positions` for repeated contiguous UTF8 inner joins.
