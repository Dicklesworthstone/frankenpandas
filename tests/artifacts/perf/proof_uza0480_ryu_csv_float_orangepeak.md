# br-frankenpandas-uza04.80 proof - CSV Float64 ryu formatter

## Target

- Bead: `br-frankenpandas-uza04.80`
- Owner: `OrangePeak`
- Lever: guarded finite `ryu::Buffer::format_finite` fast path in `fp_io::write_pandas_float`
- Scope: `fp-io` CSV Float64 rendering only
- Baseline head: `98ed64a93d225436dee4ffb15132c711e7570638`

## Baseline

Command:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0480-base RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --bin high_ram_perf_baseline
/data/projects/.scratch/cargo-target-orangepeak-uza0480-base/release-perf/high_ram_perf_baseline --profile uza0480-base --rows 100000 --iters 20 --warmup 3 --frame-cols 10 --key-cardinality 100000
```

`csv_write_read_roundtrip` baseline:

- mean: `90.53049875 ms`
- p50: `88.703384 ms`
- p95: `95.33589 ms`
- p99: `99.875493 ms`
- throughput: `1104600.1223979779 rows/s`
- `io_payload_bytes`: `8611370`
- `rows_out`: `100000`
- `checksum`: `62503875000.0`
- stable witness SHA: `d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367`

Baseline CPU profile:

- `fp_io::write_pandas_float`: `37.82%` children / `3.19%` self
- `core::fmt::write`: `34.87%` children
- `core::fmt::float::float_to_decimal_common_shortest::<f64>`: `32.95%` children
- `core::num::imp::flt2dec::strategy::grisu::format_shortest_opt`: `26.61%` children / `14.27%` self

## After

Command:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0480-after RUSTFLAGS='-C force-frame-pointers=yes' rch exec -- cargo build -p fp-conformance --profile release-perf --bin high_ram_perf_baseline
/data/projects/.scratch/cargo-target-orangepeak-uza0480-after/release-perf/high_ram_perf_baseline --profile uza0480-after --rows 100000 --iters 20 --warmup 3 --frame-cols 10 --key-cardinality 100000
```

`csv_write_read_roundtrip` after:

- mean: `82.42092585 ms`
- p50: `81.500158 ms`
- p95: `89.672275 ms`
- p99: `91.176505 ms`
- throughput: `1213284.114060944 rows/s`
- `io_payload_bytes`: `8611370`
- `rows_out`: `100000`
- `checksum`: `62503875000.0`
- stable witness SHA: `d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367`

Delta:

- `csv_write_read_roundtrip` mean: `1.098392x` faster, `8.9578%` lower
- p50: `8.1206%` lower
- p95: `5.9407%` lower
- p99: `8.7098%` lower
- throughput: `9.8392%` higher

Paired full-harness hyperfine:

- baseline first: baseline `3.86123184868 s +/- 0.0989489538449445`; after `3.66391216758 s +/- 0.0995057572474756`; after `1.053855x` faster
- after first: after `3.6012718923399993 s +/- 0.152901898550669`; baseline `3.8906604091399997 s +/- 0.11553571880311639`; after `1.080357x` faster

After CPU profile:

- samples: `4077`, lost samples: `0`
- `core::fmt::write`, `float_to_decimal_common_shortest`, and `grisu::format_shortest_opt` no longer appear on the hot CSV Float64 path
- new residual: `ryu::pretty::format64` at `30.23%` children / `22.65%` self
- `fp_io::write_pandas_float`: `7.89%` children / `5.15%` self
- next shifted residual: Ryu finite formatting itself and `DataFrameGroupBy::aggregate_named_func` allocator work

## Behavior Isomorphism

- Row ordering: unchanged; only scalar Float64 spelling inside existing row/column loops changed.
- Column ordering: unchanged; writer still follows `frame.column_names()` and typed-column traversal.
- Header/index/delimiter/quoting: unchanged; the lever is below those branches.
- Null and NaN behavior: unchanged; `write_value_csv_escaped_with_buf` still checks `value.is_nan()` before calling `write_pandas_float`, so NaN is emitted via `na_rep`.
- Infinities: unchanged; non-finite values deopt to the previous Debug-based fallback and retain `inf` / `-inf`.
- Whole-number Float64 spelling: preserved by appending `.0` when Ryu emits a finite spelling without decimal or exponent.
- Scientific notation spelling: preserved by normalizing exponent sign and at least two exponent digits, matching the previous pandas-compatible formatter.
- Python repr boundary: preserved by falling back to the previous Debug formatter when finite nonzero values below `1e-4` would be emitted by Ryu as fixed decimal instead of Python-style scientific notation.
- Negative zero: preserved; Ryu emits `-0.0` and the fast path keeps it.
- Floating-point values: no arithmetic changes; only decimal text formatting changes.
- RNG/tie-breaking: not present in this writer path.

Golden proof:

```text
SHA_MATCH d93c23c308b1cbba82051d7dd022001e84e5313c0b485a440891a604ebfce367
```

## Validation

Passed:

```text
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0480-check rch exec -- cargo test -p fp-io to_csv_float_format_matches_pandas_str --lib -- --nocapture
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0480-check rch exec -- cargo test -p fp-io to_csv_single_column_nan_quotes_empty_and_keeps_float_repr --lib -- --nocapture
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0480-check rch exec -- cargo check -p fp-io --all-targets
CARGO_TARGET_DIR=/data/projects/.scratch/cargo-target-orangepeak-uza0480-clippy rch exec -- cargo clippy -p fp-io --all-targets -- -D warnings
cargo fmt -p fp-io -- --check
git diff --check -- crates/fp-io/Cargo.toml crates/fp-io/src/lib.rs .skill-loop-progress.md tests/artifacts/perf/uza0480_pass2_primitive_selection.md
```

Note: the first two `rch exec` test commands failed open locally because no admissible worker was available at that instant. The compile-heavy `check`, `clippy`, and release-perf builds ran remotely on `vmi1227854`.

UBS was run on the changed code and metadata subset. It still exits nonzero on broad pre-existing `fp-io` findings (`22` critical, `5714` warnings), while reporting no unsafe blocks, clean formatting, no clippy warnings, clean cargo check, and tests building clean. The new formatter zero check was changed to a bit-pattern test so it is not part of the remaining floating-point equality inventory.

## Score

- Impact: `2` (roughly `9%` faster for the targeted CSV workload and `5%..8%` faster for the full high-ram harness)
- Confidence: `4` (golden SHA match, focused tests, check/clippy/fmt, paired/reversed timing, after-profile)
- Effort: `3`
- Score: `2.67`

Verdict: keep. The generic Rust formatter stack was removed from the target path with byte-stable output.
