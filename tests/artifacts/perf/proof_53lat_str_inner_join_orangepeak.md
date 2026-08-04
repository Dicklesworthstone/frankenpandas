# br-frankenpandas-53lat Proof: ordered contiguous-Utf8 inner join

## Target

- Scenario: `str_inner_join 1_000_000 10`
- Bead: `br-frankenpandas-53lat`
- Lever: ordered, strictly-unique contiguous-Utf8 byte-span merge before the existing byte-span hash fallback.

## Baseline

- Command: `rch exec -- hyperfine --warmup 2 --runs 10 --export-json tests/artifacts/perf/fp_before_str_inner_join_53lat_orangepeak.json '/data/projects/.cargo-target/frankenpandas-orangepeak-53lat/release-perf/examples/perf_profile str_inner_join 1000000 10'`
- Result: `2.360 s +/- 0.102`
- Per-iteration estimate: `236.0 ms`
- Profile: `tests/artifacts/perf/perf_report_before_str_inner_join_53lat_orangepeak.txt`
- Hotspot: ~74% of samples under `fp_join::merge_single_key_inner_unsorted`.

## After

- Build: `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-53lat RUSTFLAGS=-Cforce-frame-pointers=yes cargo build -p fp-conformance --profile release-perf --example perf_profile`
- Worker: `vmi1149989`
- Command: `rch exec -- hyperfine --warmup 2 --runs 10 --export-json tests/artifacts/perf/fp_after_str_inner_join_53lat_orangepeak.json '/data/projects/.cargo-target/frankenpandas-orangepeak-53lat/release-perf/examples/perf_profile str_inner_join 1000000 10'`
- Result: `256.0 ms +/- 11.1 ms`
- Per-iteration estimate: `25.6 ms`
- Speedup: `9.22x`
- After profile: `tests/artifacts/perf/perf_report_after_str_inner_join_53lat_orangepeak.txt`
- Shifted hotspot: byte comparisons in `strictly_increasing_utf8_key_spans` / ordered merge.

## Isomorphism

- Before golden: `tests/artifacts/perf/golden_before_str_inner_join_53lat_orangepeak.txt`
- After golden: `tests/artifacts/perf/golden_after_str_inner_join_53lat_orangepeak.txt`
- SHA256 before and after: `76d2f388645ed3f3578017c5e2d919fa809e4230793a86d54d5ca93a6d0bc10e`
- `cmp -s` before/after golden text: equal.
- Ordering proof: gate requires both key columns to be all-valid contiguous-Utf8 and strictly increasing, so every matched key is unique on both sides. The two-cursor merge emits each 1:1 match in ascending left position; duplicate or unsorted inputs reject and fall back to the existing left-major/right-insertion byte-span hash path. Floating-point/RNG behavior is not touched; the key comparison and position emission only choose row pairs.

## Verification

- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-53lat cargo test -p fp-join`: pass, 104 tests.
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-53lat cargo check -p fp-join --all-targets`: pass.
- `rch exec -- env CARGO_TARGET_DIR=/data/projects/.cargo-target/frankenpandas-orangepeak-53lat cargo clippy -p fp-join --all-targets -- -D warnings`: pass.
- `cargo fmt -p fp-join --check`: pass.
- `ubs crates/fp-join/src/lib.rs`: nonzero due broad pre-existing scanner findings/false positives outside this diff, including `DType::Int64` equality classified as secret comparison; clippy/fmt/check/tests are clean.

## Score

- Impact: `9.22`
- Confidence: `0.95` (golden-identical, focused ordering tests, full fp-join tests, check, clippy, fmt)
- Effort: `1.0`
- Score: `8.76`
